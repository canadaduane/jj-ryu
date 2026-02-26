# Plan: MergeConfidence for Uncertain Merge Status

## Background

GitHub's PR API has a `mergeable` field that indicates whether a PR can be merged without conflicts. However, this field is computed **lazily** - GitHub doesn't compute it until requested, and even then it may return `null` while computation is in progress (typically 1-5 seconds).

The current `ryu merge` implementation treats `mergeable: None` as a blocking condition, displaying "Merge status unknown (still computing)" and refusing to attempt the merge. This creates a poor UX where users must wait or retry.

## Problem Statement

When GitHub hasn't computed the mergeable status yet, `ryu merge` incorrectly blocks the merge operation even though:
1. The PR may be perfectly mergeable
2. GitHub's merge API will return a clear error if conflicts exist
3. The `gh pr merge` CLI doesn't pre-check mergeable status at all

## Success Criteria

1. PRs with unknown mergeable status can proceed to merge attempt
2. Users see clear indication when merge status is uncertain vs. definitively blocked
3. The three-phase model (gather/plan/execute) maintains its functional core / imperative shell integrity
4. Dry-run output clearly distinguishes between confident merges and uncertain attempts
5. Existing tests continue to pass; new tests cover uncertainty scenarios
6. No duplicate code paths for handling merge vs. try-merge

## The Gap

### Current State

```rust
pub enum MergeStep {
    Merge { bookmark, pr_number, pr_title, method },
    Skip { bookmark, pr_number, reasons },
}

impl MergeReadiness {
    pub const fn can_merge(&self) -> bool {
        self.is_approved && self.ci_passed && self.is_mergeable && !self.is_draft
    }
}
```

- `is_mergeable: bool` - loses the distinction between `Some(false)` and `None`
- `can_merge()` returns false for both conflicts and unknown status
- No way to express uncertainty in the plan

### Desired State

```rust
pub enum MergeConfidence {
    Certain,
    Uncertain(String),  // reason for uncertainty
}

pub enum MergeStep {
    Merge { bookmark, pr_number, pr_title, method, confidence },
    Skip { bookmark, pr_number, reasons },
}

impl MergeReadiness {
    pub fn is_blocked(&self) -> bool { /* definitive blockers only */ }
    pub fn uncertainty(&self) -> Option<&str> { /* unknown states */ }
}
```

## Learnings

### L1: Avoid Duplicate Enum Variants

Adding a separate `TryMerge` variant would create duplicate fields and require duplicate match arms in `execute_merge()`. Instead, use a `confidence` field within `Merge` to distinguish certain from uncertain merges. This keeps execution logic unified.

### L2: Definitive Blockers Take Precedence

If a PR is blocked (not approved, CI failing, draft, confirmed conflicts) AND has unknown mergeable status, it should `Skip`. The unknown status doesn't matter when there's a definitive blocker.

### L3: Single Uncertainty Field Suffices

Store `uncertainties: Vec<String>` internally for future-proofing (additional uncertainty types may emerge), but expose `uncertainty() -> Option<&str>` returning only the first. This keeps the API simple while allowing future extension without breaking changes.

### L4: GitLab Always Returns Definitive Status

GitLab's `merge_status` field is computed synchronously—it always returns a definitive value like `"can_be_merged"` or `"cannot_be_merged"`. Therefore, `details.mergeable` will never be `None` for GitLab. The uncertainty handling is GitHub-specific, but the code structure supports both platforms uniformly.

### L5: `PrInfo` Embeds Both Raw and Processed Data Intentionally

`PrInfo` contains both `details: PullRequestDetails` (raw API data, `mergeable: Option<bool>`) and `readiness: MergeReadiness` (processed data with blocking reasons). This is not duplication—they serve different purposes. However, test helpers must set `details.mergeable` and `readiness.is_mergeable` consistently.

### L6: `can_merge()` Is Currently `const fn`

The current `can_merge()` is `const fn`. The replacement `is_blocked()` can remain `const fn` (simple boolean logic), but `uncertainty()` cannot be `const fn` because it returns `Option<&str>` from a `Vec<String>`.

### L7: Update Tests Alongside Code Changes

Each phase that changes types must also update the tests that depend on those types. This maintains build integrity throughout implementation. New uncertainty-specific tests are added in a dedicated phase after all type changes are complete.

### L8: Use `matches!()` for `const fn` Option Comparisons

In a `const fn`, you cannot use `self.is_mergeable == Some(false)` — it requires `matches!(self.is_mergeable, Some(false))` instead.

