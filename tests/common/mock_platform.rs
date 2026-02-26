//! Mock platform service for testing
//!
//! These are test utilities - not all may be used in current tests but are
//! available for future test development.

#![allow(dead_code)]

use async_trait::async_trait;
use jj_ryu::error::{Error, Result};
use jj_ryu::platform::PlatformService;
use jj_ryu::types::{
    MergeMethod, MergeReadiness, MergeResult, PlatformConfig, PrComment, PrState, PullRequest,
    PullRequestDetails,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Call record for `create_pr`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePrCall {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: Option<String>,
}

/// Call record for `update_pr_base`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateBaseCall {
    pub pr_number: u64,
    pub new_base: String,
}

/// Call record for `create_pr_comment`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCommentCall {
    pub pr_number: u64,
    pub body: String,
}

/// Call record for `merge_pr`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePrCall {
    pub pr_number: u64,
    pub method: MergeMethod,
}

/// Simple mock platform service for testing
///
/// This manually implements `PlatformService` rather than using mockall,
/// because mockall has issues with methods returning references.
///
/// Features:
/// - Auto-incrementing PR numbers
/// - Call tracking for verification
/// - Configurable responses per branch
/// - Error injection for failure path testing
pub struct MockPlatformService {
    config: PlatformConfig,
    next_pr_number: AtomicU64,
    find_pr_responses: Mutex<HashMap<String, Option<PullRequest>>>,
    list_comments_responses: Mutex<HashMap<u64, Vec<PrComment>>>,
    // Call tracking
    find_pr_calls: Mutex<Vec<String>>,
    create_pr_calls: Mutex<Vec<CreatePrCall>>,
    update_base_calls: Mutex<Vec<UpdateBaseCall>>,
    create_comment_calls: Mutex<Vec<CreateCommentCall>>,
    list_comments_calls: Mutex<Vec<u64>>,
    // Error injection
    error_on_find_pr: Mutex<Option<String>>,
    error_on_create_pr: Mutex<Option<String>>,
    error_on_update_base: Mutex<Option<String>>,
    // Merge-related response maps
    pr_details_responses: Mutex<HashMap<u64, PullRequestDetails>>,
    merge_readiness_responses: Mutex<HashMap<u64, MergeReadiness>>,
    merge_responses: Mutex<HashMap<u64, MergeResult>>,
    // Merge-related call tracking
    get_pr_details_calls: Mutex<Vec<u64>>,
    check_merge_readiness_calls: Mutex<Vec<u64>>,
    merge_pr_calls: Mutex<Vec<MergePrCall>>,
    // Merge-related error injection
    error_on_merge_pr: Mutex<Option<String>>,
}

impl MockPlatformService {
    /// Create a new mock with the given config
    pub fn with_config(config: PlatformConfig) -> Self {
        Self {
            config,
            next_pr_number: AtomicU64::new(1),
            find_pr_responses: Mutex::new(HashMap::new()),
            list_comments_responses: Mutex::new(HashMap::new()),
            find_pr_calls: Mutex::new(Vec::new()),
            create_pr_calls: Mutex::new(Vec::new()),
            update_base_calls: Mutex::new(Vec::new()),
            create_comment_calls: Mutex::new(Vec::new()),
            list_comments_calls: Mutex::new(Vec::new()),
            error_on_find_pr: Mutex::new(None),
            error_on_create_pr: Mutex::new(None),
            error_on_update_base: Mutex::new(None),
            pr_details_responses: Mutex::new(HashMap::new()),
            merge_readiness_responses: Mutex::new(HashMap::new()),
            merge_responses: Mutex::new(HashMap::new()),
            get_pr_details_calls: Mutex::new(Vec::new()),
            check_merge_readiness_calls: Mutex::new(Vec::new()),
            merge_pr_calls: Mutex::new(Vec::new()),
            error_on_merge_pr: Mutex::new(None),
        }
    }

    // === Error injection methods ===

    /// Make `find_existing_pr` return an error
    pub fn fail_find_pr(&self, msg: &str) {
        *self.error_on_find_pr.lock().unwrap() = Some(msg.to_string());
    }

    /// Make `create_pr` return an error
    pub fn fail_create_pr(&self, msg: &str) {
        *self.error_on_create_pr.lock().unwrap() = Some(msg.to_string());
    }

    /// Make `update_pr_base` return an error
    pub fn fail_update_base(&self, msg: &str) {
        *self.error_on_update_base.lock().unwrap() = Some(msg.to_string());
    }

    /// Make `merge_pr` return an error
    pub fn fail_merge_pr(&self, msg: &str) {
        *self.error_on_merge_pr.lock().unwrap() = Some(msg.to_string());
    }

