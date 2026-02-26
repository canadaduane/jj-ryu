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
            Self::Merge { bookmark, .. } | Self::Skip { bookmark, .. } => bookmark,
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
/// # Arguments
/// * `analysis` - The submission analysis from `analyze_submission()`
/// * `pr_info` - Map of bookmark name to PR info, pre-fetched by caller
/// * `options` - Planning options (target bookmark, etc.)
///
/// # Returns
/// A `MergePlan` describing the merge operations to perform
#[must_use]
pub fn create_merge_plan<S: BuildHasher>(
    analysis: &SubmissionAnalysis,
    pr_info: &HashMap<String, PrInfo, S>,
    options: &MergePlanOptions,
) -> MergePlan {
    let mut steps = Vec::new();
    let mut bookmarks_to_clear = Vec::new();
    let mut rebase_target = None;
    let mut hit_blocker = false;
    let mut hit_target = false;

    // Process in stack order (trunk → leaf)
    for segment in &analysis.segments {
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
            // Continue to next PR (default: merge all consecutive mergeable)
        }
    }

    let has_actionable = steps.iter().any(|s| matches!(s, MergeStep::Merge { .. }));

    MergePlan {
        steps,
        bookmarks_to_clear,
        rebase_target,
        has_actionable,
    }
}
