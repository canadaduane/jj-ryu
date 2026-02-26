//! Merge command - merge approved PRs in the stack

use crate::cli::context::CommandContext;
use crate::cli::style::{Stylize, check, spinner_style};
use crate::cli::CliProgress;
use anstream::println;
use dialoguer::Confirm;
use indicatif::ProgressBar;
use jj_ryu::error::{Error, Result};
use jj_ryu::graph::build_change_graph;
use jj_ryu::merge::{
    create_merge_plan, execute_merge, MergeConfidence, MergeExecutionResult, MergePlan,
    MergePlanOptions, MergeStep, PrInfo,
};
use jj_ryu::submit::{analyze_submission, create_submission_plan, execute_submission};
use jj_ryu::tracking::{save_pr_cache, save_tracking};
use jj_ryu::types::NarrowedBookmarkSegment;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Options for the merge command
#[derive(Debug, Clone, Default)]
pub struct MergeOptions {
    /// Dry run - show what would be merged without making changes
    pub dry_run: bool,
    /// Preview plan and prompt for confirmation before executing
    pub confirm: bool,
}

/// Run the merge command
#[allow(clippy::too_many_lines, clippy::future_not_send)]
pub async fn run_merge(path: &Path, remote: Option<&str>, options: MergeOptions) -> Result<()> {
    // =========================================================================
    // Phase 1: GATHER - Collect all data upfront
    // =========================================================================

    let mut ctx = CommandContext::new(path, remote).await?;

    // Check tracking
    // Collect into owned strings to avoid borrow checker issues with later mutations
    let tracked_names: Vec<String> = ctx.tracked_names().into_iter().map(String::from).collect();
    if tracked_names.is_empty() {
        return Err(Error::Tracking(
            "No bookmarks tracked. Run 'ryu track' first.".to_string(),
        ));
    }

    // Build change graph
    let graph = build_change_graph(&ctx.workspace)?;

    if graph.stack.is_none() {
        println!("{}", "No stack found between trunk and working copy.".muted());
        return Ok(());
    }

    // Get stack analysis (reuse existing infrastructure)
    let analysis = analyze_submission(&graph, None)?;

    // Filter to tracked bookmarks
    let tracked_segments: Vec<&NarrowedBookmarkSegment> = analysis
        .segments
        .iter()
        .filter(|s| tracked_names.contains(&s.bookmark.name))
        .collect();

    if tracked_segments.is_empty() {
        println!("{}", "No tracked bookmarks in stack.".muted());
        return Ok(());
    }

    // Batch fetch all PR info (details + readiness)
    println!(
        "{}",
        format!("Checking {} tracked bookmark(s)...", tracked_segments.len()).muted()
    );
    let pr_info_map = fetch_all_pr_info(&tracked_segments, &ctx).await?;

    if pr_info_map.is_empty() {
        println!("{}", "No PRs found for tracked bookmarks.".muted());
        return Ok(());
    }

    // =========================================================================
    // Phase 2: PLAN - Pure function, easily testable
    // =========================================================================

    let plan_options = MergePlanOptions {
        target_bookmark: None, // Merge all consecutive mergeable PRs
    };
    let merge_plan = create_merge_plan(&analysis, &pr_info_map, &plan_options, &ctx.default_branch);

    // =========================================================================
    // Phase 3: EXECUTE - Effectful operations
    // =========================================================================

    // Dry run - just report
    if options.dry_run {
        report_merge_dry_run(&merge_plan);
        return Ok(());
    }

    // Nothing to merge
    if merge_plan.is_empty() {
        println!("{}", "No PRs are ready to merge.".muted());
        print_blocking_summary(&merge_plan);
        return Ok(());
    }

    // Confirmation prompt
    if options.confirm {
        report_merge_dry_run(&merge_plan);
        if !Confirm::new()
            .with_prompt("Proceed with merge?")
            .default(true)
            .interact()
            .map_err(|e| Error::Internal(format!("Failed to read confirmation: {e}")))?
        {
            println!("{}", "Aborted".muted());
            return Ok(());
        }
        println!();
    }

    // Execute merges
    println!(
        "{} {}",
        "Merging".emphasis(),
        format!("{} PR(s)...", merge_plan.merge_count()).accent()
    );

    let progress = CliProgress::compact();
    let merge_result = execute_merge(&merge_plan, ctx.platform.as_ref(), &progress).await?;

    // Post-merge cleanup and sync
    if merge_result.bottom_merged() {
        // Clean up merged bookmarks
        for bookmark in &merge_result.merged_bookmarks {
            ctx.pr_cache.remove(bookmark);
            ctx.tracking.untrack(bookmark);
            // Delete local bookmark (ignore errors - may already be gone)
            let _ = ctx.workspace.delete_bookmark(bookmark);
        }

        // Save state - soft failures (merge succeeded, cleanup is best-effort)
        if let Err(e) = save_pr_cache(&ctx.workspace_root, &ctx.pr_cache) {
            println!(
                "{}",
                format!("⚠️  Failed to save PR cache: {e}").warn()
            );
            println!(
                "{}",
                "   Run 'ryu submit' to rebuild.".muted()
            );
        }
        if let Err(e) = save_tracking(&ctx.workspace_root, &ctx.tracking) {
            println!(
                "{}",
                format!("⚠️  Failed to save tracking state: {e}").warn()
            );
        }

        // Post-merge sync: fetch, rebase, re-submit
        post_merge_sync(&mut ctx, &merge_plan, &merge_result).await?;
    } else {
        // Print summary without sync
        print_merge_summary(&merge_result);
    }

    Ok(())
}