### L9: Clippy Enforces Positive-First Conditionals

The `if_not_else` lint requires using positive checks first:
```rust
// Rejected: if !is_blocked() { merge } else { skip }
// Accepted: if is_blocked() { skip } else { merge }
```

### L10: Phase Boundaries Are Fluid When Code Must Compile

Phase 1 expanded to include platform implementation updates (originally Phase 2) because removing `can_merge()` required updating all call sites. Similarly, updating `plan.rs` to use `is_blocked()` was necessary even though planning logic changes were scheduled for Phase 3.

## Transitive Effect Analysis

| Change | Directly Affects | Transitively Affects |
|--------|------------------|----------------------|
| `MergeReadiness.is_mergeable` → `Option<bool>` | `src/types.rs` | `src/platform/github.rs`, `src/platform/gitlab.rs`, `src/merge/plan.rs`, `tests/unit_tests.rs`, `tests/common/mock_platform.rs` |
| New `MergeConfidence` enum | `src/merge/plan.rs` | `src/merge/execute.rs` (minimal), `src/cli/merge.rs` (display) |
| `MergeStep::Merge` gains `confidence` field | `src/merge/plan.rs` | `src/merge/execute.rs`, `src/cli/merge.rs`, `tests/unit_tests.rs` |
| `MergePlan.has_mergeable` → `has_actionable` | `src/merge/plan.rs` | `src/cli/merge.rs` |

## Phases

### Phase 1: Extend Type System ✅

**Tasks:**

- ✅ Change `MergeReadiness.is_mergeable` from `bool` to `Option<bool>` in `src/types.rs`
- ✅ Add `uncertainties: Vec<String>` field to `MergeReadiness`
- ✅ Replace `can_merge()` with `is_blocked()` (can be `const fn`) and `uncertainty()` (not `const fn`) methods
- ✅ Update `tests/unit_tests.rs` helpers: `make_mergeable_pr_info()`, `make_blocked_pr_info()` for new `MergeReadiness` structure
- ✅ Update `tests/common/mock_platform.rs` helpers: `setup_mergeable_pr()`, `setup_blocked_pr()` for new structure

**Type Definitions:**

```rust
// src/types.rs
pub struct MergeReadiness {
    pub is_approved: bool,
    pub ci_passed: bool,
    pub is_mergeable: Option<bool>,  // None = unknown, Some(false) = conflicts
    pub is_draft: bool,
    pub blocking_reasons: Vec<String>,
    pub uncertainties: Vec<String>,
}

impl MergeReadiness {
    /// Definitely cannot merge - known blockers
    pub const fn is_blocked(&self) -> bool {
        !self.is_approved 
            || !self.ci_passed 
            || self.is_draft 
            || matches!(self.is_mergeable, Some(false))
    }
    
    /// Returns the first uncertainty reason, if any
    pub fn uncertainty(&self) -> Option<&str> {
        self.uncertainties.first().map(String::as_str)
    }
}
```

**Test Helper Updates (same phase):**

```rust
// tests/unit_tests.rs - make_mergeable_pr_info()
readiness: MergeReadiness {
    is_approved: true,
    ci_passed: true,
    is_mergeable: Some(true),  // Changed from bool
    is_draft: false,
    blocking_reasons: vec![],
    uncertainties: vec![],     // New field
}

// tests/unit_tests.rs - make_blocked_pr_info()
readiness: MergeReadiness {
    is_approved: false,
    ci_passed: true,
    is_mergeable: Some(true),  // Changed from bool
    is_draft: false,
    blocking_reasons: reasons,
    uncertainties: vec![],     // New field
}

// tests/common/mock_platform.rs - setup_mergeable_pr()
MergeReadiness {
    is_approved: true,
    ci_passed: true,
    is_mergeable: Some(true),  // Changed from bool
    is_draft: false,
    blocking_reasons: vec![],
    uncertainties: vec![],     // New field
}

// tests/common/mock_platform.rs - setup_blocked_pr()
MergeReadiness {
    is_approved: false,
    ci_passed: true,
    is_mergeable: Some(true),  // Changed from bool
    is_draft: false,
    blocking_reasons: reasons,
    uncertainties: vec![],     // New field
}
```

**Note:** `is_blocked()` uses `matches!(self.is_mergeable, Some(false))` instead of `self.is_mergeable == Some(false)` to remain `const fn` compatible.

### Phase 2: Update Platform Implementations ✅

*Completed as part of Phase 1 — platform changes were required to keep code compiling after type changes.*

**Tasks:**

