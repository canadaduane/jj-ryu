# Plan: `ryu merge` Command

## Background

Users of jj-ryu manage stacked PRs with `ryu submit`, but merging those PRs back to `main` remains a manual, tedious process:

1. Go to GitHub, copy the PR description, click "squash and merge", paste description
2. Back to CLI: `jj git fetch`, find the stack, rebase onto new `main` (excluding merged commit)
3. Re-submit remaining stack with `ryu submit`
4. Wait for CI (~10 minutes), then repeat from step 1

For an n-deep stack, this is O(n × CI_time) with significant manual intervention at each step.

## Problem Statement

There is no automated way to merge approved PRs in a stack. Users must manually:
- Copy/paste PR descriptions into merge commit messages
- Navigate between GitHub UI and CLI for each PR
- Manually rebase and re-submit after each merge

## Success Criteria

1. `ryu merge` merges the bottom-most approved PR in the stack via GitHub API
2. Merge uses squash strategy with PR description as commit message
3. After merge, automatically fetches, rebases remaining stack, and re-submits
4. `ryu merge --all` merges all consecutive approved PRs (stops at first non-approved)
5. Clear feedback on merge status, CI status, and approval status
6. Graceful handling of merge failures (conflicts, CI not passing, etc.)
7. PR cache is updated after each successful merge

## The Gap

| Component | Current | Target |
|-----------|---------|--------|
| `PlatformService` | No merge/status methods | Add `get_pr_details`, `check_merge_readiness`, `merge_pr` |
| CLI | No merge command | Add `Commands::Merge` with `--all`, `--dry-run`, `--confirm` flags |
| Types | No merge-related types | Add `PullRequestDetails`, `MergeReadiness`, `MergeResult` |
| Post-merge | Manual fetch/rebase | Auto fetch, rebase remaining stack, re-submit |
| CLI setup | Duplicated in submit/sync | Extract `CommandContext` for reuse |
| Merge logic | N/A | New `src/merge/` module following submit pattern |

## Transitive Effect Analysis

```
CLI Context (cli/context.rs) - NEW
  └── CommandContext - shared setup for submit/sync/merge

PlatformService (platform/mod.rs)
  └── GitHubService (platform/github.rs) - must implement new methods
  └── GitLabService (platform/gitlab.rs) - must implement new methods
  └── MockPlatformService (tests/common/mock_platform.rs) - must implement for tests

Types (types.rs)
  └── PullRequestDetails - extended PR info including body, state
  └── MergeReadiness - approval status, CI status, mergeable flag
  └── MergeResult - merge outcome

Merge Module (merge/) - NEW, follows submit/ pattern
  └── plan.rs - MergePlan, MergeStep, create_merge_plan() (pure)
  └── execute.rs - execute_merge() (effectful)
  └── mod.rs - re-exports

CLI (main.rs)
  └── Commands::Merge - new variant
  └── cli/merge.rs - orchestrates three-phase merge

JjWorkspace (repo/workspace.rs)
  └── git_fetch() - already exists, reused after merge
  └── rebase_bookmark_onto_trunk() - NEW: rebase bookmark onto trunk

Tracking (tracking/)
  └── PrCache - clear entry after successful merge
```

**Files affected:**
- `src/cli/context.rs` - NEW: shared command context
- `src/merge/mod.rs` - NEW: module re-exports
- `src/merge/plan.rs` - NEW: MergePlan, create_merge_plan()
- `src/merge/execute.rs` - NEW: execute_merge()
- `src/types.rs` - new types (PullRequestDetails, MergeReadiness, etc.)
- `src/platform/mod.rs` - trait extension
- `src/platform/github.rs` - GitHub implementation
- `src/platform/gitlab.rs` - GitLab implementation
- `src/main.rs` - CLI command
- `src/cli/mod.rs` - module export
- `src/cli/merge.rs` - CLI orchestrator
- `src/cli/submit.rs` - refactor to use CommandContext
- `src/cli/sync.rs` - refactor to use CommandContext
- `src/lib.rs` - export merge module
- `src/repo/workspace.rs` - rebase helper
- `src/error.rs` - new error variants
- `tests/common/mock_platform.rs` - mock implementation
- `tests/integration_tests.rs` - new tests
- `tests/unit_tests.rs` - pure function tests

---

## Decisions

### D1: `bookmark` parameter semantics

The optional `bookmark` parameter specifies the **top of the merge range**:
- `ryu merge` - merges bottom-most mergeable PR only
- `ryu merge --all` - merges all consecutive mergeable PRs from bottom
- `ryu merge feat-3` - merges from bottom up to and including `feat-3` (if all mergeable)
- `ryu merge feat-3 --all` - same as above (explicit top)

When `bookmark` is specified without `--all`, merge only that specific bookmark (must be the bottom-most, or error).

### D2: Module structure follows `submit/` pattern

```
src/merge/
├── mod.rs      # Re-exports
├── plan.rs     # MergePlan, MergeStep, create_merge_plan() - PURE
└── execute.rs  # execute_merge() - EFFECTFUL
```

This mirrors `src/submit/` and enables:
- Pure planning function is testable in `tests/unit_tests.rs`
- Exported from `lib.rs` for library users
- Clear separation of concerns

### D3: `CommandContext` does NOT include `graph`

