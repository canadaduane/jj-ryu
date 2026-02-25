//! GitHub platform service implementation

use crate::error::{Error, Result};
use crate::platform::PlatformService;
use crate::types::{
    MergeMethod, MergeReadiness, MergeResult, Platform, PlatformConfig, PrComment, PrState,
    PullRequest, PullRequestDetails,
};
use async_trait::async_trait;
use octocrab::Octocrab;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

// GraphQL response types for publish_pr mutation

#[derive(Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkReadyForReviewData {
    mark_pull_request_ready_for_review: MarkReadyPayload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkReadyPayload {
    pull_request: GraphQlPullRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPullRequest {
    number: u64,
    url: String,
    base_ref_name: String,
    head_ref_name: String,
    title: String,
    id: String,
    is_draft: bool,
}

impl From<GraphQlPullRequest> for PullRequest {
    fn from(pr: GraphQlPullRequest) -> Self {
        Self {
            number: pr.number,
            html_url: pr.url,
            base_ref: pr.base_ref_name,
            head_ref: pr.head_ref_name,
            title: pr.title,
            node_id: Some(pr.id),
            is_draft: pr.is_draft,
        }
    }
}

/// GitHub service using octocrab
pub struct GitHubService {
    client: Octocrab,
    config: PlatformConfig,
    /// Token for raw HTTP requests (CI status checking)
    token: String,
    /// HTTP client for raw requests (CI status checking)
    http_client: Client,
    /// API host for raw requests
    api_host: String,
}

impl GitHubService {
    /// Create a new GitHub service
    pub fn new(token: &str, owner: String, repo: String, host: Option<String>) -> Result<Self> {
        let mut builder = Octocrab::builder().personal_token(token.to_string());

        let api_host = if let Some(ref h) = host {
            let base_url = format!("https://{h}/api/v3");
            builder = builder
                .base_uri(&base_url)
                .map_err(|e| Error::GitHubApi(e.to_string()))?;
            format!("{h}/api/v3")
        } else {
            "api.github.com".to_string()
        };

        let client = builder
            .build()
            .map_err(|e| Error::GitHubApi(e.to_string()))?;

        let http_client = Client::builder()
            .user_agent("jj-ryu")
            .build()
            .map_err(|e| Error::GitHubApi(format!("Failed to create HTTP client: {e}")))?;

        Ok(Self {
            client,
            config: PlatformConfig {
                platform: Platform::GitHub,
                owner,
                repo,
                host,
            },
            token: token.to_string(),
            http_client,
            api_host,
        })
    }

    /// Check CI status by querying both commit statuses and check runs
    ///
    /// GitHub has two CI systems:
    /// 1. Commit Status API (legacy) - used by external CI services
    /// 2. Check Runs API (modern) - used by GitHub Actions
    ///
    /// We need to check both to properly determine CI status.
    async fn check_ci_status(&self, ref_name: &str) -> Result<bool> {
        // Check commit statuses (legacy API)
        let statuses_passed = self.check_commit_statuses(ref_name).await?;

        // Check check runs (GitHub Actions API)
        let check_runs_passed = self.check_check_runs(ref_name).await?;

        // CI passes if both pass (or are not configured)
        Ok(statuses_passed && check_runs_passed)
    }

    /// Check legacy commit statuses via combined status API
    async fn check_commit_statuses(&self, ref_name: &str) -> Result<bool> {
        #[derive(Deserialize)]
        struct CombinedStatus {
            state: String,
            total_count: u32,
        }

        let url = format!(
            "https://{}/repos/{}/{}/commits/{}/status",
            self.api_host, self.config.owner, self.config.repo, ref_name
        );

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| Error::GitHubApi(format!("Failed to fetch commit status: {e}")))?;

        if !response.status().is_success() {
            debug!(
                status = %response.status(),
                "Commit status check returned non-success, assuming no statuses configured"
            );
            return Ok(true);
        }

        let status: CombinedStatus = response
            .json()
            .await
            .map_err(|e| Error::GitHubApi(format!("Failed to parse commit status: {e}")))?;

        // No statuses configured = passing
        // "success" = all passed
        // "pending" or "failure" = not passing
        if status.total_count == 0 {
            debug!("No commit statuses configured");
            return Ok(true);
        }

        debug!(state = %status.state, count = status.total_count, "Commit status result");
        Ok(status.state == "success")
    }

    /// Check GitHub Actions check runs
    async fn check_check_runs(&self, ref_name: &str) -> Result<bool> {
        #[derive(Deserialize)]
        struct CheckRunsResponse {
            total_count: u32,
            check_runs: Vec<CheckRun>,
        }

        #[derive(Deserialize)]
        struct CheckRun {
            status: String,
            conclusion: Option<String>,
        }

        let url = format!(
            "https://{}/repos/{}/{}/commits/{}/check-runs",
            self.api_host, self.config.owner, self.config.repo, ref_name
        );

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| Error::GitHubApi(format!("Failed to fetch check runs: {e}")))?;

        if !response.status().is_success() {
            debug!(
                status = %response.status(),
                "Check runs returned non-success, assuming no checks configured"
            );
            return Ok(true);
        }

        let check_runs: CheckRunsResponse = response
            .json()
            .await
            .map_err(|e| Error::GitHubApi(format!("Failed to parse check runs: {e}")))?;

        // No check runs configured = passing
        if check_runs.total_count == 0 {
            debug!("No check runs configured");
            return Ok(true);
        }

        // All check runs must be completed with success/neutral/skipped
        for run in &check_runs.check_runs {
            // If any check is still running, CI is not complete
            if run.status != "completed" {
                debug!(status = %run.status, "Check run still in progress");
                return Ok(false);
            }

            // Check conclusion for completed runs
            match run.conclusion.as_deref() {
                Some("success" | "neutral" | "skipped") => {
                    // These are passing conclusions
                }
                Some(conclusion) => {
                    debug!(conclusion = %conclusion, "Check run failed");
                    return Ok(false);
                }
                None => {
                    // Completed but no conclusion? Treat as failure
                    debug!("Check run completed but no conclusion");
                    return Ok(false);
                }
            }
        }

        debug!(count = check_runs.total_count, "All check runs passed");
        Ok(true)
    }
}

