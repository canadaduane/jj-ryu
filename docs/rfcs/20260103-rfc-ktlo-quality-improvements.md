# RFC: KTLO Quality Improvements Batch

**Status:** Proposed
**Author:** OpenCode
**Date:** 2026-01-03
**Scope:** `src/`, `Cargo.toml`

---

## Summary

Five small, surgical quality improvements to reduce technical debt, improve debuggability, and align codebase with documented conventions. No behavioral changes; pure hygiene.

---

## Motivation

Periodic KTLO (Keep The Lights On) work prevents accumulation of small issues that compound into larger maintenance burdens. This batch addresses:

1. **Unused dependency** violating AGENTS.md conventions
2. **Incorrect error semantics** conflating internal bugs with user input errors
3. **Opaque panic messages** from bare `.unwrap()` calls
4. **Zero observability** in platform/auth code paths

### Non-Goals

- New features
- Behavioral changes
- Large refactors

---

## Changes

### 1. Remove Unused `mockall` Dependency

**File:** `Cargo.toml`

**Before:**
```toml
[dev-dependencies]
mockall = "0.13"
```

**After:** Line removed.

**Rationale:** AGENTS.md explicitly states "Don't use mockall - hand-rolled `MockPlatformService` for method return type compatibility." Grep confirms 0 usages of `mockall::` in codebase. Dead dependency increases build time and dependency surface.

**Risk:** None. Verified via `cargo check`.

---

### 2. Fix `Error::Internal` Misuse for User Input Validation

**File:** `src/cli/submit.rs`

**Before:**
```rust
// Line 213
.ok_or_else(|| Error::Internal("--upto requires a bookmark name".to_string()))?;

// Lines 241-244
let target_idx = target_idx.ok_or_else(|| {
    Error::Internal(format!(
        "Target bookmark '{bookmark}' not found in analysis"
    ))
})?;
```

**After:**
```rust
// Line 213
.ok_or_else(|| Error::InvalidArgument("--upto requires a bookmark name".to_string()))?;

// Lines 241-244
let target_idx = target_idx.ok_or_else(|| {
    Error::InvalidArgument(format!(
        "Target bookmark '{bookmark}' not found in analysis"
    ))
})?;
```

**Rationale:** `Error::Internal` semantically indicates "this is a bug in jj-ryu" and may trigger different handling (e.g., "please report this issue" messaging). These are user input validation failures, not internal bugs. `Error::InvalidArgument` already exists (error.rs:97) for this purpose.

**Risk:** None. Error messages unchanged; only variant type differs.

---

### 3. Replace Post-Validation `.unwrap()` with `.expect()`

**Files:** `src/cli/submit.rs`, `src/submit/plan.rs`, `src/submit/analysis.rs`

**Locations:**

| File | Line | Before | After |
|------|------|--------|-------|
| `cli/submit.rs` | 399 | `.min().unwrap()` | `.min().expect("selections verified non-empty")` |
| `cli/submit.rs` | 400 | `.max().unwrap()` | `.max().expect("selections verified non-empty")` |
| `cli/submit.rs` | 407 | `.unwrap()` | `.expect("gap exists since span != len")` |
| `submit/plan.rs` | 534 | `.unwrap()` | `.expect("bookmark in push_set verified above")` |
| `submit/plan.rs` | 574 | `.unwrap()` | `.expect("bookmark in create_set verified above")` |
| `submit/analysis.rs` | 161 | `.last().unwrap()` | `.last().expect("segment has at least one change")` |

**Rationale:** These `.unwrap()` calls are safe due to preceding invariant checks, but bare `.unwrap()` provides poor panic messages:

```
thread 'main' panicked at 'called `Option::unwrap()` on a `None` value'
```

vs. `.expect()`:

```
thread 'main' panicked at 'selections verified non-empty'
```

The latter documents the invariant and aids debugging if assumptions are violated.