The change graph becomes stale after fetch/rebase operations. Instead of storing a stale `graph` in context:
- `CommandContext` provides workspace, tracking, platform, remote info
- Callers build `graph` when needed via `build_change_graph()`
- After fetch/rebase, callers rebuild the graph

---

## Learnings

### 1. Reuse Existing Stack Analysis

The `analyze_submission()` function already builds ordered `NarrowedBookmarkSegment` from trunk to leaf via `SubmissionAnalysis`. The merge logic should reuse this rather than reimplementing stack traversal:

```rust
// Instead of custom find_stack_prs(), use:
let analysis = analyze_submission(&graph, None)?;
// analysis.segments is ordered trunk → leaf
```

### 2. PR Cache Integration

`PrCache` in `src/tracking/pr_cache.rs` caches bookmark→PR mappings. After a successful merge:
- Call `cache.remove(bookmark_name)` to clear the entry
- Call `save_pr_cache()` to persist

This prevents stale cache entries for merged bookmarks.

### 3. Partial Merge Handling

With `--all`, if PRs #1, #2, #3 merge but #4 fails:
- PRs #1-3 are committed (cannot undo)
- Must clearly report what succeeded vs failed
- Must still fetch/rebase after partial success
- Track `merged_bookmarks: Vec<String>` for cache cleanup

### 4. Merge Method Platform Differences

```
MergeMethod::Squash  → GitHub: "squash"    | GitLab: squash: true
MergeMethod::Merge   → GitHub: "merge"     | GitLab: merge_method: "merge"
MergeMethod::Rebase  → GitHub: "rebase"    | GitLab: merge_method: "fast_forward" (different semantics)
```

Squash is the primary use case; rebase mapping differences are acceptable for MVP.

### 5. GraphQL Optimization (Future)

`publish_pr` already uses GraphQL. A single GraphQL query could batch PR details + reviews + check status. This is an optimization for later, not blocking v1.

### 6. CLI Setup Duplication

`run_submit()` and `run_sync()` share ~30 lines of identical setup:
- Open workspace
- Load tracking
- Select remote
- Parse platform config
- Create platform service

Extract to `CommandContext` to eliminate duplication and simplify merge implementation.

### 7. Functional Core / Imperative Shell

The existing `submit` module follows a three-phase pattern:
1. `analysis.rs` - Pure analysis → `SubmissionAnalysis`
2. `plan.rs` - Pure planning → `SubmissionPlan`
3. `execute.rs` - Effectful execution

The merge command should follow the same pattern:
1. **Gather** - Fetch all data upfront (PR details, readiness)
2. **Plan** - Pure `create_merge_plan()` → `MergePlan` (easily testable!)
3. **Execute** - Effectful `execute_merge()` with progress reporting

### 8. Batch Readiness Fetching

Fetch all PR readiness info upfront rather than one-at-a-time in a loop:
- More efficient (parallelizable)
- Enables pure planning phase
- Better dry-run experience (show all statuses immediately)

### 9. Reuse `execute_submission()` for Re-submit

After merge+rebase, don't duplicate PR update logic. Instead:
1. Re-analyze the rebased stack
2. Create a new `SubmissionPlan`
3. Call existing `execute_submission()`

### 10. Tracking Filter Required in Re-submit

After merge, when re-submitting remaining PRs, must filter to tracked bookmarks just like `run_submit()` does. Otherwise untracked bookmarks could be submitted.

### 11. Re-submit Failure is Soft Failure

If `execute_submission()` fails after successful merge+rebase:
- The merge succeeded (good!)
- User is left with rebased local state (good!)
- PRs just need base updates (minor)
- Warn user and suggest `ryu submit` to complete

### 12. Borrow Checker with Context Structs

When `CommandContext` stores data that returns borrowed references (like `tracking.tracked_names()` returning `Vec<&str>`), subsequent mutations to other fields (like `workspace.git_fetch()`) cause borrow conflicts. **Solution**: Collect borrowed data into owned `Vec<String>` before mutations:

```rust
// Collect into owned strings to avoid borrow checker issues with later mutations
let tracked_names: Vec<String> = ctx.tracked_names().into_iter().map(String::from).collect();
```

This also changes the filter syntax from `contains(&s.bookmark.name.as_str())` to `contains(&s.bookmark.name)`.

### 13. CommandContext is Internal Only

`CommandContext` is only used within the `cli` module and should NOT be publicly exported from `cli/mod.rs`. Use `mod context;` not `pub use context::CommandContext;`.

### 14. Display Trait for API Parameters

Types like `MergeMethod` should implement `Display` to produce lowercase strings suitable for API parameters:

```rust
impl std::fmt::Display for MergeMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Squash => write!(f, "squash"),
            // ...
        }
    }
}
```

### 15. Mark Future-Use Code with allow(dead_code)

When adding methods that will be used by future phases (like `has_tracked_bookmarks()`), add `#[allow(dead_code)]` with a comment:

```rust
#[allow(dead_code)] // Will be used by merge command
pub fn has_tracked_bookmarks(&self) -> bool { ... }
```

---

## Phase 0: Extract Command Context (Refactor) ✅

### Rationale

Before implementing merge, extract the shared CLI setup code that's duplicated between `run_submit()` and `run_sync()`. This:
- Eliminates ~30 lines of duplication per command
- Provides a clean foundation for `run_merge()`
- Makes future commands easier to implement