- ✅ Update `GitHubService::check_merge_readiness()` to populate new fields
- ✅ Move "Merge status unknown" from `blocking_reasons` to `uncertainties`
- ✅ Update `GitLabService::check_merge_readiness()` similarly (will always have empty `uncertainties`)

**GitHub Implementation:**

```rust
// src/platform/github.rs - check_merge_readiness()

let mut blocking_reasons = Vec::new();
let mut uncertainties = Vec::new();

if details.is_draft {
    blocking_reasons.push("PR is a draft".to_string());
}
if !is_approved {
    blocking_reasons.push("Not approved".to_string());
}
if !ci_passed {
    blocking_reasons.push("CI not passing".to_string());
}
if details.mergeable == Some(false) {
    blocking_reasons.push("Has merge conflicts".to_string());
}
if details.mergeable.is_none() {
    uncertainties.push("Merge status unknown (GitHub still computing)".to_string());
}

MergeReadiness {
    is_approved,
    ci_passed,
    is_mergeable: details.mergeable,  // Pass through Option<bool>
    is_draft: details.is_draft,
    blocking_reasons,
    uncertainties,
}
```

**GitLab Note:** GitLab always computes `merge_status` synchronously, so `details.mergeable` will always be `Some(true)` or `Some(false)`. The `uncertainties` vector will always be empty for GitLab, but the code structure remains consistent.

### Phase 3: Update Merge Planning Logic 🔴

*Note: `create_merge_plan()` already updated to use `is_blocked()` in Phase 1. This phase adds `MergeConfidence` and updates `MergeStep`.*

**Tasks:**

- 🔴 Add `MergeConfidence` enum to `src/merge/plan.rs`
- 🔴 Add `confidence: MergeConfidence` field to `MergeStep::Merge`
- ✅ ~~Update `create_merge_plan()` to use `is_blocked()` and `uncertainty()`~~ (done in Phase 1)
- 🔴 Set `confidence` field based on uncertainty presence in `create_merge_plan()`
- 🔴 Rename `MergePlan.has_mergeable` to `has_actionable`
- 🔴 Update `merge_count()` (no change needed - still counts `Merge` variants)
- 🔴 Update `is_empty()` (no change needed - still checks for `Merge` variants)
- 🔴 Update existing `merge_plan_test` tests to match on new `MergeStep::Merge` structure with `confidence` field

**New Types:**

```rust
// src/merge/plan.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeConfidence {
    /// All conditions verified - merge should succeed
    Certain,
    /// Some conditions unknown - merge may fail
    Uncertain(String),
}

pub enum MergeStep {
    Merge {
        bookmark: String,
        pr_number: u64,
        pr_title: String,
        method: MergeMethod,
        confidence: MergeConfidence,  // New field
    },
    Skip {
        bookmark: String,
        pr_number: u64,
        reasons: Vec<String>,
    },
}
```

**Planning Logic:**

```rust
// src/merge/plan.rs - create_merge_plan()

if info.readiness.is_blocked() {
    steps.push(MergeStep::Skip {
        bookmark: bookmark_name.clone(),
        pr_number: info.details.number,
        reasons: info.readiness.blocking_reasons.clone(),
    });
    hit_blocker = true;
} else {
    let confidence = match info.readiness.uncertainty() {
        Some(reason) => MergeConfidence::Uncertain(reason.to_string()),
        None => MergeConfidence::Certain,
    };
    steps.push(MergeStep::Merge {
        bookmark: bookmark_name.clone(),
        pr_number: info.details.number,
        pr_title: info.details.title.clone(),
        method: MergeMethod::Squash,
        confidence,
    });
    bookmarks_to_clear.push(bookmark_name.clone());
}
```

**Test Updates (same phase):**

```rust
// tests/unit_tests.rs - update match arms in existing tests

// Before:
MergeStep::Merge { bookmark, pr_number, pr_title, method } => { ... }

// After:
MergeStep::Merge { bookmark, pr_number, pr_title, method, confidence } => {
    // Existing assertions...
    assert!(matches!(confidence, MergeConfidence::Certain));
}
```

### Phase 4: Update Execution and Display 🔴

**Tasks:**

- 🔴 Update `execute_merge()` match arm to destructure `confidence` (ignore it for execution - merge attempt is the same)
- 🔴 Add `was_uncertain: bool` field to `MergeExecutionResult`
- 🔴 Set `was_uncertain` when merge fails and confidence was `Uncertain`
- 🔴 Update `report_merge_dry_run()` to display confidence level using correct styling methods
- 🔴 Update `src/cli/merge.rs` to use `plan.has_actionable` instead of `plan.has_mergeable`
- 🔴 Add contextual error messaging when uncertain merge fails

