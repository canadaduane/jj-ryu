//! Merge planning - pure functions for creating merge plans
//!
//! This module contains the pure, testable logic for creating merge plans.
//! No I/O happens here - all data is passed in, making it easy to unit test.

use crate::submit::SubmissionAnalysis;
use crate::types::{MergeMethod, MergeReadiness, PullRequestDetails};
use std::collections::HashMap;
use std::hash::BuildHasher;

/// Gathered PR information for planning
///
/// This struct holds all the information needed to plan a merge,
/// fetched beforehand by the CLI orchestrator.
#[derive(Debug, Clone)]
pub struct PrInfo {
    /// Bookmark name this PR is associated with
    pub bookmark: String,
    /// Full PR details including title, body, state
    pub details: PullRequestDetails,
    /// Merge readiness check results
    pub readiness: MergeReadiness,
}

/// Confidence level for a merge attempt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeConfidence {
    /// All conditions verified - merge should succeed
    Certain,
    /// Some conditions unknown - merge may fail
    Uncertain(String),
}

/// A single step in the merge plan
#[derive(Debug, Clone)]
pub enum MergeStep {
    /// Merge this PR
    Merge {
        /// Bookmark name
        bookmark: String,
        /// PR number
        pr_number: u64,
        /// PR title (for display)
        pr_title: String,
        /// Merge method to use
        method: MergeMethod,
        /// Confidence level for this merge
        confidence: MergeConfidence,
    },
    /// Retarget this PR's base branch to trunk before merging
    ///
    /// After merging PR N, PR N+1's base branch (which was PR N's branch)
    /// must be retargeted to trunk so it merges into trunk, not the defunct branch.
    RetargetBase {
        /// Bookmark name (for display)
        bookmark: String,
        /// PR number to retarget
        pr_number: u64,
        /// Current base branch (for display: "feat-a" → "main")
        old_base: String,
        /// New base branch (trunk)
        new_base: String,
    },
    /// Skip this PR (not ready to merge)
    Skip {
        /// Bookmark name
        bookmark: String,
        /// PR number
        pr_number: u64,
        /// Reasons why this PR cannot be merged
        reasons: Vec<String>,
    },
}

impl MergeStep {
    /// Get the bookmark name for this step
    pub fn bookmark_name(&self) -> &str {
        match self {
            Self::Merge { bookmark, .. }
            | Self::RetargetBase { bookmark, .. }
            | Self::Skip { bookmark, .. } => bookmark,
        }
    }
}

impl std::fmt::Display for MergeStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Merge {
                pr_number,
                pr_title,
                confidence,
                ..
            } => {
                let prefix = match confidence {
                    MergeConfidence::Certain => "merge",
                    MergeConfidence::Uncertain(_) => "merge (uncertain)",
                };
                write!(f, "{prefix} PR #{pr_number}: {pr_title}")
            }
            Self::RetargetBase {
                pr_number,
                old_base,
                new_base,
                ..
            } => {
                write!(f, "retarget PR #{pr_number}: {old_base} → {new_base}")
            }
            Self::Skip {
                pr_number,
                bookmark,
                reasons,
            } => {
                write!(f, "skip PR #{pr_number} ({bookmark})")?;
                if !reasons.is_empty() {
                    write!(f, ": {}", reasons.join(", "))?;
                }
                Ok(())
            }
        }
    }
}

/// Options for merge planning
#[derive(Debug, Clone, Default)]
pub struct MergePlanOptions {
    /// Target bookmark (merge up to and including this bookmark)
    /// If None, merge all consecutive mergeable PRs
    pub target_bookmark: Option<String>,
}

/// Merge plan - the functional core output
///
/// This is a pure data structure that describes what merge operations
/// should be performed. Created by `create_merge_plan()` (pure)
/// and executed by `execute_merge()` (effectful).
#[derive(Debug, Clone)]
pub struct MergePlan {
    /// Ordered steps to perform (or skip)
    pub steps: Vec<MergeStep>,
    /// Bookmarks to remove from PR cache after successful merges
    pub bookmarks_to_clear: Vec<String>,
    /// First unmerged bookmark (for rebasing remaining stack)
    pub rebase_target: Option<String>,
    /// Whether there are any actionable PRs (including uncertain merges)
    pub has_actionable: bool,
    /// Trunk branch name (e.g., "main") - needed for retarget steps
    pub trunk_branch: String,
}

impl MergePlan {
    /// Check if the plan has any merge steps
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.steps.iter().any(|s| matches!(s, MergeStep::Merge { .. }))
    }

    /// Count mergeable PRs
    #[must_use]
    pub fn merge_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s, MergeStep::Merge { .. }))
            .count()
    }
}