### Tasks
- ✅ Create `src/cli/context.rs` with `CommandContext` struct
- ✅ Refactor `run_submit()` to use `CommandContext`
- ✅ Refactor `run_sync()` to use `CommandContext`
- ✅ Export from `src/cli/mod.rs`

### Implementation

```rust
// src/cli/context.rs

use jj_ryu::error::{Error, Result};
use jj_ryu::platform::{create_platform_service, parse_repo_info, PlatformService};
use jj_ryu::repo::{select_remote, JjWorkspace};
use jj_ryu::tracking::{load_pr_cache, load_tracking, PrCache, TrackingState};
use std::path::{Path, PathBuf};

/// Shared context for CLI commands that interact with the platform
/// 
/// Note: Does NOT include ChangeGraph because it becomes stale after
/// fetch/rebase operations. Callers should build graph when needed.
pub struct CommandContext {
    pub workspace: JjWorkspace,
    pub workspace_root: PathBuf,
    pub tracking: TrackingState,
    pub pr_cache: PrCache,
    pub platform: Box<dyn PlatformService>,
    pub remote_name: String,
    pub default_branch: String,
}

impl CommandContext {
    /// Create a new command context
    ///
    /// This performs the common setup shared by submit/sync/merge:
    /// - Open workspace
    /// - Load tracking state
    /// - Load PR cache
    /// - Select and validate remote
    /// - Detect platform and create service
    pub async fn new(path: &Path, remote: Option<&str>) -> Result<Self> {
        // Open workspace
        let workspace = JjWorkspace::open(path)?;
        let workspace_root = workspace.workspace_root().to_path_buf();

        // Load tracking and PR cache
        let tracking = load_tracking(&workspace_root)?;
        let pr_cache = load_pr_cache(&workspace_root)?;

        // Get remotes and select one
        let remotes = workspace.git_remotes()?;
        let remote_name = select_remote(&remotes, remote)?;

        // Detect platform from remote URL
        let remote_info = remotes
            .iter()
            .find(|r| r.name == remote_name)
            .ok_or_else(|| Error::RemoteNotFound(remote_name.clone()))?;

        let platform_config = parse_repo_info(&remote_info.url)?;

        // Create platform service
        let platform = create_platform_service(&platform_config).await?;

        // Get default branch
        let default_branch = workspace.default_branch()?;

        Ok(Self {
            workspace,
            workspace_root,
            tracking,
            pr_cache,
            platform,
            remote_name,
            default_branch,
        })
    }

    /// Check if any bookmarks are tracked
    pub fn has_tracked_bookmarks(&self) -> bool {
        !self.tracking.tracked_names().is_empty()
    }

    /// Get tracked bookmark names
    pub fn tracked_names(&self) -> Vec<&str> {
        self.tracking.tracked_names()
    }
}
```

### Refactored `run_submit()` (sketch)

```rust
pub async fn run_submit(path: &Path, bookmark: Option<&str>, remote: Option<&str>, options: SubmitOptions<'_>) -> Result<()> {
    // Validate options
    if options.draft && options.publish {
        return Err(Error::InvalidArgument("Cannot use --draft and --publish together".to_string()));
    }

    // Create shared context
    let mut ctx = CommandContext::new(path, remote).await?;

    // Check tracking (unless --all)
    if !options.all && !ctx.has_tracked_bookmarks() {
        return Err(Error::Tracking("No bookmarks tracked...".to_string()));
    }

    // Build change graph
    let graph = build_change_graph(&ctx.workspace)?;
    
    if graph.stack.is_none() {
        println!("No bookmarks found between trunk and working copy.");
        return Ok(());
    }

    // ... rest of submit logic using ctx.workspace, ctx.platform, etc.
}
```

---

## Phase 1: Extend Type System ✅

### Tasks
- ✅ Add `PullRequestDetails` struct to `src/types.rs`
- ✅ Add `PrState` enum to `src/types.rs`
- ✅ Add `MergeReadiness` struct to `src/types.rs`
- ✅ Add `MergeResult` struct to `src/types.rs`
- ✅ Add `MergeMethod` enum to `src/types.rs`

### Type Definitions

```rust
/// PR state (open, closed, merged)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

/// Extended PR details for merge operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestDetails {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: PrState,
    pub is_draft: bool,
    pub mergeable: Option<bool>,
    pub head_ref: String,
    pub base_ref: String,
    pub html_url: String,
}

/// Merge readiness check result
#[derive(Debug, Clone)]
pub struct MergeReadiness {
    pub is_approved: bool,
    pub ci_passed: bool,
    pub is_mergeable: bool,
    pub is_draft: bool,
    pub blocking_reasons: Vec<String>,
}

impl MergeReadiness {
    pub fn can_merge(&self) -> bool {
        self.is_approved && self.ci_passed && self.is_mergeable && !self.is_draft
    }
}

/// Result of a merge operation
#[derive(Debug, Clone)]
pub struct MergeResult {
    pub merged: bool,
    pub sha: Option<String>,
    pub message: Option<String>,
}

/// Merge strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}
```

---

## Phase 2: Extend Platform Trait 🔴

### Tasks
- 🔴 Add `get_pr_details()` method to `PlatformService` trait
- 🔴 Add `check_merge_readiness()` method to `PlatformService` trait
- 🔴 Add `merge_pr()` method to `PlatformService` trait