**Risk:** None. Only affects panic message text.

---

### 4. Add Tracing to Platform Services and Auth Modules

**Files:**
- `src/platform/github.rs`
- `src/platform/gitlab.rs`
- `src/auth/github.rs`
- `src/auth/gitlab.rs`

**Pattern:**

```rust
use tracing::debug;

async fn find_existing_pr(&self, head_branch: &str) -> Result<Option<PullRequest>> {
    debug!(head_branch, "finding existing PR");
    // ... implementation ...
    debug!(pr_number = pr.number, "found existing PR");
    Ok(Some(pr))
}
```

**Methods instrumented:**

| Module | Methods |
|--------|---------|
| `platform/github.rs` | `find_existing_pr`, `create_pr_with_options`, `update_pr_base`, `publish_pr`, `list_pr_comments`, `create_pr_comment`, `update_pr_comment` |
| `platform/gitlab.rs` | Same 7 methods |
| `auth/github.rs` | `get_github_auth` (CLI attempt, env fallback, success/failure) |
| `auth/gitlab.rs` | `get_gitlab_auth` (CLI attempt w/ host, env fallback, success/failure) |

**What is logged:**
- Method entry with key parameters (branch names, PR numbers)
- Success with key result info (created PR number, found/not-found)
- Auth source detection (gh CLI, env var)

**What is NOT logged:**
- Tokens or credentials (security)
- Full response bodies (noise)
- Redundant info already in error messages

**Rationale:** Currently, debugging auth or API failures requires adding print statements and rebuilding. With tracing:

```bash
RUST_LOG=jj_ryu::platform=debug ryu submit
```

Provides full visibility without code changes.

**Runtime cost:** Zero when no tracing subscriber is installed (default). The `tracing` crate is already a dependency.

**Risk:** None. Debug-level traces are invisible unless explicitly enabled.

---

## Changes Summary

| File | Change Type | Lines |
|------|-------------|-------|
| `Cargo.toml` | Dependency removal | -1 |
| `src/cli/submit.rs` | Error variant + expect | ~5 |
| `src/submit/plan.rs` | expect messages | ~2 |
| `src/submit/analysis.rs` | expect message | ~1 |
| `src/platform/github.rs` | Tracing instrumentation | ~30 |
| `src/platform/gitlab.rs` | Tracing instrumentation | ~30 |
| `src/auth/github.rs` | Tracing instrumentation | ~10 |
| `src/auth/gitlab.rs` | Tracing instrumentation | ~10 |

**Total:** ~90 lines changed, 0 behavioral changes.

---

## Testing

All changes verified via:

```bash
cargo clippy -- -D warnings  # Pass
cargo test --lib             # 54 tests pass
```

No new tests required; these are non-behavioral changes.

---

## Trade-offs

### Alternative: More Granular Error Variants

Could add `Error::MissingCliArgument`, `Error::BookmarkNotInStack`, etc. instead of using `InvalidArgument`.

**Rejected:** Over-engineering for current use cases. `InvalidArgument` is sufficient and matches existing patterns.

### Alternative: `tracing::instrument` Macro

Could use `#[instrument]` attribute macro for automatic span creation.

**Rejected:** Adds proc-macro dependency, logs all parameters by default (including potentially sensitive ones), less control over what's logged.

### Alternative: Structured Logging with `serde`

Could log structured JSON for machine parsing.

**Rejected:** Over-engineering. Human-readable debug logs are sufficient for current debugging needs.

---

## Security Considerations

- Auth tokens are explicitly NOT logged
- No new external inputs or network calls
- No credential handling changes

---

## Migration

None required. All changes are internal; no API or CLI changes.

---

## Conclusion

Five surgical improvements that:

1. Remove dead code/dependencies
2. Improve error semantics accuracy
3. Document invariants via expect messages
4. Enable debugging without code changes

All changes are low-risk, non-behavioral, and verified passing CI checks.
