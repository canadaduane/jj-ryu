//! Change graph builder
//!
//! Builds a `ChangeGraph` from jj workspace state using jj-lib APIs.

use crate::error::Result;
use crate::repo::JjWorkspace;
use crate::types::{Bookmark, BookmarkSegment, BranchStack, ChangeGraph, LogEntry};
use std::collections::{HashMap, HashSet};
use tracing::debug;

/// Result from traversing a bookmark toward trunk
struct TraversalResult {
    /// Segments discovered (ordered from bookmark back to trunk)
    segments: Vec<RawSegment>,
    /// If we hit a fully-collected bookmark, its change ID
    already_seen_change_id: Option<String>,
    /// Number of bookmarks excluded due to merge commits
    excluded_bookmark_count: usize,
    /// Change IDs that should be marked as tainted (due to merge commits)
    newly_tainted_change_ids: Vec<String>,
}

/// A raw segment before full bookmark resolution
struct RawSegment {
    bookmark_names: Vec<String>,
    changes: Vec<LogEntry>,
}

/// Build a change graph from the current workspace state
///
/// This analyzes all bookmarks owned by the current user and builds
/// a graph showing how they stack on top of each other.
#[allow(clippy::too_many_lines)]
pub fn build_change_graph(workspace: &JjWorkspace) -> Result<ChangeGraph> {
    debug!("Discovering user bookmarks...");

    // Get all local bookmarks
    let all_bookmarks = workspace.local_bookmarks()?;

    debug!(
        "Found {} bookmarks: {:?}",
        all_bookmarks.len(),
        all_bookmarks.iter().map(|b| &b.name).collect::<Vec<_>>()
    );

    // Build bookmarks by name map
    let bookmarks_by_name: HashMap<String, Bookmark> = all_bookmarks
        .iter()
        .map(|b| (b.name.clone(), b.clone()))
        .collect();

    // Data structures for the algorithm
    let mut fully_collected_bookmarks: HashSet<String> = HashSet::new();
    let mut bookmark_to_change_id: HashMap<String, String> = HashMap::new();
    let mut bookmarked_change_adjacency_list: HashMap<String, String> = HashMap::new();
    let mut bookmarked_change_id_to_segment: HashMap<String, Vec<LogEntry>> = HashMap::new();
    let mut stack_roots: HashSet<String> = HashSet::new();
    let mut tainted_change_ids: HashSet<String> = HashSet::new();
    let mut total_excluded_bookmark_count = 0;

    // Process each bookmark to collect segment changes
    for bookmark in &all_bookmarks {
        if fully_collected_bookmarks.contains(&bookmark.name) {
            debug!("Skipping already processed bookmark: {}", bookmark.name);
            continue;
        }

        debug!("Processing bookmark: {}", bookmark.name);

        let result = traverse_and_discover_segments(
            workspace,
            bookmark,
            &fully_collected_bookmarks,
            &tainted_change_ids,
        )?;

        // Handle excluded bookmarks (those that encountered merges)
        if result.excluded_bookmark_count > 0 {
            // Add newly tainted change IDs for future traversals
            tainted_change_ids.extend(result.newly_tainted_change_ids);
            total_excluded_bookmark_count += result.excluded_bookmark_count;
            debug!(
                "  Excluded {} due to merge commit in history",
                bookmark.name
            );
            continue;
        }

        // Store segment changes for all bookmarks found in the result
        for segment in &result.segments {
            if segment.changes.is_empty() {
                continue;
            }
            let first_change_id = segment.changes[0].change_id.clone();
            bookmarked_change_id_to_segment
                .insert(first_change_id.clone(), segment.changes.clone());

            for bm_name in &segment.bookmark_names {
                bookmark_to_change_id.insert(bm_name.clone(), first_change_id.clone());
                fully_collected_bookmarks.insert(bm_name.clone());
            }

            debug!(
                "    Found segment for [{}]: {} changes",
                segment.bookmark_names.join(", "),
                segment.changes.len()
            );
        }

        // Establish stacking relationships based on the segment order
        // Segments are returned in order from target back to base
        for i in 0..result.segments.len().saturating_sub(1) {
            let child_segment = &result.segments[i];
            let parent_segment = &result.segments[i + 1];

            if child_segment.changes.is_empty() || parent_segment.changes.is_empty() {
                continue;
            }

            let child_id = child_segment.changes[0].change_id.clone();
            let parent_id = parent_segment.changes[0].change_id.clone();

            bookmarked_change_adjacency_list.insert(child_id.clone(), parent_id.clone());
            debug!(
                "    Stacking: [{}] -> [{}]",
                child_segment.bookmark_names.join(", "),
                parent_segment.bookmark_names.join(", ")
            );
        }

        // If we hit a fully-collected bookmark, establish relationship to it
        if let Some(ref already_seen_id) = result.already_seen_change_id {
            if let Some(root_segment) = result.segments.last() {
                if !root_segment.changes.is_empty() {
                    let root_id = root_segment.changes[0].change_id.clone();
                    bookmarked_change_adjacency_list.insert(root_id, already_seen_id.clone());
                }
            }
        } else if let Some(root_segment) = result.segments.last() {
            // We reached trunk, so the last segment is a root
            if !root_segment.changes.is_empty() {
                let root_id = root_segment.changes[0].change_id.clone();
                stack_roots.insert(root_id);
                for bm_name in &root_segment.bookmark_names {
                    debug!("    Root bookmark identified: {}", bm_name);
                }
            }
        }

        debug!(
            "  Processed {} - found {} segments",
            bookmark.name,
            result.segments.len()
        );
    }

    // Compute stack leafs (change IDs with no children)
    let change_ids_with_children: HashSet<String> =
        bookmarked_change_adjacency_list.values().cloned().collect();
    let stack_leafs: HashSet<String> = bookmarked_change_id_to_segment
        .keys()
        .filter(|id| !change_ids_with_children.contains(*id))
        .cloned()
        .collect();

    // Group segments into stacks
    let stacks = group_segments_into_stacks(
        &bookmarks_by_name,
        &stack_leafs,
        &bookmarked_change_adjacency_list,
        &bookmarked_change_id_to_segment,
    );

    Ok(ChangeGraph {
        bookmarks: bookmarks_by_name,
        bookmark_to_change_id,
        bookmarked_change_adjacency_list,
        bookmarked_change_id_to_segment,
        stack_leafs,
        stack_roots,
        stacks,
        excluded_bookmark_count: total_excluded_bookmark_count,
    })
}