### Trait Extension

```rust
#[async_trait]
pub trait PlatformService: Send + Sync {
    // ... existing methods ...

    /// Get full PR details including body and state
    async fn get_pr_details(&self, pr_number: u64) -> Result<PullRequestDetails>;

    /// Check if PR is ready to merge (approved, CI passed, no conflicts)
    async fn check_merge_readiness(&self, pr_number: u64) -> Result<MergeReadiness>;

    /// Merge a PR with the specified method
    /// For squash: uses PR title as commit title, PR body as commit message
    async fn merge_pr(&self, pr_number: u64, method: MergeMethod) -> Result<MergeResult>;
}
```

---

## Phase 3: GitHub Implementation 🔴

### Tasks
- 🔴 Implement `get_pr_details()` in `GitHubService`
- 🔴 Implement `check_merge_readiness()` in `GitHubService`
- 🔴 Implement `merge_pr()` in `GitHubService`

### API Endpoints Used

```
GET /repos/{owner}/{repo}/pulls/{pr_number}
  -> title, body, state, draft, mergeable, head.ref, base.ref

GET /repos/{owner}/{repo}/pulls/{pr_number}/reviews
  -> Check for APPROVED reviews

GET /repos/{owner}/{repo}/commits/{ref}/check-runs
  -> CI status (or use combined status endpoint)

PUT /repos/{owner}/{repo}/pulls/{pr_number}/merge
  {
    "merge_method": "squash",
    "commit_title": "{title} (#{number})",
    "commit_message": "{body}"
  }
```

### Implementation Notes

- `octocrab` (v0.47) provides `pulls().get()` for PR details
- Reviews endpoint: `pulls().list_reviews()` or raw API call
- Check runs: may need raw API call via `octocrab._get()`
- Merge: `pulls().merge()` with options

---

## Phase 4: GitLab Implementation 🔴

### Tasks
- 🔴 Implement `get_pr_details()` in `GitLabService`
- 🔴 Implement `check_merge_readiness()` in `GitLabService`
- 🔴 Implement `merge_pr()` in `GitLabService`

### API Endpoints Used

```
GET /projects/{id}/merge_requests/{mr_iid}
  -> title, description, state, draft, merge_status

GET /projects/{id}/merge_requests/{mr_iid}/approvals
  -> Approval status

GET /projects/{id}/merge_requests/{mr_iid}/pipelines
  -> CI status

PUT /projects/{id}/merge_requests/{mr_iid}/merge
  {
    "squash": true,
    "squash_commit_message": "{title}\n\n{description}"
  }
```

---

## Phase 5: CLI Command 🔴

### Tasks
- 🔴 Add `Commands::Merge` variant to `src/main.rs`
- 🔴 Create `src/cli/merge.rs` module
- 🔴 Export from `src/cli/mod.rs`
- 🔴 Wire up in `main()` match arm

### CLI Definition

```rust
/// Merge approved PRs in the stack
Merge {
    /// Bookmark to merge up to (defaults to bottom-most tracked)
    bookmark: Option<String>,

    /// Merge all consecutive approved PRs in the stack
    #[arg(long)]
    all: bool,

    /// Dry run - show what would be merged without merging
    #[arg(long)]
    dry_run: bool,

    /// Preview plan and prompt for confirmation before executing
    #[arg(long, short = 'c')]
    confirm: bool,

    /// Git remote to use
    #[arg(long)]
    remote: Option<String>,
}
```

---

## Phase 6: Merge Module (Following submit/ Pattern) 🔴

### Structure

```
src/merge/
├── mod.rs      # Re-exports
├── plan.rs     # MergePlan, MergeStep, create_merge_plan() - PURE
└── execute.rs  # execute_merge() - EFFECTFUL
```

### Tasks
- 🔴 Create `src/merge/mod.rs` with re-exports
- 🔴 Create `src/merge/plan.rs` with `MergePlan`, `MergeStep`, `create_merge_plan()`
- 🔴 Create `src/merge/execute.rs` with `execute_merge()`
- 🔴 Export from `src/lib.rs`

### `src/merge/mod.rs`

```rust
//! Merge engine for stacked PRs
//!
//! Three-phase pattern matching submit/:
//! 1. Gather - fetch PR details and readiness (effectful, bounded)
//! 2. Plan - create MergePlan (pure, testable)
//! 3. Execute - perform merges (effectful)

mod execute;
mod plan;

pub use execute::{execute_merge, MergeExecutionResult};
pub use plan::{create_merge_plan, MergePlan, MergeStep, PrInfo};
```

### `src/merge/plan.rs` (Pure - Testable)