/// Create a merge plan (PURE - no I/O, easily testable)
///
/// This function takes the submission analysis and pre-fetched PR info,
/// and produces a plan describing what merges should be performed.
///
/// After each merge (except the last), a `RetargetBase` step is inserted
/// to retarget the next PR's base to trunk. This is necessary because
/// GitHub's merge API merges into the PR's current base branch, not trunk.
///
/// # Arguments
/// * `analysis` - The submission analysis from `analyze_submission()`
/// * `pr_info` - Map of bookmark name to PR info, pre-fetched by caller
/// * `options` - Planning options (target bookmark, etc.)
/// * `trunk_branch` - The trunk branch name (e.g., "main")
///
/// # Returns
/// A `MergePlan` describing the merge operations to perform
#[must_use]
pub fn create_merge_plan<S: BuildHasher>(
    analysis: &SubmissionAnalysis,
    pr_info: &HashMap<String, PrInfo, S>,
    options: &MergePlanOptions,
    trunk_branch: &str,
) -> MergePlan {
    let mut steps = Vec::new();
    let mut bookmarks_to_clear = Vec::new();
    let mut rebase_target = None;
    let mut hit_blocker = false;
    let mut hit_target = false;

    // Collect mergeable bookmarks first (we need lookahead for retarget steps)
    let mut mergeable_indices: Vec<usize> = Vec::new();

    // Process in stack order (trunk → leaf)
    for (idx, segment) in analysis.segments.iter().enumerate() {
        let bookmark_name = &segment.bookmark.name;

        // Check if we've passed the target bookmark
        if let Some(ref target) = options.target_bookmark {
            if hit_target {
                // Past target - this becomes rebase target
                if rebase_target.is_none() {
                    rebase_target = Some(bookmark_name.clone());
                }
                continue;
            }
            if bookmark_name == target {
                hit_target = true;
            }
        }

        let Some(info) = pr_info.get(bookmark_name) else {
            // No PR for this bookmark - skip it
            continue;
        };

        if hit_blocker {
            // After hitting a blocker, remaining PRs become the rebase target
            if rebase_target.is_none() {
                rebase_target = Some(bookmark_name.clone());
            }
            continue;
        }

        if info.readiness.is_blocked() {
            steps.push(MergeStep::Skip {
                bookmark: bookmark_name.clone(),
                pr_number: info.details.number,
                reasons: info.readiness.blocking_reasons.clone(),
            });
            hit_blocker = true;
            if rebase_target.is_none() {
                rebase_target = Some(bookmark_name.clone());
            }
        } else {
            // Track this as mergeable for retarget step insertion
            mergeable_indices.push(idx);

            // Determine confidence based on uncertainty
            let confidence = info
                .readiness
                .uncertainty()
                .map_or(MergeConfidence::Certain, |reason| {
                    MergeConfidence::Uncertain(reason.to_string())
                });
            steps.push(MergeStep::Merge {
                bookmark: bookmark_name.clone(),
                pr_number: info.details.number,
                pr_title: info.details.title.clone(),
                method: MergeMethod::Squash,
                confidence,
            });
            bookmarks_to_clear.push(bookmark_name.clone());
        }
    }

    // Now insert RetargetBase steps between consecutive Merge steps
    // We need to do this after collecting all steps because we need lookahead
    let mut final_steps = Vec::new();
    let mut merge_step_count = 0;

    for step in steps {
        match &step {
            MergeStep::Merge { .. } => {
                final_steps.push(step);
                merge_step_count += 1;

                // Check if there's a next mergeable PR that needs retargeting
                if merge_step_count < mergeable_indices.len() {
                    let next_idx = mergeable_indices[merge_step_count];
                    let next_segment = &analysis.segments[next_idx];
                    let next_bookmark = &next_segment.bookmark.name;

                    if let Some(next_info) = pr_info.get(next_bookmark) {
                        let old_base = &next_info.details.base_ref;
                        // Only add retarget if the base isn't already trunk
                        if old_base != trunk_branch {
                            final_steps.push(MergeStep::RetargetBase {
                                bookmark: next_bookmark.clone(),
                                pr_number: next_info.details.number,
                                old_base: old_base.clone(),
                                new_base: trunk_branch.to_string(),
                            });
                        }
                    }
                }
            }
            MergeStep::Skip { .. } | MergeStep::RetargetBase { .. } => {
                final_steps.push(step);
            }
        }
    }

    let has_actionable = final_steps
        .iter()
        .any(|s| matches!(s, MergeStep::Merge { .. }));

    MergePlan {
        steps: final_steps,
        bookmarks_to_clear,
        rebase_target,
        has_actionable,
        trunk_branch: trunk_branch.to_string(),
    }
}