**Display Format:**

```
Merge plan:

  ✓ Would merge PR #101: Add auth
    Bookmark: feat-a

  ? Would attempt PR #102: Add sessions
    Bookmark: feat-b
    ⚠ Merge status unknown (GitHub still computing)

  ✗ Would skip PR #103: Add logout
    - Not approved
```

**Display Implementation (corrected styling):**

```rust
// src/cli/merge.rs - report_merge_dry_run()

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
```

**Contextual Error Messaging:**

When an uncertain merge fails, the error message should acknowledge the uncertainty:

```rust
// src/merge/execute.rs - execute_merge()

MergeStep::Merge {
    bookmark,
    pr_number,
    pr_title,
    method,
    confidence,
} => {
    // ... merge attempt ...
    Err(e) => {
        result.failed_bookmark = Some(bookmark.clone());
        result.error_message = Some(e.to_string());
        result.was_uncertain = matches!(confidence, MergeConfidence::Uncertain(_));
        break;
    }
}
```

Then in CLI error display:
```rust
if result.was_uncertain {
    eprintln!("{}", "Merge failed (merge status was uncertain)".warn());
}
eprintln!("  {}: {}", "Error".error(), result.error_message.unwrap_or_default());
```

**Note:** This requires adding `was_uncertain: bool` field to `MergeExecutionResult`.

### Phase 5: Add New Uncertainty Tests 🔴

**Tasks:**

- 🔴 Add `make_uncertain_pr_info()` helper to `tests/unit_tests.rs`
- 🔴 Add `setup_uncertain_pr()` helper to `tests/common/mock_platform.rs`
- 🔴 Add test: `test_create_merge_plan_uncertain_mergeable_has_uncertain_confidence`
- 🔴 Add test: `test_blocked_with_unknown_mergeable_still_skips`
- 🔴 Add test: `test_merge_readiness_is_blocked`
- 🔴 Add test: `test_merge_readiness_uncertainty`

**Test Helper Consistency Guidance:**

When creating `PrInfo` in tests, ensure `details.mergeable` and `readiness.is_mergeable` are set to the same value:

```rust
fn make_uncertain_pr_info(bookmark: &str, pr_number: u64, title: &str) -> PrInfo {
    PrInfo {
        bookmark: bookmark.to_string(),
        details: PullRequestDetails {
            number: pr_number,
            title: title.to_string(),
            body: Some(format!("PR body for {bookmark}")),
            state: PrState::Open,
            is_draft: false,
            mergeable: None,  // ← Unknown!
            head_ref: bookmark.to_string(),
            base_ref: "main".to_string(),
            html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
        },
        readiness: MergeReadiness {
            is_approved: true,
            ci_passed: true,
            is_mergeable: None,  // ← Must match details.mergeable!
            is_draft: false,
            blocking_reasons: vec![],
            uncertainties: vec!["Merge status unknown (GitHub still computing)".to_string()],
        },
    }
}
```

**New Tests:**