/// Helper to convert octocrab PR to our `PullRequest` type
fn pr_from_octocrab(pr: &octocrab::models::pulls::PullRequest) -> PullRequest {
    PullRequest {
        number: pr.number,
        html_url: pr
            .html_url
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        base_ref: pr.base.ref_field.clone(),
        head_ref: pr.head.ref_field.clone(),
        title: pr.title.as_deref().unwrap_or_default().to_string(),
        node_id: pr.node_id.clone(),
        is_draft: pr.draft.unwrap_or(false),
    }
}

#[async_trait]
impl PlatformService for GitHubService {
    async fn find_existing_pr(&self, head_branch: &str) -> Result<Option<PullRequest>> {
        debug!(head_branch, "finding existing PR");
        let head = format!("{}:{}", &self.config.owner, head_branch);

        let prs = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .list()
            .head(head)
            .state(octocrab::params::State::Open)
            .send()
            .await?;

        let result = prs.items.first().map(pr_from_octocrab);
        if let Some(ref pr) = result {
            debug!(pr_number = pr.number, "found existing PR");
        } else {
            debug!("no existing PR found");
        }
        Ok(result)
    }

    async fn create_pr_with_options(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: Option<&str>,
        draft: bool,
    ) -> Result<PullRequest> {
        debug!(head, base, draft, "creating PR");
        let pulls = self.client.pulls(&self.config.owner, &self.config.repo);
        let mut builder = pulls.create(title, head, base).draft(draft);

        if let Some(body_text) = body {
            builder = builder.body(body_text);
        }

        let pr = builder.send().await?;

        let result = pr_from_octocrab(&pr);
        debug!(pr_number = result.number, "created PR");
        Ok(result)
    }

    async fn update_pr_base(&self, pr_number: u64, new_base: &str) -> Result<PullRequest> {
        debug!(pr_number, new_base, "updating PR base");
        let pr = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .update(pr_number)
            .base(new_base)
            .send()
            .await?;

        debug!(pr_number, "updated PR base");
        Ok(pr_from_octocrab(&pr))
    }

    async fn publish_pr(&self, pr_number: u64) -> Result<PullRequest> {
        debug!(pr_number, "publishing PR");
        // Fetch PR to get node_id for GraphQL mutation
        let pr = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .get(pr_number)
            .await?;

        let node_id = pr.node_id.as_ref().ok_or_else(|| {
            Error::GitHubApi("PR missing node_id for GraphQL mutation".to_string())
        })?;

        // Execute GraphQL mutation to mark PR as ready for review
        let response: GraphQlResponse<MarkReadyForReviewData> = self
            .client
            .graphql(&serde_json::json!({
                "query": r"
                    mutation MarkPullRequestReadyForReview($pullRequestId: ID!) {
                        markPullRequestReadyForReview(input: { pullRequestId: $pullRequestId }) {
                            pullRequest {
                                number
                                url
                                baseRefName
                                headRefName
                                title
                                id
                                isDraft
                            }
                        }
                    }
                ",
                "variables": {
                    "pullRequestId": node_id
                }
            }))
            .await
            .map_err(|e| Error::GitHubApi(format!("GraphQL mutation failed: {e}")))?;

        // Check for GraphQL errors
        if let Some(errors) = response.errors
            && !errors.is_empty()
        {
            let messages: Vec<_> = errors.into_iter().map(|e| e.message).collect();
            return Err(Error::GitHubApi(format!(
                "GraphQL error: {}",
                messages.join(", ")
            )));
        }

        // Extract typed response
        let data = response
            .data
            .ok_or_else(|| Error::GitHubApi("No data in GraphQL response".to_string()))?;

        debug!(pr_number, "published PR");
        Ok(data.mark_pull_request_ready_for_review.pull_request.into())
    }