    /// Set the response for `find_existing_pr` for a specific branch
    pub fn set_find_pr_response(&self, branch: &str, pr: Option<PullRequest>) {
        self.find_pr_responses
            .lock()
            .unwrap()
            .insert(branch.to_string(), pr);
    }

    /// Set the response for `list_pr_comments` for a specific PR
    pub fn set_list_comments_response(&self, pr_number: u64, comments: Vec<PrComment>) {
        self.list_comments_responses
            .lock()
            .unwrap()
            .insert(pr_number, comments);
    }

    /// Set the response for `get_pr_details` for a specific PR
    pub fn set_pr_details_response(&self, pr_number: u64, details: PullRequestDetails) {
        self.pr_details_responses
            .lock()
            .unwrap()
            .insert(pr_number, details);
    }

    /// Set the response for `check_merge_readiness` for a specific PR
    pub fn set_merge_readiness_response(&self, pr_number: u64, readiness: MergeReadiness) {
        self.merge_readiness_responses
            .lock()
            .unwrap()
            .insert(pr_number, readiness);
    }

    /// Set the response for `merge_pr` for a specific PR
    pub fn set_merge_response(&self, pr_number: u64, result: MergeResult) {
        self.merge_responses
            .lock()
            .unwrap()
            .insert(pr_number, result);
    }

    /// Helper to set up a mergeable PR with all required responses
    pub fn setup_mergeable_pr(&self, pr_number: u64, bookmark: &str, title: &str) {
        // Set find_pr response
        self.set_find_pr_response(
            bookmark,
            Some(PullRequest {
                number: pr_number,
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
                base_ref: "main".to_string(),
                head_ref: bookmark.to_string(),
                title: title.to_string(),
                node_id: Some(format!("PR_node_{pr_number}")),
                is_draft: false,
            }),
        );

        // Set PR details
        self.set_pr_details_response(
            pr_number,
            PullRequestDetails {
                number: pr_number,
                title: title.to_string(),
                body: Some("PR body".to_string()),
                state: PrState::Open,
                is_draft: false,
                mergeable: Some(true),
                head_ref: bookmark.to_string(),
                base_ref: "main".to_string(),
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            },
        );

        // Set merge readiness (approved and ready)
        self.set_merge_readiness_response(
            pr_number,
            MergeReadiness {
                is_approved: true,
                ci_passed: true,
                is_mergeable: Some(true),
                is_draft: false,
                blocking_reasons: vec![],
                uncertainties: vec![],
            },
        );

        // Set merge response (success)
        self.set_merge_response(
            pr_number,
            MergeResult {
                merged: true,
                sha: Some(format!("merged_sha_{pr_number}")),
                message: None,
            },
        );
    }

    /// Helper to set up a non-mergeable PR (e.g., not approved)
    pub fn setup_blocked_pr(&self, pr_number: u64, bookmark: &str, title: &str, reasons: Vec<String>) {
        // Set find_pr response
        self.set_find_pr_response(
            bookmark,
            Some(PullRequest {
                number: pr_number,
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
                base_ref: "main".to_string(),
                head_ref: bookmark.to_string(),
                title: title.to_string(),
                node_id: Some(format!("PR_node_{pr_number}")),
                is_draft: false,
            }),
        );

        // Set PR details
        self.set_pr_details_response(
            pr_number,
            PullRequestDetails {
                number: pr_number,
                title: title.to_string(),
                body: Some("PR body".to_string()),
                state: PrState::Open,
                is_draft: false,
                mergeable: Some(true),
                head_ref: bookmark.to_string(),
                base_ref: "main".to_string(),
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            },
        );

        // Set merge readiness (blocked)
        self.set_merge_readiness_response(
            pr_number,
            MergeReadiness {
                is_approved: false,
                ci_passed: true,
                is_mergeable: Some(true),
                is_draft: false,
                blocking_reasons: reasons,
                uncertainties: vec![],
            },
        );
    }

    /// Helper to set up a PR with uncertain merge status (GitHub still computing)
    pub fn setup_uncertain_pr(&self, pr_number: u64, bookmark: &str, title: &str) {
        // Set find_pr response
        self.set_find_pr_response(
            bookmark,
            Some(PullRequest {
                number: pr_number,
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
                base_ref: "main".to_string(),
                head_ref: bookmark.to_string(),
                title: title.to_string(),
                node_id: Some(format!("PR_node_{pr_number}")),
                is_draft: false,
            }),
        );

        // Set PR details with mergeable: None (unknown)
        self.set_pr_details_response(
            pr_number,
            PullRequestDetails {
                number: pr_number,
                title: title.to_string(),
                body: Some("PR body".to_string()),
                state: PrState::Open,
                is_draft: false,
                mergeable: None, // Unknown - GitHub still computing
                head_ref: bookmark.to_string(),
                base_ref: "main".to_string(),
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            },
        );

        // Set merge readiness with uncertainty
        self.set_merge_readiness_response(
            pr_number,
            MergeReadiness {
                is_approved: true,
                ci_passed: true,
                is_mergeable: None, // Must match details.mergeable
                is_draft: false,
                blocking_reasons: vec![],
                uncertainties: vec!["Merge status unknown (GitHub still computing)".to_string()],
            },
        );

        // Set merge response (optimistic - assume it will work)
        self.set_merge_response(
            pr_number,
            MergeResult {
                merged: true,
                sha: Some(format!("merged_sha_{pr_number}")),
                message: None,
            },
        );
    }