```rust
#[test]
fn test_create_merge_plan_uncertain_mergeable_has_uncertain_confidence() {
    // PR with is_mergeable: None should produce Merge with Uncertain confidence
    let graph = make_linear_stack(&["feat-a"]);
    let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();
    let mut pr_info = HashMap::new();
    pr_info.insert("feat-a".to_string(), make_uncertain_pr_info("feat-a", 1, "Feature A"));
    
    let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default());
    
    assert!(!plan.is_empty());
    match &plan.steps[0] {
        MergeStep::Merge { confidence, .. } => {
            assert!(matches!(confidence, MergeConfidence::Uncertain(_)));
        }
        _ => panic!("Expected Merge step"),
    }
}

#[test]
fn test_blocked_with_unknown_mergeable_still_skips() {
    // If not approved AND mergeable unknown, should Skip (blocker takes precedence)
    let graph = make_linear_stack(&["feat-a"]);
    let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();
    
    let mut pr_info = HashMap::new();
    let mut info = make_uncertain_pr_info("feat-a", 1, "Feature A");
    info.readiness.is_approved = false;
    info.readiness.blocking_reasons = vec!["Not approved".to_string()];
    pr_info.insert("feat-a".to_string(), info);
    
    let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default());
    
    assert!(plan.is_empty()); // No Merge steps
    assert!(matches!(&plan.steps[0], MergeStep::Skip { reasons, .. } 
        if reasons.contains(&"Not approved".to_string())));
}

#[test]
fn test_merge_readiness_is_blocked() {
    // Unit tests for is_blocked() with various combinations
    let base = MergeReadiness {
        is_approved: true,
        ci_passed: true,
        is_mergeable: Some(true),
        is_draft: false,
        blocking_reasons: vec![],
        uncertainties: vec![],
    };
    assert!(!base.is_blocked());
    
    // Not approved blocks
    let mut r = base.clone();
    r.is_approved = false;
    assert!(r.is_blocked());
    
    // CI failing blocks
    let mut r = base.clone();
    r.ci_passed = false;
    assert!(r.is_blocked());
    
    // Conflicts block
    let mut r = base.clone();
    r.is_mergeable = Some(false);
    assert!(r.is_blocked());
    
    // Unknown does NOT block
    let mut r = base.clone();
    r.is_mergeable = None;
    assert!(!r.is_blocked());
    
    // Draft blocks
    let mut r = base.clone();
    r.is_draft = true;
    assert!(r.is_blocked());
}

#[test]
fn test_merge_readiness_uncertainty() {
    // Unit tests for uncertainty() method
    let mut r = MergeReadiness {
        is_approved: true,
        ci_passed: true,
        is_mergeable: None,
        is_draft: false,
        blocking_reasons: vec![],
        uncertainties: vec![],
    };
    assert!(r.uncertainty().is_none());
    
    r.uncertainties = vec!["Reason 1".to_string()];
    assert_eq!(r.uncertainty(), Some("Reason 1"));
    
    r.uncertainties = vec!["Reason 1".to_string(), "Reason 2".to_string()];
    assert_eq!(r.uncertainty(), Some("Reason 1")); // Returns first only
}
```

### Phase 6: Documentation 🔴

**Tasks:**

- 🔴 Create `src/merge/AGENTS.md` with MergeConfidence documentation
- 🔴 Add note to `.plans/002-merge.md` about this enhancement

## Tests Summary

| Test | Purpose |
|------|---------|
| `test_create_merge_plan_uncertain_mergeable_has_uncertain_confidence` | Verify unknown mergeable creates `Uncertain` confidence |
| `test_blocked_with_unknown_mergeable_still_skips` | Verify definitive blockers take precedence over uncertainty |
| `test_merge_readiness_is_blocked` | Unit test `is_blocked()` logic with all field combinations |
| `test_merge_readiness_uncertainty` | Unit test `uncertainty()` method with 0, 1, and 2+ uncertainties |
| Update existing `merge_plan_test` tests | Ensure backward compatibility with new field types |

## Resources for Implementation

| Resource | Purpose |
|----------|---------|
| `src/types.rs` L210-240 | `MergeReadiness` struct |
| `src/merge/plan.rs` | `MergeStep` enum, `create_merge_plan()` |
| `src/merge/execute.rs` | `execute_merge()` - add `was_uncertain` to result |
| `src/cli/merge.rs` L340-390 | `report_merge_dry_run()` display |
| `src/cli/style.rs` | Styling methods: `.success()`, `.warn()`, `.accent()`, `.muted()` |
| `src/platform/github.rs` L480-530 | `check_merge_readiness()` |
| `src/platform/gitlab.rs` L450-480 | Similar changes (will always have empty uncertainties) |
| `tests/unit_tests.rs` L805+ | `merge_plan_test` module |
| `tests/common/mock_platform.rs` L175-275 | `setup_mergeable_pr()`, `setup_blocked_pr()` |

## Summary

This plan introduces `MergeConfidence` to distinguish between certain and uncertain merge attempts, while keeping execution logic unified. The key design decisions:

1. **Single `Merge` variant with `confidence` field** - avoids duplicate code paths
2. **`is_blocked()` for definitive blockers** - not approved, CI failing, draft, confirmed conflicts
3. **`uncertainty()` for unknown states** - mergeable status not yet computed
4. **Blockers take precedence** - if blocked AND uncertain, result is Skip
5. **Rename `has_mergeable` → `has_actionable`** - reflects that uncertain merges are actionable
6. **GitLab unaffected** - always returns definitive merge status, uncertainties will be empty
7. **Contextual error messaging** - uncertain merge failures acknowledge the uncertainty
8. **Test helper consistency** - `details.mergeable` and `readiness.is_mergeable` must match
9. **Tests updated per phase** - existing test updates accompany code changes to maintain build integrity

This maintains the functional core / imperative shell pattern while providing clear user feedback about merge confidence levels.