/// Traverse from a bookmark toward trunk, discovering segments and relationships
fn traverse_and_discover_segments(
    workspace: &JjWorkspace,
    bookmark: &Bookmark,
    fully_collected_bookmarks: &HashSet<String>,
    tainted_change_ids: &HashSet<String>,
) -> Result<TraversalResult> {
    let mut segments: Vec<RawSegment> = Vec::new();
    let mut current_segment: Option<RawSegment> = None;
    let mut already_seen_change_id: Option<String> = None;
    let mut seen_change_ids: Vec<String> = Vec::new();

    // Query trunk..bookmark to get all commits in between
    let revset = format!("trunk()..{}", bookmark.commit_id);
    let changes = workspace.resolve_revset(&revset)?;

    // Check for merge commits or already-tainted changes
    for change in &changes {
        seen_change_ids.push(change.change_id.clone());

        // Check if this change is a merge commit or already tainted
        if change.parents.len() > 1 || tainted_change_ids.contains(&change.change_id) {
            debug!(
                "Found {} in bookmark {} - excluding bookmark and descendants",
                if change.parents.len() > 1 {
                    "merge commit"
                } else {
                    "tainted change"
                },
                bookmark.name
            );

            // Return the seen change IDs as newly tainted
            return Ok(TraversalResult {
                segments: Vec::new(),
                already_seen_change_id: None,
                excluded_bookmark_count: 1,
                newly_tainted_change_ids: seen_change_ids,
            });
        }
    }

    // Process changes to build segments
    for change in &changes {
        if !change.local_bookmarks.is_empty() {
            // Found a bookmark boundary - save current segment and start a new one
            if let Some(seg) = current_segment.take() {
                segments.push(seg);
            }

            // Check if any of these bookmarks are fully collected
            if change
                .local_bookmarks
                .iter()
                .any(|b| fully_collected_bookmarks.contains(b))
            {
                debug!("    Found fully-collected bookmark at {}", change.commit_id);
                already_seen_change_id = Some(change.change_id.clone());
                break;
            }

            current_segment = Some(RawSegment {
                bookmark_names: change.local_bookmarks.clone(),
                changes: Vec::new(),
            });

            debug!(
                "    Starting new segment for bookmarks: {} at commit {}",
                change.local_bookmarks.join(", "),
                change.commit_id
            );
        }

        if let Some(ref mut seg) = current_segment {
            seg.changes.push(change.clone());
        }
    }

    // Don't forget the last segment
    if let Some(seg) = current_segment {
        segments.push(seg);
    }

    Ok(TraversalResult {
        segments,
        already_seen_change_id,
        excluded_bookmark_count: 0,
        newly_tainted_change_ids: Vec::new(),
    })
}