```rust
//! Merge planning - pure functions for creating merge plans

use crate::submit::SubmissionAnalysis;
use crate::types::{MergeMethod, MergeReadiness, PullRequestDetails};
use std::collections::HashMap;

/// Gathered PR information for planning
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub bookmark: String,
    pub details: PullRequestDetails,
    pub readiness: MergeReadiness,
}

/// A single step in the merge plan
#[derive(Debug, Clone)]
pub enum MergeStep {
    /// Merge this PR
    Merge {
        bookmark: String,
        pr_number: u64,
        pr_title: String,
        method: MergeMethod,
    },
    /// Skip this PR (not ready to merge)
    Skip {
        bookmark: String,
        pr_number: u64,
        reasons: Vec<String>,
    },
}

/// Options for merge planning
#[derive(Debug, Clone, Default)]
pub struct MergePlanOptions {
    /// Merge all consecutive mergeable PRs
    pub all: bool,
    /// Target bookmark (merge up to this bookmark)
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
    /// Whether there are any mergeable PRs
    pub has_mergeable: bool,
}

impl MergePlan {
    /// Check if the plan has any merge steps
    pub fn is_empty(&self) -> bool {
        !self.steps.iter().any(|s| matches!(s, MergeStep::Merge { .. }))
    }

    /// Count mergeable PRs
    pub fn merge_count(&self) -> usize {
        self.steps.iter().filter(|s| matches!(s, MergeStep::Merge { .. })).count()
    }
}

/// Create a merge plan (PURE - no I/O, easily testable)
pub fn create_merge_plan(
    analysis: &SubmissionAnalysis,
    pr_info: &HashMap<String, PrInfo>,
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
            continue; // No PR for this bookmark
        };
        
        if hit_blocker {
            // After hitting a blocker, remaining PRs become the rebase target
            if rebase_target.is_none() {
                rebase_target = Some(bookmark_name.clone());
            }
            continue;
        }
        
        if info.readiness.can_merge() {
            steps.push(MergeStep::Merge {
                bookmark: bookmark_name.clone(),
                pr_number: info.details.number,
                pr_title: info.details.title.clone(),
                method: MergeMethod::Squash,
            });
            bookmarks_to_clear.push(bookmark_name.clone());
            
            // If not --all, stop after first mergeable
            if !options.all {
                hit_blocker = true;
            }
        } else {
            steps.push(MergeStep::Skip {
                bookmark: bookmark_name.clone(),
                pr_number: info.details.number,
                reasons: info.readiness.blocking_reasons.clone(),
            });
            hit_blocker = true;
            if rebase_target.is_none() {
                rebase_target = Some(bookmark_name.clone());
            }
        }
    }
    
    let has_mergeable = steps.iter().any(|s| matches!(s, MergeStep::Merge { .. }));
    
    MergePlan {
        steps,
        bookmarks_to_clear,
        rebase_target,
        has_mergeable,
    }
}
```

### `src/merge/execute.rs` (Effectful)

```rust
//! Merge execution - effectful operations

use crate::error::Result;
use crate::merge::plan::{MergePlan, MergeStep};
use crate::platform::PlatformService;
use crate::submit::ProgressCallback;
use crate::tracking::PrCache;
use crate::types::MergeMethod;

/// Result of merge execution
#[derive(Debug, Clone, Default)]
pub struct MergeExecutionResult {
    pub merged_bookmarks: Vec<String>,
    pub failed_bookmark: Option<String>,
    pub error_message: Option<String>,
}

/// Execute the merge plan (EFFECTFUL)
pub async fn execute_merge(
    plan: &MergePlan,
    platform: &dyn PlatformService,
    pr_cache: &mut PrCache,
    progress: &dyn ProgressCallback,
) -> Result<MergeExecutionResult> {
    let mut result = MergeExecutionResult::default();
    
    for step in &plan.steps {
        match step {
            MergeStep::Merge { bookmark, pr_number, pr_title, method } => {
                progress.on_message(&format!("🔀 Merging PR #{}: {}", pr_number, pr_title)).await;
                
                match platform.merge_pr(*pr_number, *method).await {
                    Ok(merge_result) if merge_result.merged => {
                        progress.on_message(&format!(
                            "✅ Merged: {}", 
                            merge_result.sha.as_deref().unwrap_or("(no sha)")
                        )).await;
                        result.merged_bookmarks.push(bookmark.clone());
                        
                        // Clear from PR cache
                        pr_cache.remove(bookmark);
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
            MergeStep::Skip { bookmark, pr_number, reasons } => {
                progress.on_message(&format!(
                    "⏭️  Skipping PR #{} ({}): {}", 
                    pr_number, bookmark, reasons.join(", ")
                )).await;
                break; // Stop at first skip
            }
        }
    }
    
    Ok(result)
}
```

---

## Phase 6b: CLI Orchestrator 🔴

### Tasks
- 🔴 Implement `run_merge()` orchestrator in `src/cli/merge.rs`
- 🔴 Implement `fetch_all_pr_info()` - batch data gathering
- 🔴 Implement `report_merge_dry_run()` - dry run output
- 🔴 Implement `post_merge_sync()` - fetch, rebase, re-submit

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    run_merge() orchestrator                  │
├─────────────────────────────────────────────────────────────┤
│  Phase 1: GATHER (effectful, bounded)                       │
│    - CommandContext::new()                                  │
│    - build_change_graph()                                   │
│    - analyze_submission()                                   │
│    - fetch_all_pr_info() ← batch API calls                  │
├─────────────────────────────────────────────────────────────┤
│  Phase 2: PLAN (pure, testable)                             │
│    - create_merge_plan() → MergePlan                        │
├─────────────────────────────────────────────────────────────┤
│  Phase 3: EXECUTE (effectful)                               │
│    - execute_merge() or report_dry_run()                    │
│    - post_merge_sync(): fetch, rebase, re-submit            │
└─────────────────────────────────────────────────────────────┘
```

### Implementation

```rust
// src/cli/merge.rs

