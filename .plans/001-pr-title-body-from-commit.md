# Plan: PR Title & Body from jj Commit Description

## Background

`ryu submit` creates PRs for bookmarked commits but currently:
- **Title**: Uses the first line of the oldest (root) commit in the segment
- **Body**: Empty (not populated)

A single PR can contain **multiple commits** (all commits in the segment between the previous bookmark and this one). Users expect the PR description to include context from all commits.

## Problem Statement

When `ryu submit` creates a PR, it discards commit bodies entirely. Users must manually edit PR descriptions on GitHub/GitLab after submission, breaking the workflow of "describe once in jj, submit everywhere."

## Success Criteria

1. New PRs are created with body populated from **all commits** in the segment
2. Title continues to use the first line of the root commit (existing behavior)
3. Body concatenates all commit bodies, separated by blank lines, in root-to-tip order
4. Existing PRs are not affected (body is only set on creation, not update)
5. Commits with empty bodies are skipped (no empty sections)

## The Gap

| Component | Current | Target |
|-----------|---------|--------|
| `LogEntry` | `description_first_line: String` | Add `description: String` (full) |
| `PlatformService::create_pr_with_options` | No body param | Add `body: Option<&str>` |
| `PrToCreate` | No body field | Add `body: Option<String>` |
| `generate_pr_title()` | Returns title only | Returns `(title, Option<body>)` |

## Transitive Effect Analysis

```
LogEntry (types.rs)
  └── commit_to_log_entry() (workspace.rs) - must populate new field
  └── BookmarkSegment.changes (types.rs) - contains LogEntry, no change needed
      └── NarrowedBookmarkSegment.changes - same
          └── generate_pr_title() (analysis.rs) - must extract body from tip
              └── create_submission_plan() (plan.rs) - must pass body to PrToCreate
                  └── execute_create_pr() (execute.rs) - must pass body to platform

PlatformService::create_pr_with_options (platform/mod.rs)
  └── GitHubService (platform/github.rs) - must pass body to octocrab
  └── GitLabService (platform/gitlab.rs) - must pass body to API
  └── MockPlatformService (tests/common/mock_platform.rs) - must accept body param
```

**Test files affected:**
- `tests/unit_tests.rs` - may have `LogEntry` construction
- `tests/integration_tests.rs` - uses `MockPlatformService`
- `tests/common/mock_platform.rs` - trait impl

---

## Phase 1: Extend Data Model ✅

### Tasks
- ✅ Add `description: String` field to `LogEntry` in `src/types.rs`
- ✅ Update `commit_to_log_entry()` in `src/repo/workspace.rs` to store full description
- ✅ Update test helpers that construct `LogEntry` (search for `LogEntry {`)

---

## Phase 2: Extend Platform Trait ✅

### Tasks
- ✅ Add `body: Option<&str>` parameter to `create_pr_with_options()` in `src/platform/mod.rs`
- ✅ Update `GitHubService::create_pr_with_options()` to pass body to octocrab
- ✅ Update `GitLabService::create_pr_with_options()` to pass body to API
- ✅ Update `MockPlatformService` in `tests/common/mock_platform.rs`

### Key Interface Change
```rust
async fn create_pr_with_options(
    &self,
    head: &str,
    base: &str,
    title: &str,
    body: Option<&str>,  // NEW
    draft: bool,
) -> Result<PullRequest>;
```

---

## Phase 3: Wire Through Submission Pipeline ✅

### Tasks
- ✅ Add `body: Option<String>` field to `PrToCreate` in `src/submit/plan.rs`
- ✅ Rename `generate_pr_title()` to `generate_pr_content()` in `src/submit/analysis.rs`
- ✅ Return `(String, Option<String>)` tuple (title, body) from `generate_pr_content()`
- ✅ Concatenate bodies from **all commits** in root-to-tip order
- ✅ Update `create_submission_plan()` to populate `PrToCreate.body`
- ✅ Update `execute_create_pr()` in `src/submit/execute.rs` to pass body

### Body Extraction Logic
```rust
// changes is newest-first, so reverse for root-to-tip order
fn extract_body(description: &str) -> Option<&str> {
    // Skip first line and blank line separator
    let body_start = description.find("\n\n").map(|i| i + 2)?;
    let body = description[body_start..].trim();
    if body.is_empty() { None } else { Some(body) }
}

let bodies: Vec<&str> = segment.changes
    .iter()
    .rev()  // root-to-tip order
    .filter_map(|c| extract_body(&c.description))
    .collect();

let body = if bodies.is_empty() {
    None
} else {
    Some(bodies.join("\n\n"))
};
```

### Example Output
For a segment with 3 commits:
```
[Body from Commit A (root)]

[Body from Commit B]

[Body from Commit C (tip/bookmarked)]
```

---

## Phase 4: Update Display Formatting ✅

### Tasks
- ✅ Update `ExecutionStep::CreatePr` Display impl to show body presence (e.g., `[+body]`)
- ✅ Update dry-run output in `format_step_for_dry_run()` if body is present (uses Display impl)

---

## Phase 5: Tests ✅

### Unit Tests
- ✅ Test `generate_pr_content()` extracts title from root commit
- ✅ Test body concatenates all commit bodies in root-to-tip order
- ✅ Test body extraction handles: no body, single-line description, multi-paragraph body
- ✅ Test body is `None` when all commits have only first-line descriptions
- ✅ Test commits with empty bodies are skipped (no empty sections in output)

### Integration Tests
- ✅ Verify `MockPlatformService` receives body in `create_pr_with_options` calls
- ✅ Test full submission flow includes body in created PR

### Test Locations
- Body extraction: `src/submit/analysis.rs` (inline tests or `tests/unit_tests.rs`)
- Integration: `tests/integration_tests.rs`

---

## Documentation Updates ✅

### Tasks
- ✅ Update README.md to document that PR body comes from commit description
- ✅ No TECHNICAL.md exists; skip

---

## Changeset

Not applicable (this project doesn't use changesets).

---

## Summary

| Phase | Files Modified |
|-------|----------------|
| 1. Data Model | `types.rs`, `workspace.rs`, test helpers |
| 2. Platform Trait | `platform/mod.rs`, `github.rs`, `gitlab.rs`, `mock_platform.rs` |
| 3. Submission Pipeline | `plan.rs`, `analysis.rs`, `execute.rs` |
| 4. Display | `plan.rs` or `execute.rs` |
| 5. Tests | `unit_tests.rs`, `integration_tests.rs` |
| 6. Docs | `README.md` |

Total files: ~10