    // === Call verification methods ===

    /// Get all branches that `find_existing_pr` was called with
    pub fn get_find_pr_calls(&self) -> Vec<String> {
        self.find_pr_calls.lock().unwrap().clone()
    }

    /// Get all `create_pr` calls
    pub fn get_create_pr_calls(&self) -> Vec<CreatePrCall> {
        self.create_pr_calls.lock().unwrap().clone()
    }

    /// Get all `update_pr_base` calls
    pub fn get_update_base_calls(&self) -> Vec<UpdateBaseCall> {
        self.update_base_calls.lock().unwrap().clone()
    }

    /// Get all `create_pr_comment` calls
    pub fn get_create_comment_calls(&self) -> Vec<CreateCommentCall> {
        self.create_comment_calls.lock().unwrap().clone()
    }

    /// Get all `list_pr_comments` calls
    pub fn get_list_comments_calls(&self) -> Vec<u64> {
        self.list_comments_calls.lock().unwrap().clone()
    }

    /// Get all `get_pr_details` calls
    pub fn get_pr_details_calls(&self) -> Vec<u64> {
        self.get_pr_details_calls.lock().unwrap().clone()
    }

    /// Get all `check_merge_readiness` calls
    pub fn get_merge_readiness_calls(&self) -> Vec<u64> {
        self.check_merge_readiness_calls.lock().unwrap().clone()
    }

    /// Get all `merge_pr` calls
    pub fn get_merge_pr_calls(&self) -> Vec<MergePrCall> {
        self.merge_pr_calls.lock().unwrap().clone()
    }

    /// Assert that `create_pr` was called with specific head and base
    pub fn assert_create_pr_called(&self, head: &str, base: &str) {
        let calls = self.get_create_pr_calls();
        assert!(
            calls.iter().any(|c| c.head == head && c.base == base),
            "Expected create_pr({head}, {base}) but got: {calls:?}"
        );
    }

    /// Assert that `update_pr_base` was called with specific args
    pub fn assert_update_base_called(&self, pr_number: u64, new_base: &str) {
        let calls = self.get_update_base_calls();
        assert!(
            calls
                .iter()
                .any(|c| c.pr_number == pr_number && c.new_base == new_base),
            "Expected update_pr_base({pr_number}, {new_base}) but got: {calls:?}"
        );
    }

    /// Assert that `find_existing_pr` was called for each bookmark
    pub fn assert_find_pr_called_for(&self, branches: &[&str]) {
        let calls = self.get_find_pr_calls();
        for branch in branches {
            assert!(
                calls.contains(&branch.to_string()),
                "Expected find_existing_pr({branch}) but got: {calls:?}"
            );
        }
    }

    /// Assert that `merge_pr` was called for a specific PR
    pub fn assert_merge_called(&self, pr_number: u64) {
        let calls = self.get_merge_pr_calls();
        assert!(
            calls.iter().any(|c| c.pr_number == pr_number),
            "Expected merge_pr({pr_number}) but got: {calls:?}"
        );
    }

    /// Assert that `merge_pr` was NOT called for a specific PR
    pub fn assert_merge_not_called(&self, pr_number: u64) {
        let calls = self.get_merge_pr_calls();
        assert!(
            !calls.iter().any(|c| c.pr_number == pr_number),
            "Expected merge_pr({pr_number}) NOT to be called but it was: {calls:?}"
        );
    }

    /// Assert that `merge_pr` was called with a specific method
    pub fn assert_merge_called_with_method(&self, pr_number: u64, method: MergeMethod) {
        let calls = self.get_merge_pr_calls();
        assert!(
            calls.iter().any(|c| c.pr_number == pr_number && c.method == method),
            "Expected merge_pr({pr_number}, {method:?}) but got: {calls:?}"
        );
    }

    /// Get count of merge_pr calls
    pub fn merge_call_count(&self) -> usize {
        self.merge_pr_calls.lock().unwrap().len()
    }
}