/// Fetch all PR info upfront (details + readiness)
#[allow(clippy::future_not_send)]
async fn fetch_all_pr_info(
    segments: &[&NarrowedBookmarkSegment],
    ctx: &CommandContext,
) -> Result<HashMap<String, PrInfo>> {
    let mut result = HashMap::new();

    for segment in segments {
        let bookmark_name = &segment.bookmark.name;

        // Find existing PR
        let Some(existing) = ctx.platform.find_existing_pr(bookmark_name).await? else {
            continue;
        };

        // Fetch details and readiness
        let details = ctx.platform.get_pr_details(existing.number).await?;
        let readiness = ctx.platform.check_merge_readiness(existing.number).await?;

        result.insert(
            bookmark_name.clone(),
            PrInfo {
                bookmark: bookmark_name.clone(),
                details,
                readiness,
            },
        );
    }

    Ok(result)
}

/// Post-merge sync: fetch, rebase remaining stack, re-submit
///
/// Only called when bottom-most PR merged successfully (trunk changed).
#[allow(clippy::future_not_send)]
async fn post_merge_sync(
    ctx: &mut CommandContext,
    plan: &MergePlan,
    merge_result: &MergeExecutionResult,
) -> Result<()> {
    // Fetch to get new main
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(spinner_style());
    spinner.set_message(format!(
        "Fetching from {}...",
        ctx.remote_name.emphasis()
    ));
    spinner.enable_steady_tick(Duration::from_millis(80));

    ctx.workspace.git_fetch(&ctx.remote_name)?;

    spinner.finish_with_message(format!(
        "{} Fetched from {}",
        check(),
        ctx.remote_name.emphasis()
    ));

    // Rebase remaining stack if there's a target
    if let Some(ref next_bookmark) = plan.rebase_target {
        println!(
            "🔄 Rebasing {} onto trunk...",
            next_bookmark.accent()
        );

        if let Err(e) = ctx.workspace.rebase_bookmark_onto_trunk(next_bookmark) {
            // Rebase failure - warn but don't fail the command
            println!(
                "{}",
                format!("⚠️  Rebase failed: {e}").warn()
            );
            println!(
                "{}",
                "   Run 'jj rebase' manually to fix.".muted()
            );
        } else {
            // Re-submit to update PR bases
            println!("📤 Updating remaining PRs...");

            // Re-analyze after rebase
            let graph = build_change_graph(&ctx.workspace)?;
            let analysis = analyze_submission(&graph, None)?;

            // Filter to tracked bookmarks (important!)
            let tracked_names: Vec<String> =
                ctx.tracked_names().into_iter().map(String::from).collect();
            let mut filtered_analysis = analysis.clone();
            filtered_analysis
                .segments
                .retain(|s| tracked_names.contains(&s.bookmark.name));

            if !filtered_analysis.segments.is_empty() {
                // Create submission plan and execute
                let submit_plan = create_submission_plan(
                    &filtered_analysis,
                    ctx.platform.as_ref(),
                    &ctx.remote_name,
                    &ctx.default_branch,
                )
                .await?;

                let progress = CliProgress::compact();
                if let Err(e) = execute_submission(
                    &submit_plan,
                    &mut ctx.workspace,
                    ctx.platform.as_ref(),
                    &progress,
                    false,
                )
                .await
                {
                    // Soft failure - merge succeeded, just PR updates failed
                    println!(
                        "{}",
                        format!("⚠️  Failed to update remaining PRs: {e}").warn()
                    );
                    println!(
                        "{}",
                        "   Run 'ryu submit' to complete the update.".muted()
                    );
                }
            }
        }
    }

    // Summary
    print_merge_summary(merge_result);

    Ok(())
}

