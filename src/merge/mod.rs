//! Merge engine for stacked PRs
//!
//! Three-phase pattern matching submit/:
//! 1. Gather - fetch PR details and readiness (effectful, bounded)
//! 2. Plan - create `MergePlan` (pure, testable)
//! 3. Execute - perform merges (effectful)

mod execute;
mod plan;

pub use execute::{execute_merge, MergeExecutionResult};
pub use plan::{create_merge_plan, MergePlan, MergePlanOptions, MergeStep, PrInfo};