#[async_trait]
impl PlatformService for MockPlatformService {
    async fn find_existing_pr(&self, head_branch: &str) -> Result<Option<PullRequest>> {
        self.find_pr_calls
            .lock()
            .unwrap()
            .push(head_branch.to_string());

        // Check for injected error
        if let Some(msg) = self.error_on_find_pr.lock().unwrap().as_ref() {
            return Err(Error::Platform(msg.clone()));
        }

        let responses = self.find_pr_responses.lock().unwrap();
        Ok(responses.get(head_branch).cloned().flatten())
    }

    async fn create_pr_with_options(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: Option<&str>,
        draft: bool,
    ) -> Result<PullRequest> {
        self.create_pr_calls.lock().unwrap().push(CreatePrCall {
            head: head.to_string(),
            base: base.to_string(),
            title: title.to_string(),
            body: body.map(ToString::to_string),
        });

        // Check for injected error
        if let Some(msg) = self.error_on_create_pr.lock().unwrap().as_ref() {
            return Err(Error::Platform(msg.clone()));
        }

        let number = self.next_pr_number.fetch_add(1, Ordering::SeqCst);
        let pr = PullRequest {
            number,
            html_url: format!("https://github.com/test/repo/pull/{number}"),
            base_ref: base.to_string(),
            head_ref: head.to_string(),
            title: title.to_string(),
            node_id: Some(format!("PR_node_{number}")),
            is_draft: draft,
        };
        Ok(pr)
    }

    async fn update_pr_base(&self, pr_number: u64, new_base: &str) -> Result<PullRequest> {
        self.update_base_calls.lock().unwrap().push(UpdateBaseCall {
            pr_number,
            new_base: new_base.to_string(),
        });

        // Check for injected error
        if let Some(msg) = self.error_on_update_base.lock().unwrap().as_ref() {
            return Err(Error::Platform(msg.clone()));
        }

        Ok(PullRequest {
            number: pr_number,
            html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            base_ref: new_base.to_string(),
            head_ref: "updated".to_string(),
            title: "Updated PR".to_string(),
            node_id: Some(format!("PR_node_{pr_number}")),
            is_draft: false,
        })
    }

    async fn list_pr_comments(&self, pr_number: u64) -> Result<Vec<PrComment>> {
        self.list_comments_calls.lock().unwrap().push(pr_number);
        let responses = self.list_comments_responses.lock().unwrap();
        Ok(responses.get(&pr_number).cloned().unwrap_or_default())
    }

    async fn create_pr_comment(&self, pr_number: u64, body: &str) -> Result<()> {
        self.create_comment_calls
            .lock()
            .unwrap()
            .push(CreateCommentCall {
                pr_number,
                body: body.to_string(),
            });
        Ok(())
    }

    async fn update_pr_comment(
        &self,
        _pr_number: u64,
        _comment_id: u64,
        _body: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn publish_pr(&self, pr_number: u64) -> Result<PullRequest> {
        Ok(PullRequest {
            number: pr_number,
            html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            base_ref: "main".to_string(),
            head_ref: "published".to_string(),
            title: "Published PR".to_string(),
            node_id: Some(format!("PR_node_{pr_number}")),
            is_draft: false, // After publishing, is_draft is false
        })
    }

    fn config(&self) -> &PlatformConfig {
        &self.config
    }

    // =========================================================================
    // Merge-related methods
    // =========================================================================

    async fn get_pr_details(&self, pr_number: u64) -> Result<PullRequestDetails> {
        self.get_pr_details_calls.lock().unwrap().push(pr_number);

        let responses = self.pr_details_responses.lock().unwrap();
        responses.get(&pr_number).cloned().ok_or_else(|| {
            Error::Platform(format!(
                "get_pr_details: no response configured for PR #{pr_number}"
            ))
        })
    }

    async fn check_merge_readiness(&self, pr_number: u64) -> Result<MergeReadiness> {
        self.check_merge_readiness_calls
            .lock()
            .unwrap()
            .push(pr_number);

        let responses = self.merge_readiness_responses.lock().unwrap();
        responses.get(&pr_number).cloned().ok_or_else(|| {
            Error::Platform(format!(
                "check_merge_readiness: no response configured for PR #{pr_number}"
            ))
        })
    }

    async fn merge_pr(&self, pr_number: u64, method: MergeMethod) -> Result<MergeResult> {
        self.merge_pr_calls
            .lock()
            .unwrap()
            .push(MergePrCall { pr_number, method });

        // Check for injected error
        if let Some(msg) = self.error_on_merge_pr.lock().unwrap().as_ref() {
            return Err(Error::Platform(msg.clone()));
        }

        let responses = self.merge_responses.lock().unwrap();
        responses.get(&pr_number).cloned().ok_or_else(|| {
            Error::Platform(format!(
                "merge_pr: no response configured for PR #{pr_number}"
            ))
        })
    }
}