/// Group segments into stacks based on their relationships
fn group_segments_into_stacks(
    bookmarks: &HashMap<String, Bookmark>,
    stack_leafs: &HashSet<String>,
    adjacency_list: &HashMap<String, String>,
    change_id_to_segment: &HashMap<String, Vec<LogEntry>>,
) -> Vec<BranchStack> {
    let mut stacks = Vec::new();

    for leaf_change_id in stack_leafs {
        let stack_change_ids = build_path_to_root(leaf_change_id, adjacency_list);
        let segments = build_segments(&stack_change_ids, bookmarks, change_id_to_segment);

        stacks.push(BranchStack { segments });
    }

    stacks
}

/// Build a path from a leaf bookmark back to the root
fn build_path_to_root(
    leaf_change_id: &str,
    adjacency_list: &HashMap<String, String>,
) -> Vec<String> {
    let mut path = vec![leaf_change_id.to_string()];
    let mut current = leaf_change_id.to_string();

    while let Some(parent) = adjacency_list.get(&current) {
        path.push(parent.clone());
        current = parent.clone();
    }

    path.reverse();
    path
}

/// Build `BookmarkSegments` from a list of change IDs
fn build_segments(
    stack_change_ids: &[String],
    bookmarks: &HashMap<String, Bookmark>,
    change_id_to_segment: &HashMap<String, Vec<LogEntry>>,
) -> Vec<BookmarkSegment> {
    let mut segments = Vec::new();

    for change_id in stack_change_ids {
        if let Some(changes) = change_id_to_segment.get(change_id) {
            if changes.is_empty() {
                continue;
            }

            let bookmark_list: Vec<Bookmark> = changes[0]
                .local_bookmarks
                .iter()
                .filter_map(|name| bookmarks.get(name).cloned())
                .collect();

            segments.push(BookmarkSegment {
                bookmarks: bookmark_list,
                changes: changes.clone(),
            });
        }
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_path_to_root_single() {
        let adjacency: HashMap<String, String> = HashMap::new();
        let path = build_path_to_root("leaf", &adjacency);
        assert_eq!(path, vec!["leaf"]);
    }

    #[test]
    fn test_build_path_to_root_chain() {
        let mut adjacency: HashMap<String, String> = HashMap::new();
        adjacency.insert("c".to_string(), "b".to_string());
        adjacency.insert("b".to_string(), "a".to_string());

        let path = build_path_to_root("c", &adjacency);
        assert_eq!(path, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_build_segments_empty() {
        let bookmarks: HashMap<String, Bookmark> = HashMap::new();
        let change_id_to_segment: HashMap<String, Vec<LogEntry>> = HashMap::new();

        let segments = build_segments(&["id1".to_string()], &bookmarks, &change_id_to_segment);
        assert!(segments.is_empty());
    }
}