use crate::cli::context::CommandContext;
use crate::cli::style::{Stylize, check, spinner_style};
use crate::cli::CliProgress;
use anstream::println;
use dialoguer::Confirm;
use indicatif::ProgressBar;
use jj_ryu::error::{Error, Result};
use jj_ryu::graph::build_change_graph;
use jj_ryu::merge::{create_merge_plan, execute_merge, MergePlan, MergePlanOptions, MergeStep, PrInfo};
use jj_ryu::submit::{analyze_submission, create_submission_plan, execute_submission};
use jj_ryu::tracking::save_pr_cache;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Options for the merge command
pub struct MergeOptions {
    pub dry_run: bool,
    pub confirm: bool,
    pub all: bool,
}

/// Run the merge command
pub async fn run_merge(
    path: &Path,
    bookmark: Option<&str>,
    remote: Option<&str>,
    options: MergeOptions,
) -> Result<()> {
    // =========================================================================
    // Phase 1: GATHER - Collect all data upfront
    // =========================================================================
    
    let mut ctx = CommandContext::new(path, remote).await?;
    
    // Check tracking
    if !ctx.has_tracked_bookmarks() {
        return Err(Error::Tracking(
            "No bookmarks tracked. Run 'ryu track' first.".to_string()
        ));
    }
    
    // Build change graph
    let graph = build_change_graph(&ctx.workspace)?;
    
    if graph.stack.is_none() {
        println!("{}", "No stack found between trunk and working copy.".muted());
        return Ok(());
    }
    
    // Get stack analysis (reuse existing infrastructure)
    let analysis = analyze_submission(&graph, bookmark)?;
    
    // Filter to tracked bookmarks
    // Collect into owned strings to avoid borrow checker issues with later mutations
    let tracked_names: Vec<String> = ctx.tracked_names().into_iter().map(String::from).collect();
    let tracked_segments: Vec<_> = analysis.segments.iter()
        .filter(|s| tracked_names.contains(&s.bookmark.name))
        .collect();
    
    if tracked_segments.is_empty() {
        println!("{}", "No tracked bookmarks with PRs in stack.".muted());
        return Ok(());
    }
    
    // Batch fetch all PR info (details + readiness)
    let pr_info_map = fetch_all_pr_info(&tracked_segments, &ctx).await?;
    
    if pr_info_map.is_empty() {
        println!("{}", "No PRs found for tracked bookmarks.".muted());
        return Ok(());
    }

    // =========================================================================
    // Phase 2: PLAN - Pure function, easily testable
    // =========================================================================
    
    let plan_options = MergePlanOptions {
        all: options.all,
        target_bookmark: bookmark.map(String::from),
    };
    let merge_plan = create_merge_plan(&analysis, &pr_info_map, &plan_options);

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
    let progress = CliProgress::compact();
    let merge_result = execute_merge(
        &merge_plan, 
        ctx.platform.as_ref(), 
        &mut ctx.pr_cache,
        &progress
    ).await?;
    
    // Post-merge: save cache, fetch, rebase, re-submit
    if !merge_result.merged_bookmarks.is_empty() {
        save_pr_cache(&ctx.workspace_root, &ctx.pr_cache)?;
        post_merge_sync(&mut ctx, &merge_plan, &merge_result).await?;
    }
    
    Ok(())
}

/// Fetch all PR info upfront (details + readiness)
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
        
        result.insert(bookmark_name.clone(), PrInfo {
            bookmark: bookmark_name.clone(),
            details,
            readiness,
        });
    }
    
    Ok(result)
}

/// Post-merge operations: fetch, rebase, re-submit
async fn post_merge_sync(
    ctx: &mut CommandContext,
    plan: &MergePlan,
    merge_result: &MergeExecutionResult,
) -> Result<()> {
    // Fetch to get new main
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(spinner_style());
    spinner.set_message(format!("Fetching from {}...", ctx.remote_name.emphasis()));
    spinner.enable_steady_tick(Duration::from_millis(80));
    
    ctx.workspace.git_fetch(&ctx.remote_name)?;
    
    spinner.finish_with_message(format!("{} Fetched from {}", check(), ctx.remote_name.emphasis()));
    
    // Rebase remaining stack if there's a target
    if let Some(ref next_bookmark) = plan.rebase_target {
        println!("🔄 Rebasing remaining stack onto trunk...");
        ctx.workspace.rebase_bookmark_onto_trunk(next_bookmark)?;
        
        // Re-submit to update PR bases
        println!("📤 Updating remaining PRs...");
        
        // Re-analyze after rebase
        let graph = build_change_graph(&ctx.workspace)?;
        let analysis = analyze_submission(&graph, None)?;
        
        // Filter to tracked bookmarks (important!)
        // Use owned strings since we may have mutated ctx earlier
        let tracked_names: Vec<String> = ctx.tracked_names().into_iter().map(String::from).collect();
        let mut filtered_analysis = analysis.clone();
        filtered_analysis.segments.retain(|s| tracked_names.contains(&s.bookmark.name));
        
        // Create submission plan and execute
        let submit_plan = create_submission_plan(
            &filtered_analysis, 
            ctx.platform.as_ref(), 
            &ctx.remote_name, 
            &ctx.default_branch
        ).await?;
        
        let progress = CliProgress::compact();
        if let Err(e) = execute_submission(&submit_plan, &mut ctx.workspace, ctx.platform.as_ref(), &progress, false).await {
            // Soft failure - merge succeeded, just PR updates failed
            println!("⚠️  Failed to update remaining PRs: {}", e);
            println!("   Run 'ryu submit' to complete the update.");
        }
    }
    
    // Summary
    println!();
    println!("✅ {} Merge complete!", check());
    println!("   Merged: {}", merge_result.merged_bookmarks.join(", ").accent());
    
    if let Some(ref failed) = merge_result.failed_bookmark {
        println!("   ⚠️  Failed: {} ({})", failed.warning(), merge_result.error_message.as_deref().unwrap_or("unknown"));
    }
    
    Ok(())
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
            MergeStep::Merge { bookmark, pr_number, pr_title, .. } => {
                println!("  {} PR #{}: {}", "✓ Would merge".success(), pr_number, pr_title);
                println!("    Bookmark: {}", bookmark.accent());
            }
            MergeStep::Skip { bookmark, pr_number, reasons } => {
                println!("  {} PR #{} ({})", "✗ Would skip".warning(), pr_number, bookmark);
                for reason in reasons {
                    println!("    - {}", reason.muted());
                }
            }
        }
    }
    
    println!();
    if plan.has_mergeable {
        println!("{}", "Run without --dry-run to execute.".muted());
    } else {
        println!("{}", "No PRs are ready to merge.".muted());
    }
}

