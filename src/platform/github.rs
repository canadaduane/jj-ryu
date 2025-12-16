//! GitHub platform service implementation

use crate::error::{Error, Result};
use crate::platform::PlatformService;
use crate::types::{Platform, PlatformConfig, PrComment, PullRequest};
use async_trait::async_trait;
use octocrab::Octocrab;

/// GitHub service using octocrab
pub struct GitHubService {
    client: Octocrab,
    config: PlatformConfig,
}

impl GitHubService {
    /// Create a new GitHub service
    pub fn new(token: &str, owner: String, repo: String, host: Option<String>) -> Result<Self> {
        let mut builder = Octocrab::builder().personal_token(token.to_string());

        if let Some(ref h) = host {
            let base_url = format!("https://{h}/api/v3");
            builder = builder
                .base_uri(&base_url)
                .map_err(|e| Error::GitHubApi(e.to_string()))?;
        }

        let client = builder
            .build()
            .map_err(|e| Error::GitHubApi(e.to_string()))?;

        Ok(Self {
            client,
            config: PlatformConfig {
                platform: Platform::GitHub,
                owner,
                repo,
                host,
            },
        })
    }
}

#[async_trait]
impl PlatformService for GitHubService {
    async fn find_existing_pr(&self, head_branch: &str) -> Result<Option<PullRequest>> {
        let head = format!("{}:{}", &self.config.owner, head_branch);

        let prs = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .list()
            .head(head)
            .state(octocrab::params::State::Open)
            .send()
            .await?;

        Ok(prs.items.first().map(|pr| PullRequest {
            number: pr.number,
            html_url: pr
                .html_url
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            base_ref: pr.base.ref_field.clone(),
            head_ref: pr.head.ref_field.clone(),
            title: pr.title.as_deref().unwrap_or_default().to_string(),
        }))
    }

    async fn create_pr(&self, head: &str, base: &str, title: &str) -> Result<PullRequest> {
        let pr = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .create(title, head, base)
            .send()
            .await?;

        Ok(PullRequest {
            number: pr.number,
            html_url: pr
                .html_url
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            base_ref: pr.base.ref_field.clone(),
            head_ref: pr.head.ref_field.clone(),
            title: pr.title.as_deref().unwrap_or_default().to_string(),
        })
    }

    async fn update_pr_base(&self, pr_number: u64, new_base: &str) -> Result<PullRequest> {
        let pr = self
            .client
            .pulls(&self.config.owner, &self.config.repo)
            .update(pr_number)
            .base(new_base)
            .send()
            .await?;

        Ok(PullRequest {
            number: pr.number,
            html_url: pr
                .html_url
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            base_ref: pr.base.ref_field.clone(),
            head_ref: pr.head.ref_field.clone(),
            title: pr.title.as_deref().unwrap_or_default().to_string(),
        })
    }

    async fn list_pr_comments(&self, pr_number: u64) -> Result<Vec<PrComment>> {
        let comments = self
            .client
            .issues(&self.config.owner, &self.config.repo)
            .list_comments(pr_number)
            .send()
            .await?;

        Ok(comments
            .items
            .into_iter()
            .map(|c| PrComment {
                id: c.id.0,
                body: c.body.unwrap_or_default(),
            })
            .collect())
    }

    async fn create_pr_comment(&self, pr_number: u64, body: &str) -> Result<()> {
        self.client
            .issues(&self.config.owner, &self.config.repo)
            .create_comment(pr_number, body)
            .await?;
        Ok(())
    }

    async fn update_pr_comment(&self, _pr_number: u64, comment_id: u64, body: &str) -> Result<()> {
        self.client
            .issues(&self.config.owner, &self.config.repo)
            .update_comment(octocrab::models::CommentId(comment_id), body)
            .await?;
        Ok(())
    }

    fn config(&self) -> &PlatformConfig {
        &self.config
    }
}
