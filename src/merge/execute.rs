//! Merge execution - effectful operations
//!
//! This module contains the effectful code that actually performs merges.
//! It takes a `MergePlan` (created by the pure planning functions) and
//! executes the merge operations via the platform API.

use crate::error::Result;
use crate::merge::plan::{MergePlan, MergeStep};
use crate::platform::PlatformService;
use crate::submit::ProgressCallback;

/// Result of merge execution
#[derive(Debug, Clone, Default)]
pub struct MergeExecutionResult {
    /// Bookmarks that were successfully merged
    pub merged_bookmarks: Vec<String>,
    /// Bookmark where merge failed (if any)
    pub failed_bookmark: Option<String>,
    /// Error message from failed merge (if any)
    pub error_message: Option<String>,
}

impl MergeExecutionResult {
    /// Check if all planned merges succeeded
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.failed_bookmark.is_none()
    }

    /// Check if at least some merges succeeded
    #[must_use]
    pub const fn has_merges(&self) -> bool {
        !self.merged_bookmarks.is_empty()
    }

    /// Check if the bottom-most PR was merged (trunk changed)
    ///
    /// This is important for determining whether to rebase the remaining stack.
    #[must_use]
    pub const fn bottom_merged(&self) -> bool {
        // If we have any merges and no failure, or the first merge succeeded
        // before failure, the bottom was merged
        !self.merged_bookmarks.is_empty()
    }
}

/// Execute the merge plan (EFFECTFUL)
///
/// This function performs the actual merge operations via the platform API.
/// It stops at the first failure or skip, tracking what succeeded.
///
/// # Arguments
/// * `plan` - The merge plan to execute
/// * `platform` - Platform service for API calls
/// * `progress` - Progress callback for status updates
///
/// # Returns
/// A `MergeExecutionResult` with the outcome of the execution
pub async fn execute_merge(
    plan: &MergePlan,
    platform: &dyn PlatformService,
    progress: &dyn ProgressCallback,
) -> Result<MergeExecutionResult> {
    let mut result = MergeExecutionResult::default();

    for step in &plan.steps {
        match step {
            MergeStep::Merge {
                bookmark,
                pr_number,
                pr_title,
                method,
            } => {
                progress
                    .on_message(&format!("🔀 Merging PR #{pr_number}: {pr_title}"))
                    .await;

                match platform.merge_pr(*pr_number, *method).await {
                    Ok(merge_result) if merge_result.merged => {
                        let sha_display = merge_result.sha.as_deref().unwrap_or("(no sha)");
                        progress
                            .on_message(&format!("✅ Merged: {sha_display}"))
                            .await;
                        result.merged_bookmarks.push(bookmark.clone());
                    }
                    Ok(merge_result) => {
                        // Merge API returned but didn't merge
                        result.failed_bookmark = Some(bookmark.clone());
                        result.error_message = merge_result.message;
                        break;
                    }
                    Err(e) => {
                        result.failed_bookmark = Some(bookmark.clone());
                        result.error_message = Some(e.to_string());
                        break;
                    }
                }
            }
            MergeStep::Skip {
                bookmark,
                pr_number,
                reasons,
            } => {
                progress
                    .on_message(&format!(
                        "⏭️  Skipping PR #{pr_number} ({bookmark}): {}",
                        reasons.join(", ")
                    ))
                    .await;
                // Stop at first skip - we can't merge out of order
                break;
            }
        }
    }

    Ok(result)
}
