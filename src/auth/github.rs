//! GitHub authentication

use crate::auth::AuthSource;
use crate::error::{Error, Result};
use std::env;
use tokio::process::Command;

/// GitHub authentication configuration
#[derive(Debug, Clone)]
pub struct GitHubAuthConfig {
    /// Authentication token
    pub token: String,
    /// Where the token was obtained from
    pub source: AuthSource,
}

/// Get GitHub authentication
///
/// Priority:
/// 1. gh CLI (`gh auth token`)
/// 2. `GITHUB_TOKEN` environment variable
/// 3. `GH_TOKEN` environment variable
pub async fn get_github_auth() -> Result<GitHubAuthConfig> {
    // Try gh CLI first
    if let Some(token) = get_gh_cli_token().await {
        return Ok(GitHubAuthConfig {
            token,
            source: AuthSource::Cli,
        });
    }

    // Try environment variables
    if let Ok(token) = env::var("GITHUB_TOKEN") {
        return Ok(GitHubAuthConfig {
            token,
            source: AuthSource::EnvVar,
        });
    }

    if let Ok(token) = env::var("GH_TOKEN") {
        return Ok(GitHubAuthConfig {
            token,
            source: AuthSource::EnvVar,
        });
    }

    Err(Error::Auth(
        "No GitHub authentication found. Run `gh auth login` or set GITHUB_TOKEN".to_string(),
    ))
}

async fn get_gh_cli_token() -> Option<String> {
    // Check gh is available
    Command::new("gh").arg("--version").output().await.ok()?;

    // Check authenticated
    let status = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .await
        .ok()?;

    if !status.status.success() {
        return None;
    }

    // Get token
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

/// Test GitHub authentication
pub async fn test_github_auth(config: &GitHubAuthConfig) -> Result<String> {
    let octocrab = octocrab::Octocrab::builder()
        .personal_token(config.token.clone())
        .build()
        .map_err(|e| Error::GitHubApi(e.to_string()))?;

    let user = octocrab
        .current()
        .user()
        .await
        .map_err(|e| Error::Auth(format!("Invalid token: {e}")))?;

    Ok(user.login)
}