    async fn list_pr_comments(&self, pr_number: u64) -> Result<Vec<PrComment>> {
        debug!(pr_number, "listing PR comments");
        let comments = self
            .client
            .issues(&self.config.owner, &self.config.repo)
            .list_comments(pr_number)
            .send()
            .await?;

        let result: Vec<PrComment> = comments
            .items
            .into_iter()
            .map(|c| PrComment {
                id: c.id.0,
                body: c.body.unwrap_or_default(),
            })
            .collect();
        debug!(pr_number, count = result.len(), "listed PR comments");
        Ok(result)
    }

    async fn create_pr_comment(&self, pr_number: u64, body: &str) -> Result<()> {
        debug!(pr_number, "creating PR comment");
        self.client
            .issues(&self.config.owner, &self.config.repo)
            .create_comment(pr_number, body)
            .await?;
        debug!(pr_number, "created PR comment");
        Ok(())
    }

    async fn update_pr_comment(&self, _pr_number: u64, comment_id: u64, body: &str) -> Result<()> {
        debug!(comment_id, "updating PR comment");
        self.client
            .issues(&self.config.owner, &self.config.repo)
            .update_comment(octocrab::models::CommentId(comment_id), body)
            .await?;
        debug!(comment_id, "updated PR comment");
        Ok(())
    }

    fn config(&self) -> &PlatformConfig {
        &self.config
    }

    // =========================================================================
    // Merge-related methods
    // =========================================================================

    async fn get_pr_details(&self, pr_number: u64) -> Result<PullRequestDetails> {
        debug!(pr_number, "getting PR details");

        let pr = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .get(pr_number)
            .await?;

        // Determine PR state from GitHub's state field and merged_at
        let state = match pr.state {
            Some(octocrab::models::IssueState::Open) => PrState::Open,
            Some(octocrab::models::IssueState::Closed) if pr.merged_at.is_some() => PrState::Merged,
            // IssueState is non-exhaustive, so use wildcard for Closed and any future variants
            Some(_) | None => PrState::Closed,
        };

        let details = PullRequestDetails {
            number: pr.number,
            title: pr.title.clone().unwrap_or_default(),
            body: pr.body.clone(),
            state,
            is_draft: pr.draft.unwrap_or(false),
            mergeable: pr.mergeable,
            head_ref: pr.head.ref_field.clone(),
            base_ref: pr.base.ref_field.clone(),
            html_url: pr
                .html_url
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        };

        debug!(pr_number, state = ?details.state, "got PR details");
        Ok(details)
    }

    async fn check_merge_readiness(&self, pr_number: u64) -> Result<MergeReadiness> {
        debug!(pr_number, "checking merge readiness");

        // Get PR details first
        let details = self.get_pr_details(pr_number).await?;

        // Check reviews for approval
        let reviews = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .list_reviews(pr_number)
            .send()
            .await?;

        // Look for at least one APPROVED review
        let is_approved = reviews.items.iter().any(|r| {
            r.state
                .as_ref()
                .is_some_and(|s| *s == octocrab::models::pulls::ReviewState::Approved)
        });

        // Check CI status
        let ci_passed = self
            .check_ci_status(&details.head_ref)
            .await
            .unwrap_or(true); // If we can't check, assume passing

        // Build blocking reasons
        let mut blocking_reasons = Vec::new();
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
            blocking_reasons.push("Merge status unknown (still computing)".to_string());
        }

        let readiness = MergeReadiness {
            is_approved,
            ci_passed,
            is_mergeable: details.mergeable.unwrap_or(false),
            is_draft: details.is_draft,
            blocking_reasons,
        };

        debug!(
            pr_number,
            can_merge = readiness.can_merge(),
            "checked merge readiness"
        );
        Ok(readiness)
    }

    async fn merge_pr(&self, pr_number: u64, method: MergeMethod) -> Result<MergeResult> {
        debug!(pr_number, %method, "merging PR");

        // Get PR details for commit message (squash needs title/body)
        let details = self.get_pr_details(pr_number).await?;

        let octocrab_method = match method {
            MergeMethod::Squash => octocrab::params::pulls::MergeMethod::Squash,
            MergeMethod::Merge => octocrab::params::pulls::MergeMethod::Merge,
            MergeMethod::Rebase => octocrab::params::pulls::MergeMethod::Rebase,
        };

        let pulls = self.client.pulls(&self.config.owner, &self.config.repo);

        // Build and send merge request
        // For squash, use PR title and body as commit message
        let result = if method == MergeMethod::Squash {
            let mut builder = pulls.merge(pr_number).method(octocrab_method);
            builder = builder.title(format!("{} (#{})", details.title, pr_number));
            if let Some(ref body) = details.body {
                builder = builder.message(body);
            }
            builder.send().await
        } else {
            pulls.merge(pr_number).method(octocrab_method).send().await
        }
        .map_err(|e| Error::GitHubApi(format!("Merge failed: {e}")))?;

        let merge_result = MergeResult {
            merged: result.merged,
            sha: result.sha,
            message: result.message,
        };

        debug!(
            pr_number,
            merged = merge_result.merged,
            sha = ?merge_result.sha,
            "merge complete"
        );
        Ok(merge_result)
    }
}
