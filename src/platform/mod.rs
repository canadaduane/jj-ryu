//! Platform services for GitHub and GitLab
//!
//! Provides a unified interface for PR/MR operations across platforms.

mod detection;
mod factory;
mod github;
mod gitlab;

pub use detection::{detect_platform, parse_repo_info};
pub use factory::create_platform_service;
pub use github::GitHubService;
pub use gitlab::GitLabService;

use crate::error::Result;
use crate::types::{
    MergeMethod, MergeReadiness, MergeResult, PlatformConfig, PrComment, PullRequest,
    PullRequestDetails,
};
use async_trait::async_trait;

/// Platform service trait for PR/MR operations
///
/// This trait abstracts GitHub and GitLab operations, allowing the same
/// submission logic to work with either platform.
#[async_trait]
pub trait PlatformService: Send + Sync {
    /// Find an existing open PR for a head branch
    async fn find_existing_pr(&self, head_branch: &str) -> Result<Option<PullRequest>>;

    /// Create a new PR with default options (non-draft, no body).
    ///
    /// This is a convenience method that delegates to [`create_pr_with_options`]
    /// with `body: None` and `draft: false`. Implementors should override
    /// `create_pr_with_options`, not this method.
    ///
    /// [`create_pr_with_options`]: Self::create_pr_with_options
    async fn create_pr(&self, head: &str, base: &str, title: &str) -> Result<PullRequest> {
        self.create_pr_with_options(head, base, title, None, false)
            .await
    }

    /// Create a new PR with explicit body and draft options.
    ///
    /// Implementors must provide this method. The default [`create_pr`] method
    /// delegates here with `body: None` and `draft: false`.
    ///
    /// [`create_pr`]: Self::create_pr
    async fn create_pr_with_options(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: Option<&str>,
        draft: bool,
    ) -> Result<PullRequest>;

    /// Update the base branch of an existing PR
    async fn update_pr_base(&self, pr_number: u64, new_base: &str) -> Result<PullRequest>;

    /// Publish a draft PR (convert to ready for review)
    async fn publish_pr(&self, pr_number: u64) -> Result<PullRequest>;

    /// List comments on a PR
    async fn list_pr_comments(&self, pr_number: u64) -> Result<Vec<PrComment>>;

    /// Create a comment on a PR
    async fn create_pr_comment(&self, pr_number: u64, body: &str) -> Result<()>;

    /// Update an existing comment on a PR
    async fn update_pr_comment(&self, pr_number: u64, comment_id: u64, body: &str) -> Result<()>;

    /// Get the platform configuration
    fn config(&self) -> &PlatformConfig;

    // =========================================================================
    // Merge-related methods (for ryu merge command)
    // =========================================================================

    /// Get full PR details including body and state
    ///
    /// Returns extended PR information needed for merge operations,
    /// including the PR body (for commit message) and merge status.
    async fn get_pr_details(&self, pr_number: u64) -> Result<PullRequestDetails>;

    /// Check if PR is ready to merge
    ///
    /// Checks approval status, CI status, and merge conflicts.
    /// Returns a `MergeReadiness` struct with all conditions and blocking reasons.
    async fn check_merge_readiness(&self, pr_number: u64) -> Result<MergeReadiness>;

    /// Merge a PR with the specified method
    ///
    /// For squash merges, the PR title is used as commit title and
    /// the PR body is used as commit message.
    async fn merge_pr(&self, pr_number: u64, method: MergeMethod) -> Result<MergeResult>;
}