/// Print summary of blocking reasons
fn print_blocking_summary(plan: &MergePlan) {
    for step in &plan.steps {
        if let MergeStep::Skip { bookmark, pr_number, reasons } = step {
            println!("  PR #{} ({}):", pr_number, bookmark.accent());
            for reason in reasons {
                println!("    - {}", reason.muted());
            }
        }
    }
}
```

---

## Phase 6c: Workspace Rebase Helper 🔴

### Tasks
- 🔴 Add `rebase_bookmark_onto_trunk()` to `JjWorkspace`

### Implementation

```rust
impl JjWorkspace {
    /// Rebase a bookmark and its descendants onto trunk
    /// 
    /// After a merge, the bottom of the stack is now in trunk.
    /// This rebases the next bookmark (and everything above it) onto the new trunk.
    pub fn rebase_bookmark_onto_trunk(&mut self, bookmark: &str) -> Result<()> {
        // Use jj rebase -b <bookmark> -d trunk()
        // This moves the bookmark and all its descendants onto trunk
        
        let output = std::process::Command::new("jj")
            .args(["rebase", "-b", bookmark, "-d", "trunk()"])
            .current_dir(&self.workspace_path)
            .output()
            .map_err(|e| Error::RebaseFailed(format!("Failed to run jj rebase: {e}")))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::RebaseFailed(stderr.to_string()));
        }
        
        Ok(())
    }
}
```

**Note**: Using CLI `jj rebase` for simplicity. Could use jj-lib directly for tighter integration, but CLI is more maintainable and already battle-tested.

---

## Phase 7: Error Handling 🔴

### Tasks
- 🔴 Add `RebaseFailed` error variant to `src/error.rs`

### Error Variants

```rust
/// Rebase failed after merge
#[error("rebase failed: {0}")]
RebaseFailed(String),
```

**Note**: Other error variants (`MergeConflict`, `NotApproved`, `CiNotPassing`, `DraftPr`) are not needed as errors - they're represented as `MergeReadiness.blocking_reasons` instead.

---

## Phase 8: Tests 🔴

### Unit Tests (Pure Planning Function)
- 🔴 Test `create_merge_plan()` with single mergeable PR
- 🔴 Test `create_merge_plan()` with multiple mergeable PRs + `all: true`
- 🔴 Test `create_merge_plan()` stops at first non-mergeable
- 🔴 Test `create_merge_plan()` with no mergeable PRs
- 🔴 Test `create_merge_plan()` with `target_bookmark` specified
- 🔴 Test `MergeReadiness::can_merge()` logic
- 🔴 Test `MergePlan::is_empty()` and `merge_count()`

### Integration Tests
- 🔴 Test `run_merge` with single approved PR (merge + rebase + re-submit)
- 🔴 Test `run_merge --all` with multiple approved PRs
- 🔴 Test `run_merge` stops at first non-approved PR
- 🔴 Test `run_merge --dry-run` shows status without merging
- 🔴 Test `run_merge --confirm` prompts before execution
- 🔴 Test merge failure handling (API error)
- 🔴 Test partial merge (2 succeed, 3rd fails) - verify cache cleanup for succeeded
- 🔴 Test PR cache is cleared after successful merge
- 🔴 Test re-submit failure is soft (merge still reported as success)
- 🔴 Test `CommandContext::new()` setup

### Mock Extensions

Add to `MockPlatformService`:

```rust
// New response maps
get_pr_details_responses: Mutex<HashMap<u64, PullRequestDetails>>,
merge_readiness_responses: Mutex<HashMap<u64, MergeReadiness>>,
merge_responses: Mutex<HashMap<u64, MergeResult>>,

// New call tracking
get_pr_details_calls: Mutex<Vec<u64>>,
check_merge_readiness_calls: Mutex<Vec<u64>>,
merge_pr_calls: Mutex<Vec<(u64, MergeMethod)>>,

