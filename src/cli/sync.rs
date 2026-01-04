//! Sync command - sync all stacks with remote

use crate::cli::CliProgress;
use crate::cli::style::{CHECK, Stylize, arrow, check, spinner_style};
use anstream::println;
use dialoguer::Confirm;
use indicatif::ProgressBar;
use jj_ryu::error::{Error, Result};
use jj_ryu::graph::build_change_graph;
use jj_ryu::platform::{create_platform_service, parse_repo_info};
use jj_ryu::repo::{JjWorkspace, select_remote};
use jj_ryu::submit::{
    SubmissionPlan, analyze_submission, create_submission_plan, execute_submission,
};
use jj_ryu::types::BranchStack;
use std::path::Path;
use std::time::Duration;

/// Options for the sync command
#[derive(Debug, Clone, Default)]
pub struct SyncOptions<'a> {
    /// Dry run - show what would be done without making changes
    pub dry_run: bool,
    /// Preview plan and prompt for confirmation before executing
    pub confirm: bool,
    /// Only sync the stack containing this bookmark
    pub stack: Option<&'a str>,
}

/// Run the sync command
#[allow(clippy::too_many_lines)]
pub async fn run_sync(path: &Path, remote: Option<&str>, options: SyncOptions<'_>) -> Result<()> {
    // Open workspace
    let mut workspace = JjWorkspace::open(path)?;

    // Get remotes and select one
    let remotes = workspace.git_remotes()?;
    let remote_name = select_remote(&remotes, remote)?;

    // Detect platform
    let remote_info = remotes
        .iter()
        .find(|r| r.name == remote_name)
        .ok_or_else(|| Error::RemoteNotFound(remote_name.clone()))?;

    let platform_config = parse_repo_info(&remote_info.url)?;

    // Create platform service
    let platform = create_platform_service(&platform_config).await?;

    // Fetch from remote with spinner
    if !options.dry_run {
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(spinner_style());
        spinner.set_message(format!("Fetching from {}...", remote_name.emphasis()));
        spinner.enable_steady_tick(Duration::from_millis(80));

        workspace.git_fetch(&remote_name)?;

        spinner.finish_with_message(format!(
            "{} Fetched from {}",
            check(),
            remote_name.emphasis()
        ));
    }

    // Build change graph
    let graph = build_change_graph(&workspace)?;

    if graph.stacks.is_empty() {
        println!("{}", "No stacks to sync".muted());
        return Ok(());
    }

    // Filter stacks if --stack is specified
    let stacks_to_sync: Vec<&BranchStack> = if let Some(stack_bookmark) = options.stack {
        // Find the stack containing this bookmark
        let matching_stack = graph.stacks.iter().find(|stack| {
            stack
                .segments
                .iter()
                .any(|seg| seg.bookmarks.iter().any(|b| b.name == stack_bookmark))
        });

        match matching_stack {
            Some(stack) => vec![stack],
            None => {
                return Err(Error::BookmarkNotFound(format!(
                    "Bookmark '{stack_bookmark}' not found in any stack"
                )));
            }
        }
    } else {
        graph.stacks.iter().collect()
    };

    if stacks_to_sync.is_empty() {
        println!("{}", "No stacks to sync".muted());
        return Ok(());
    }

    let default_branch = workspace.default_branch()?;
    let progress = CliProgress::compact();

    // Build plans for all stacks first (for confirmation)
    let mut stack_plans: Vec<(&str, SubmissionPlan)> = Vec::new();

    for stack in &stacks_to_sync {
        // Get the leaf bookmark (last segment, first bookmark)
        let Some(last_segment) = stack.segments.last() else {
            continue;
        };
        let Some(leaf_bm) = last_segment.bookmarks.first() else {
            continue;
        };
        let leaf_bookmark = &leaf_bm.name;

        let analysis = analyze_submission(&graph, leaf_bookmark)?;
        let plan =
            create_submission_plan(&analysis, platform.as_ref(), &remote_name, &default_branch)
                .await?;

        stack_plans.push((leaf_bookmark, plan));
    }

    // Show confirmation if requested
    if options.confirm && !options.dry_run {
        print_sync_preview(&stack_plans);
        if !Confirm::new()
            .with_prompt("Proceed with sync?")
            .default(true)
            .interact()
            .map_err(|e| Error::Internal(format!("Failed to read confirmation: {e}")))?
        {
            println!("{}", "Aborted".muted());
            return Ok(());
        }
        println!();
    }

    // Sync each stack
    let mut total_pushed = 0;
    let mut total_created = 0;
    let mut total_updated = 0;

    for (leaf_bookmark, plan) in stack_plans {
        println!("{} {}", "Syncing stack:".emphasis(), leaf_bookmark.accent());

        let result = execute_submission(
            &plan,
            &mut workspace,
            platform.as_ref(),
            &progress,
            options.dry_run,
        )
        .await?;

        total_pushed += result.pushed_bookmarks.len();
        total_created += result.created_prs.len();
        total_updated += result.updated_prs.len();
    }

    // Summary
    println!();
    if options.dry_run {
        println!("{}", "Dry run complete".muted());
    } else {
        println!(
            "{} {} pushed, {} created, {} updated",
            format!("{CHECK} Sync complete:").success(),
            total_pushed.accent(),
            total_created.accent(),
            total_updated.accent()
        );
    }

    Ok(())
}

/// Print sync preview for --confirm
fn print_sync_preview(stack_plans: &[(&str, SubmissionPlan)]) {
    println!("{}:", "Sync plan".emphasis());
    println!();

    for (leaf_bookmark, plan) in stack_plans {
        println!("{} {}", "Stack:".emphasis(), leaf_bookmark.accent());

        if plan.execution_steps.is_empty() {
            println!("  {}", "Already in sync".muted());
            println!();
            continue;
        }

        println!("  {}:", "Steps".emphasis());
        for step in &plan.execution_steps {
            println!("    {} {}", arrow(), step);
        }

        println!();
    }
}
