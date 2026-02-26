//! Core types for jj-ryu

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A jj bookmark (branch reference)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bookmark {
    /// Bookmark name
    pub name: String,
    /// Git commit ID (hex)
    pub commit_id: String,
    /// jj change ID (hex)
    pub change_id: String,
    /// Whether this bookmark exists on any remote
    pub has_remote: bool,
    /// Whether local and remote are in sync
    pub is_synced: bool,
}

/// A commit/change entry from jj log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Git commit ID (hex)
    pub commit_id: String,
    /// jj change ID (hex)
    pub change_id: String,
    /// Author name
    pub author_name: String,
    /// Author email
    pub author_email: String,
    /// First line of commit description
    pub description_first_line: String,
    /// Full commit description (includes first line)
    pub description: String,
    /// Parent commit IDs
    pub parents: Vec<String>,
    /// Local bookmarks pointing to this commit
    pub local_bookmarks: Vec<String>,
    /// Remote bookmarks pointing to this commit (format: "name@remote")
    pub remote_bookmarks: Vec<String>,
    /// Whether this is the working copy commit
    pub is_working_copy: bool,
    /// When the commit was authored
    pub authored_at: DateTime<Utc>,
    /// When the commit was committed
    pub committed_at: DateTime<Utc>,
}

/// A segment of changes belonging to one or more bookmarks
#[derive(Debug, Clone)]
pub struct BookmarkSegment {
    /// Bookmarks pointing to the tip of this segment
    pub bookmarks: Vec<Bookmark>,
    /// Changes in this segment (newest first)
    pub changes: Vec<LogEntry>,
}

/// A segment narrowed to a single bookmark (after user selection)
#[derive(Debug, Clone)]
pub struct NarrowedBookmarkSegment {
    /// The selected bookmark for this segment
    pub bookmark: Bookmark,
    /// Changes in this segment (newest first)
    pub changes: Vec<LogEntry>,
}

/// A stack of bookmarks from trunk to a leaf
#[derive(Debug, Clone)]
pub struct BranchStack {
    /// Segments from trunk (index 0) to leaf (last index)
    pub segments: Vec<BookmarkSegment>,
}

/// The complete change graph for a repository
///
/// Represents the single linear stack from trunk to working copy.
/// Only bookmarks between trunk and working copy are included.
#[derive(Debug, Clone, Default)]
pub struct ChangeGraph {
    /// All bookmarks in the stack by name
    pub bookmarks: HashMap<String, Bookmark>,
    /// The single stack from trunk to working copy (None if working copy is at trunk)
    pub stack: Option<BranchStack>,
    /// Number of bookmarks excluded due to merge commits
    pub excluded_bookmark_count: usize,
}

/// A pull request / merge request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    /// PR/MR number
    pub number: u64,
    /// Web URL for the PR/MR
    pub html_url: String,
    /// Base branch name
    pub base_ref: String,
    /// Head branch name
    pub head_ref: String,
    /// PR/MR title
    pub title: String,
    /// GraphQL node ID (GitHub only, used for mutations)
    pub node_id: Option<String>,
    /// Whether PR is a draft
    pub is_draft: bool,
}

/// A comment on a pull request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrComment {
    /// Comment ID
    pub id: u64,
    /// Comment body text
    pub body: String,
}

/// A git remote
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRemote {
    /// Remote name (e.g., "origin")
    pub name: String,
    /// Remote URL
    pub url: String,
}

/// Detected platform type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    /// GitHub or GitHub Enterprise
    GitHub,
    /// GitLab or self-hosted GitLab
    GitLab,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHub => write!(f, "GitHub"),
            Self::GitLab => write!(f, "GitLab"),
        }
    }
}

/// Platform configuration
#[derive(Debug, Clone)]
pub struct PlatformConfig {
    /// Platform type
    pub platform: Platform,
    /// Repository owner (user or organization)
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Custom host (None for github.com/gitlab.com)
    pub host: Option<String>,
}

// =============================================================================
// Merge-related types (for ryu merge command)
// =============================================================================

/// PR state (open, closed, merged)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    /// PR is open and can be merged
    Open,
    /// PR was closed without merging
    Closed,
    /// PR was merged
    Merged,
}

impl std::fmt::Display for PrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Closed => write!(f, "closed"),
            Self::Merged => write!(f, "merged"),
        }
    }
}

/// Extended PR details for merge operations
///
/// This contains more information than `PullRequest`, including the body
/// and merge status, which are needed for merge operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestDetails {
    /// PR/MR number
    pub number: u64,
    /// PR/MR title
    pub title: String,
    /// PR/MR body/description
    pub body: Option<String>,
    /// Current state of the PR
    pub state: PrState,
    /// Whether PR is a draft
    pub is_draft: bool,
    /// Whether PR can be merged (no conflicts)
    pub mergeable: Option<bool>,
    /// Head branch name
    pub head_ref: String,
    /// Base branch name
    pub base_ref: String,
    /// Web URL for the PR/MR
    pub html_url: String,
}

/// Merge readiness check result
///
/// Captures all the conditions that must be met for a PR to be merged.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct MergeReadiness {
    /// Whether the PR has been approved by reviewers
    pub is_approved: bool,
    /// Whether CI checks have passed
    pub ci_passed: bool,
    /// Whether the PR can be merged (no conflicts)
    /// - `Some(true)` = mergeable
    /// - `Some(false)` = has conflicts
    /// - `None` = unknown (GitHub still computing)
    pub is_mergeable: Option<bool>,
    /// Whether the PR is a draft
    pub is_draft: bool,
    /// Human-readable reasons why the PR cannot be merged (definitive blockers)
    pub blocking_reasons: Vec<String>,
    /// Reasons why merge status is uncertain (unknown states, not definitive blockers)
    pub uncertainties: Vec<String>,
}

impl MergeReadiness {
    /// Check if there are definitive blockers preventing merge.
    ///
    /// Returns `true` if the PR definitely cannot be merged:
    /// - Not approved
    /// - CI failing
    /// - Is a draft
    /// - Has confirmed merge conflicts (`is_mergeable == Some(false)`)
    ///
    /// Returns `false` if the PR might be mergeable (including unknown status).
    pub const fn is_blocked(&self) -> bool {
        !self.is_approved
            || !self.ci_passed
            || self.is_draft
            || matches!(self.is_mergeable, Some(false))
    }

    /// Returns the first uncertainty reason, if any.
    ///
    /// Use this to check if the merge attempt has unknown factors
    /// (e.g., GitHub hasn't computed mergeable status yet).
    pub fn uncertainty(&self) -> Option<&str> {
        self.uncertainties.first().map(String::as_str)
    }
}

/// Result of a merge operation
#[derive(Debug, Clone)]
pub struct MergeResult {
    /// Whether the merge was successful
    pub merged: bool,
    /// The SHA of the merge commit (if successful)
    pub sha: Option<String>,
    /// Message from the merge operation (especially on failure)
    pub message: Option<String>,
}

/// Merge strategy/method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    /// Squash all commits into one
    Squash,
    /// Create a merge commit
    Merge,
    /// Rebase commits onto base branch
    Rebase,
}

impl std::fmt::Display for MergeMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Squash => write!(f, "squash"),
            Self::Merge => write!(f, "merge"),
            Self::Rebase => write!(f, "rebase"),
        }
    }
}