/// Print merge summary
fn print_merge_summary(merge_result: &MergeExecutionResult) {
    println!();
    if merge_result.is_success() {
        println!(
            "{} Merge complete!",
            format!("{}", check()).success()
        );
    } else {
        println!(
            "{} Merge partially complete",
            "⚠️".warn()
        );
    }

    if !merge_result.merged_bookmarks.is_empty() {
        println!(
            "   Merged: {}",
            merge_result.merged_bookmarks.join(", ").accent()
        );
    }

    if let Some(ref failed) = merge_result.failed_bookmark {
        if merge_result.was_uncertain {
            println!(
                "   {} {} (merge status was uncertain)",
                "Failed:".warn(),
                failed.warn()
            );
        } else {
            println!("   {} {}", "Failed:".warn(), failed.warn());
        }
        if let Some(ref msg) = merge_result.error_message {
            println!("          {}", msg.muted());
        }
    }
}

/// Report what would be merged (dry run)
fn report_merge_dry_run(plan: &MergePlan) {
    println!("{}:", "Merge plan".emphasis());
    println!();

    if plan.steps.is_empty() {
        println!("  {}", "No PRs to process".muted());
        println!();
        return;
    }

    for step in &plan.steps {
        match step {
            MergeStep::Merge {
                bookmark,
                pr_number,
                pr_title,
                confidence,
                ..
            } => {
                match confidence {
                    MergeConfidence::Certain => {
                        println!(
                            "  {} PR #{}: {}",
                            "✓ Would merge".success(),
                            pr_number,
                            pr_title
                        );
                    }
                    MergeConfidence::Uncertain(reason) => {
                        println!(
                            "  {} PR #{}: {}",
                            "? Would attempt".warn(),
                            pr_number,
                            pr_title
                        );
                        println!("    ⚠ {}", reason.muted());
                    }
                }
                println!("    Bookmark: {}", bookmark.accent());
            }
            MergeStep::RetargetBase {
                bookmark,
                pr_number,
                old_base,
                new_base,
            } => {
                println!(
                    "  {} PR #{} ({}): {} → {}",
                    "↪ Would retarget".accent(),
                    pr_number,
                    bookmark,
                    old_base.muted(),
                    new_base.accent()
                );
            }
            MergeStep::Skip {
                bookmark,
                pr_number,
                reasons,
            } => {
                println!(
                    "  {} PR #{} ({})",
                    "✗ Would skip".warn(),
                    pr_number,
                    bookmark
                );
                for reason in reasons {
                    println!("    - {}", reason.muted());
                }
            }
        }
    }

    println!();
    if plan.has_actionable {
        println!("{}", "Run without --dry-run to execute.".muted());
    } else {
        println!("{}", "No PRs are ready to merge.".muted());
    }
}

/// Print summary of blocking reasons
fn print_blocking_summary(plan: &MergePlan) {
    for step in &plan.steps {
        if let MergeStep::Skip {
            bookmark,
            pr_number,
            reasons,
        } = step
        {
            println!("  PR #{} ({}):", pr_number, bookmark.accent());
            for reason in reasons {
                println!("    - {}", reason.muted());
            }
        }
    }
}
