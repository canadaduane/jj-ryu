# Plan: Retarget PRs During Stack Merge

**Created:** 2026-02-26
**Status:** ✅ Complete

## Background

When merging a stack of PRs, each PR has a base branch pointing to its parent:
- PR1 base: `main`
- PR2 base: `feat-a` (PR1's branch)
- PR3 base: `feat-b` (PR2's branch)

GitHub's merge API merges a PR into **its current base branch**, not into trunk. When PR1 merges into `main`, PR2's base is still `feat-a`—so merging PR2 merges it into the now-defunct `feat-a` branch, not `main`.

This was discovered when attempting to merge a 5-PR stack: all 5 PRs closed successfully, but only PR1's changes made it to `main`. Each subsequent PR merged into its parent's branch rather than trunk.

## Problem Statement

The `ryu merge` command merges stacked PRs sequentially but does not retarget subsequent PRs to trunk after each merge. This causes:

1. PR N+1 merges into PR N's branch (now merged), not into trunk
2. All PRs appear "merged" in GitHub (closed with merge commit)
3. Only the first PR's changes reach the trunk branch
4. User loses work unless they notice and manually recover

## Success Criteria

1. After each successful merge, the next PR in the stack is retargeted to trunk before merging
2. Dry-run output shows both merge and retarget steps
3. If retarget fails, execution stops with clear error (no partial state)
4. Existing tests continue to pass
5. New tests verify retarget step generation and execution

## The Gap

### Current State

`MergeStep` only has `Merge` and `Skip` variants:

```rust
pub enum MergeStep {
    Merge { bookmark, pr_number, pr_title, method, confidence },
    Skip { bookmark, pr_number, reasons },
}
```

`MergePlan` does not carry the trunk branch name:

```rust
pub struct MergePlan {
    pub steps: Vec<MergeStep>,
    pub bookmarks_to_clear: Vec<String>,
    pub rebase_target: Option<String>,
    pub has_actionable: bool,
}
```

`execute_merge()` iterates steps and calls `platform.merge_pr()` without any retargeting.

### Desired State

`MergeStep` gains a `RetargetBase` variant:

```rust
pub enum MergeStep {
    Merge { bookmark, pr_number, pr_title, method, confidence },
    RetargetBase { bookmark, pr_number, old_base, new_base },
    Skip { bookmark, pr_number, reasons },
}
```

`MergePlan` carries the trunk branch:

```rust
pub struct MergePlan {
    pub steps: Vec<MergeStep>,
    pub bookmarks_to_clear: Vec<String>,
    pub rebase_target: Option<String>,
    pub has_actionable: bool,
    pub trunk_branch: String,  // NEW
}
```

`create_merge_plan()` generates interleaved `Merge` → `RetargetBase` steps:

```
Merge { feat-a, pr: 1 }
RetargetBase { feat-b, pr: 2, old_base: "feat-a", new_base: "main" }
Merge { feat-b, pr: 2 }
RetargetBase { feat-c, pr: 3, old_base: "feat-b", new_base: "main" }
Merge { feat-c, pr: 3 }
```

`execute_merge()` handles `RetargetBase` by calling `platform.update_pr_base()`.

## Learnings

### L1: `update_pr_base` Already Exists

`PlatformService` already has `update_pr_base(pr_number, new_base)` used by the submit flow. No new platform methods needed.

### L2: `PrInfo.details.base_ref` Contains Current Base

The `PullRequestDetails` fetched during gather phase includes `base_ref`, so we have the `old_base` value for display without additional API calls.

### L3: Plan Must Be Pure

`create_merge_plan()` is marked `#[must_use]` and documented as pure. The new `trunk_branch` parameter must be passed in, not fetched.

### L4: Retarget Failure Should Be Fatal

Unlike stack comment failures (soft errors), a retarget failure leaves the system in an inconsistent state. The next merge would fail anyway, so we should stop immediately.

### L5: First PR Always Targets Trunk (By Definition)

In a properly-formed stack, the first PR's base is already trunk. The retarget logic only applies to PRs 2..N. If PR1 doesn't target trunk, that's a misconfigured stack—the user should fix it before merging.

### L6: Pattern Consistency Over Code Sharing

`submit` and `merge` have different execution semantics (soft errors, partial success, constraint ordering). Sharing a generic step executor framework would add complexity without meaningful benefit. Instead, maintain **pattern consistency**:
- Both step enums should have `Display` impl
- Both step enums should have `bookmark_name()` accessor
- Document why modules are intentionally separate

### L7: The Platform API Is the True Shared Layer

Both modules call `platform.update_pr_base()`. The platform trait is the correct abstraction boundary. Execution wrappers are thin (~10 lines) and domain-specific—duplicating them is acceptable.

## Transitive Effect Analysis

| Changed | Direct Dependents | Transitive Effects |
|---------|-------------------|-------------------|
| `MergeStep` enum | `create_merge_plan()`, `execute_merge()`, dry-run display | Tests constructing `MergePlan` manually |
| `MergePlan` struct | `create_merge_plan()`, `execute_merge()`, CLI merge | Tests constructing `MergePlan` manually |
| `create_merge_plan()` signature | `cli/merge.rs` | None (CLI is the only caller) |
| `execute_merge()` | CLI merge | Tests calling `execute_merge()` |
| Dry-run display | None | User-visible output changes |

**Risk**: Tests in `unit_tests.rs` manually construct `MergePlan`. These will fail to compile after adding `trunk_branch` field.

## Phases

### Phase 1: Extend Type System ✅

**Goal**: Add `RetargetBase` variant and `trunk_branch` field.

#### Tasks

1. ✅ Add `RetargetBase` variant to `MergeStep` in `src/merge/plan.rs`:
   ```rust
   /// Retarget this PR's base branch before merging
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
   ```

2. ✅ Add `trunk_branch: String` field to `MergePlan` struct

3. ✅ Update `create_merge_plan()` signature to accept `trunk_branch: &str` parameter

4. ✅ Fix compilation errors in `cli/merge.rs` (pass `default_branch` to planner)

5. ✅ Fix compilation errors in unit tests (add `trunk_branch` to manual `MergePlan` construction)

6. ✅ Add `bookmark_name(&self) -> &str` method to `MergeStep` (pattern consistency with `ExecutionStep`)

7. ✅ Add `std::fmt::Display` impl for `MergeStep` (pattern consistency with `ExecutionStep`)

### Phase 2: Generate Retarget Steps in Planning ✅

**Goal**: `create_merge_plan()` produces interleaved Merge/RetargetBase steps.

#### Tasks

1. ✅ Refactor loop to use indexed iteration (need `idx` for lookahead)

2. ✅ After each `MergeStep::Merge` (except the last), check if there's a next mergeable PR

3. ✅ If next PR exists, insert `MergeStep::RetargetBase` with:
   - `bookmark`: next PR's bookmark
   - `pr_number`: next PR's number
   - `old_base`: next PR's current `details.base_ref`
   - `new_base`: `trunk_branch` parameter

4. ✅ Ensure `Skip` steps do NOT generate retarget steps (we stop at first skip anyway)

5. ✅ Only generate retarget if `old_base != new_base` (skip redundant retargets)

**Key Logic** (in `create_merge_plan()`):
```rust
// After adding a Merge step, check for next PR
if !hit_blocker {
    if let Some(next_segment) = analysis.segments.get(idx + 1) {
        if let Some(next_info) = pr_info.get(&next_segment.bookmark.name) {
            steps.push(MergeStep::RetargetBase {
                bookmark: next_segment.bookmark.name.clone(),
                pr_number: next_info.details.number,
                old_base: next_info.details.base_ref.clone(),
                new_base: trunk_branch.to_string(),
            });
        }
    }
}
```

### Phase 3: Execute Retarget Steps ✅

**Goal**: `execute_merge()` handles `RetargetBase` variant.

#### Tasks

1. ✅ Add match arm for `MergeStep::RetargetBase` in `execute_merge()`:
   ```rust
   MergeStep::RetargetBase { bookmark, pr_number, old_base, new_base } => {
       progress.on_message(&format!(
           "↪️ Retargeting PR #{pr_number} ({bookmark}): {old_base} → {new_base}"
       )).await;
       
       match platform.update_pr_base(*pr_number, new_base).await {
           Ok(_) => {
               progress.on_message(&format!("✅ Retargeted to {new_base}")).await;
           }
           Err(e) => {
               result.failed_bookmark = Some(bookmark.clone());
               result.error_message = Some(format!("Retarget failed: {e}"));
               result.was_uncertain = false;
               break;
           }
       }
   }
   ```

2. ✅ Retarget failures are fatal—set `failed_bookmark` and break

3. ✅ Do NOT add retargeted bookmarks to `merged_bookmarks` (they're not merged yet)

### Phase 4: Update Dry-Run Display ✅

**Goal**: Dry-run output shows retarget steps.

#### Tasks

1. ✅ Add match arm in `report_merge_dry_run()` for `RetargetBase`:
   ```rust
   MergeStep::RetargetBase { bookmark, pr_number, old_base, new_base } => {
       println!(
           "  {} PR #{} ({}): {} → {}",
           "↪ Would retarget".accent(),
           pr_number,
           bookmark,
           old_base.muted(),
           new_base.accent()
       );
   }
   ```

### Phase 5: Add Tests ✅

**Goal**: Verify retarget step generation and execution.

#### Tasks

1. ✅ **Unit test**: `test_create_merge_plan_generates_retarget_steps`
   - 3-PR stack, all mergeable
   - Verify plan has: Merge, Retarget, Merge, Retarget, Merge (5 steps)
   - Verify retarget `new_base` is trunk branch

2. ✅ **Unit test**: `test_create_merge_plan_no_retarget_after_skip`
   - 3-PR stack, PR2 blocked
   - Verify plan has: Merge (PR1), Skip (PR2), no retarget steps
   - Because we stop at skip, no retarget needed

3. ✅ **Unit test**: `test_create_merge_plan_single_pr_no_retarget`
   - 1-PR stack
   - Verify plan has: Merge only, no retarget
   - Edge case: nothing to retarget after last merge

4. ✅ **Unit test**: `test_create_merge_plan_skips_redundant_retarget`
   - 2-PR stack, PR2 already targets main
   - Verify no retarget step generated

5. ✅ **Execution test**: `test_execute_merge_calls_retarget`
   - Mock platform with 2 PRs
   - Execute plan with Merge + Retarget + Merge
   - Assert `update_pr_base` was called with correct args
   - Assert both PRs merged successfully

6. ✅ **Execution test**: `test_execute_merge_stops_on_retarget_failure`
   - Mock platform with retarget failure injected
   - Verify execution stops, `failed_bookmark` is set
   - Verify first merge succeeded but second didn't execute

### Phase 6: Update Documentation ✅

**Goal**: Document the retarget behavior and architectural decisions.

#### Tasks

1. ✅ Update `src/merge/AGENTS.md`:
   - Add `RetargetBase` to core types section
   - Document step interleaving pattern
   - Add "retarget failure is fatal" to anti-patterns

2. ✅ Add "Why Not Share Code with submit?" section to `src/merge/AGENTS.md`:
   - Explain different execution semantics (soft errors, partial success, constraint ordering)
   - Clarify that platform API is the shared layer
   - Prevent future "unification" attempts

3. ⛔ Update `docs/merge.md` (if exists) with user-facing explanation — file does not exist

## Tests Summary

| Test | Type | Purpose |
|------|------|---------|
| `test_create_merge_plan_generates_retarget_steps` | Unit | Verify interleaved step generation |
| `test_create_merge_plan_no_retarget_after_skip` | Unit | Verify skip stops retarget generation |
| `test_create_merge_plan_single_pr_no_retarget` | Unit | Edge case: single PR |
| `test_create_merge_plan_skips_redundant_retarget` | Unit | Edge case: base already trunk |
| `test_execute_merge_calls_retarget` | Async | Verify platform API called correctly |
| `test_execute_merge_stops_on_retarget_failure` | Async | Verify failure handling |

## Resources for Implementation

### Files to Modify

- `src/merge/plan.rs` — `MergeStep`, `MergePlan`, `create_merge_plan()`
- `src/merge/execute.rs` — `execute_merge()` match arm
- `src/merge/mod.rs` — re-export if needed
- `src/cli/merge.rs` — pass `default_branch` to planner, update dry-run display
- `tests/unit_tests.rs` — add new tests, fix existing `MergePlan` constructions

### Files for Reference

- `src/platform/mod.rs` — `update_pr_base()` signature
- `tests/common/mock_platform.rs` — `UpdateBaseCall` struct, mock behavior
- `src/submit/plan.rs` — pattern for interleaved steps (Push/Retarget)

### Existing Test Helpers

- `make_mergeable_pr_info()` — creates `PrInfo` for mergeable PR
- `make_blocked_pr_info()` — creates `PrInfo` for blocked PR
- `make_linear_stack()` — creates `ChangeGraph` with linear stack
- `MockPlatformService::assert_update_base_called()` — verify retarget calls

### Pattern References

- `src/submit/plan.rs` — `ExecutionStep::bookmark_name()` implementation
- `src/submit/plan.rs` — `impl Display for ExecutionStep`

## Summary

This plan adds a `RetargetBase` step to the merge plan that retargets each subsequent PR to trunk after each successful merge. The fix follows the existing FC/IS pattern:

1. **Planning (pure)**: Generate interleaved Merge/RetargetBase steps
2. **Execution (effectful)**: Execute steps in order, calling `update_pr_base()` for retargets

The key insight is that GitHub's merge API merges into the PR's *current* base, so we must update that base to trunk before each merge (except the first, which already targets trunk).

**Architectural decision**: We maintain pattern consistency with `submit` (Display, bookmark_name) but intentionally keep execution frameworks separate due to different domain semantics.

## PR Stack

### PR 1: `refactor(merge): add Display and bookmark_name to MergeStep` ✅

**Type**: Mechanical refactor / prep

**Scope**: Pattern consistency with `ExecutionStep`, no behavior change

**Phases**: 1.6, 1.7 (partial)

**Changes**:
- Add `bookmark_name(&self) -> &str` method to `MergeStep`
- Add `impl Display for MergeStep` (for existing `Merge` and `Skip` variants)
- Refactor `report_merge_dry_run()` in `cli/merge.rs` to use `Display`

**Why separate**: 
- Pure refactor, no behavior change
- Makes PR 2 smaller and more focused
- Independently useful (cleaner dry-run code)

**Tests**: None needed (no behavior change, existing tests cover usage)

---

### PR 2: `fix(merge): retarget PRs to trunk during stack merge` ✅

**Type**: Bug fix

**Scope**: Fix the 5-PR merge bug where only first PR reaches trunk

**Phases**: 1.1–1.5, 2, 3, 4, 5, 6 (all remaining work)

**Changes**:
- Add `RetargetBase` variant to `MergeStep` (extend `Display` impl)
- Add `trunk_branch` field to `MergePlan`
- Update `create_merge_plan()` signature and logic to generate interleaved steps
- Update `execute_merge()` to handle `RetargetBase`
- Update dry-run display for `RetargetBase`
- Fix existing tests (add `trunk_branch` field to manual `MergePlan` constructions)
- Add 5 new tests from Phase 5
- Update `src/merge/AGENTS.md` with retarget docs and "Why Not Share" section

**Why together**:
- Feature is atomic—can't ship partial retarget
- Tests and implementation belong together for bug fixes
- Documentation explains the feature being added

**Tests**: All 5 new tests from Phase 5

---

### Why Not More PRs?

| Considered Split | Rejected Because |
|------------------|------------------|
| Types separate from logic | Types alone don't compile (signature change requires logic) |
| Planning separate from execution | Can't test planning without execution (no observable behavior) |
| Tests separate from implementation | For bug fixes, "test + fix" together is idiomatic |
| Docs separate from code | Docs explain the code being added—natural unit |

## Changeset

This is a **bugfix** that changes merge behavior. No public API changes—only internal step generation and execution.

**User-visible changes**:
- Dry-run output now shows "↪ Would retarget" steps between merges
- Multi-PR stack merges now work correctly (all PRs reach trunk)

---

## Amendment: Test and Code Quality Improvements

**Discovered during**: Post-implementation review (after Phase 5)
**Target phase**: New Phase 7 (cleanup)
**Status**: 🔴 Not Started

### Preamble

During trenchant review of the completed implementation, several issues were identified:

1. **Test passes for wrong reason**: `test_create_merge_plan_multiple_consecutive_mergeable` asserts "all steps should be Merge steps" and passes—but only because the test helper `make_mergeable_pr_info` sets `base_ref: "main"` for all PRs, triggering the "skip redundant retarget" optimization. The test doesn't exercise the real-world scenario where PRs have stacked base refs.

2. **Helper creates unrealistic data**: `make_mergeable_pr_info` sets `base_ref: "main"` for all PRs, which doesn't model real stacked PR scenarios. This is why we needed `make_mergeable_pr_info_with_base` for the new retarget tests.

3. **Dead code in match arm**: The second-pass loop has `MergeStep::RetargetBase { .. }` in a match arm, but `RetargetBase` can never appear in the input `steps` vector—it's only created during the second pass.

### Learnings

- **L8**: Test helpers that create "convenient" default values can mask bugs. Explicit base refs are necessary to test retarget logic.
- **L9**: When adding a new enum variant, audit existing match arms to ensure they're not handling impossible cases.

### Phase 7: Test and Code Quality Cleanup ✅

**Goal**: Strengthen test coverage and remove dead code.

#### Tasks

1. ✅ Update `test_create_merge_plan_multiple_consecutive_mergeable` to use realistic base refs:
   - PR1 targets `main`, PR2 targets `feat-a`, PR3 targets `feat-b`
   - Assert 5 steps: Merge, Retarget, Merge, Retarget, Merge
   - This ensures the test exercises the actual retarget logic

2. ✅ Add comment to `make_mergeable_pr_info` explaining it creates "flat" PRs (all targeting main):
   ```rust
   /// Helper to create a mergeable PrInfo.
   /// NOTE: Sets base_ref to "main" for all PRs. For realistic stacked PR scenarios
   /// where PRs target their parent's branch, use `make_mergeable_pr_info_with_base`.
   ```

3. ✅ Remove dead `RetargetBase` case from second-pass match arm in `create_merge_plan()`:
   ```rust
   // Change from:
   MergeStep::Skip { .. } | MergeStep::RetargetBase { .. } => { ... }
   // To:
   MergeStep::Skip { .. } => { ... }
   ```

4. ✅ Add comment explaining the two-pass algorithm in `create_merge_plan()`:
   ```rust
   // Two-pass algorithm:
   // Pass 1: Collect all Merge/Skip steps and track indices of mergeable PRs
   // Pass 2: Interleave RetargetBase steps between consecutive Merge steps
   // 
   // This is necessary because we need lookahead to know if there's a "next"
   // mergeable PR that requires retargeting. A single-pass approach would
   // require complex state management or iterator peeking.
   ```

### Tests Affected

| Test | Change |
|------|--------|
| `test_create_merge_plan_multiple_consecutive_mergeable` | Update to expect 5 steps with retargets |

### PR Stack Update

This amendment adds **PR 3** to the stack:

### PR 3: `test(merge): strengthen retarget test coverage` ✅

**Type**: Test improvement / cleanup

**Scope**: Fix test that passes for wrong reason, add documentation

**Changes**:
- Update `test_create_merge_plan_multiple_consecutive_mergeable` with realistic base refs
- Add documentation comments to test helpers
- Remove dead code in `create_merge_plan()`
- Add algorithm explanation comment

**Why separate from PR 2**:
- PR 2 is the bug fix (correctness)
- PR 3 is test quality (robustness)
- Keeps PR 2 focused on the fix itself