// Setters
pub fn set_pr_details(&self, pr_number: u64, details: PullRequestDetails);
pub fn set_merge_readiness(&self, pr_number: u64, readiness: MergeReadiness);
pub fn set_merge_result(&self, pr_number: u64, result: MergeResult);

// Assertions
pub fn assert_merge_called(&self, pr_number: u64);
pub fn assert_merge_not_called(&self, pr_number: u64);
```

---

## Phase 9: Documentation 🔴

### Tasks
- 🔴 Update `README.md` with `ryu merge` command documentation
- 🔴 Update `AGENTS.md` with merge command entry in WHERE TO LOOK table
- 🔴 Create `src/merge/AGENTS.md` following submit pattern
- 🔴 Add merge workflow example to README

### README Addition

```markdown
### Merging

After PRs are approved:

```sh
# Merge the bottom-most approved PR, rebase remaining stack, update PRs
ryu merge

# Merge all consecutive approved PRs in one go
ryu merge --all

# Preview what would be merged
ryu merge --dry-run

# Preview and confirm before merging
ryu merge --confirm
```

The merge command:
1. Merges approved PRs via GitHub/GitLab API (squash merge)
2. Fetches to sync local state with new main
3. Rebases the remaining stack onto the updated main
4. Re-submits to update PR base branches
```

### `src/merge/AGENTS.md`

```markdown
# merge/

## OVERVIEW

Merge engine for stacked PRs. Three-phase pattern: gather → plan → execute.

## FILES

| File | Purpose |
|------|---------|
| `plan.rs` | `MergePlan`, `MergeStep`, `create_merge_plan()` - PURE |
| `execute.rs` | `execute_merge()` - EFFECTFUL |
| `mod.rs` | Re-exports |

## WHERE TO LOOK

| Task | Location |
|------|----------|
| Add merge option | `MergePlanOptions` in `plan.rs` |
| Change merge behavior | `create_merge_plan()` in `plan.rs` |
| Change merge execution | `execute_merge()` in `execute.rs` |
| CLI orchestration | `src/cli/merge.rs` |
```

### AGENTS.md Addition

Add to WHERE TO LOOK table:

```markdown
| Merge PRs | `src/merge/`, `src/cli/merge.rs` | Three-phase: gather/plan/execute |
| CLI setup | `src/cli/context.rs` | Shared `CommandContext` for submit/sync/merge |
```

---

## Resources for Implementation

When implementing, include these files in context:

1. **Architecture reference**: `AGENTS.md`, `src/submit/AGENTS.md`
2. **Type patterns**: `src/types.rs`
3. **Platform trait**: `src/platform/mod.rs`, `src/platform/github.rs`
4. **CLI patterns**: `src/main.rs`, `src/cli/submit.rs`, `src/cli/sync.rs`
5. **Submit module pattern**: `src/submit/mod.rs`, `src/submit/plan.rs`, `src/submit/execute.rs`
6. **Stack analysis**: `src/submit/analysis.rs` (reuse `analyze_submission()`)
7. **PR cache**: `src/tracking/pr_cache.rs`
8. **Test patterns**: `tests/common/mock_platform.rs`, `tests/integration_tests.rs`
9. **Error patterns**: `src/error.rs`

---

## Summary

| Phase | Files Modified | Status |
|-------|----------------|--------|
| 0. Command Context | `cli/context.rs` (new), `cli/submit.rs`, `cli/sync.rs`, `cli/mod.rs` | ✅ |
| 1. Types | `types.rs` | ✅ |
| 2. Platform Trait | `platform/mod.rs` | 🔴 |
| 3. GitHub Impl | `platform/github.rs` | 🔴 |
| 4. GitLab Impl | `platform/gitlab.rs` | 🔴 |
| 5. CLI Command | `main.rs`, `cli/mod.rs` | 🔴 |
| 6. Merge Module | `merge/mod.rs`, `merge/plan.rs`, `merge/execute.rs` (new) | 🔴 |
| 6b. CLI Orchestrator | `cli/merge.rs` (new) | 🔴 |
| 6c. Rebase Helper | `repo/workspace.rs` | 🔴 |
| 7. Errors | `error.rs` | 🔴 |
| 8. Tests | `mock_platform.rs`, `integration_tests.rs`, `unit_tests.rs` | 🔴 |
| 9. Docs | `README.md`, `AGENTS.md`, `merge/AGENTS.md` (new) | 🔴 |

**Total new files**: 5
- `src/cli/context.rs`
- `src/cli/merge.rs`
- `src/merge/mod.rs`
- `src/merge/plan.rs`
- `src/merge/execute.rs`

**Total modified files**: ~13
**Estimated PR size**: ~1400-1800 lines (includes merge module + CommandContext refactor)

**Key architectural decisions:**
- **Phase 0**: Extract `CommandContext` (without graph - becomes stale)
- **Phase 6**: New `src/merge/` module following `src/submit/` pattern
- **Pure planning**: `create_merge_plan()` is testable in unit tests
- **Batch fetching**: All PR readiness fetched upfront for pure planning
- **Reuse**: Leverage existing `execute_submission()` for post-merge PR updates
- **Soft failures**: Re-submit failure doesn't fail the merge command
- **`--confirm` flag**: Consistent with submit/sync commands