//! Shared command context for CLI commands
//!
//! Extracts common setup code shared by submit, sync, and merge commands.

use jj_ryu::error::{Error, Result};
use jj_ryu::platform::{create_platform_service, parse_repo_info, PlatformService};
use jj_ryu::repo::{select_remote, JjWorkspace};
use jj_ryu::tracking::{load_pr_cache, load_tracking, PrCache, TrackingState};
use std::path::{Path, PathBuf};

/// Shared context for CLI commands that interact with the platform
///
/// This struct encapsulates the common setup needed by submit, sync, and merge:
/// - Opening the jj workspace
/// - Loading tracking state and PR cache
/// - Selecting and validating the remote
/// - Detecting the platform and creating the service
///
/// Note: Does NOT include `ChangeGraph` because it becomes stale after
/// fetch/rebase operations. Callers should build the graph when needed
/// via `build_change_graph()`.
pub struct CommandContext {
    /// The jj workspace
    pub workspace: JjWorkspace,
    /// Root path of the workspace
    pub workspace_root: PathBuf,
    /// Tracking state for bookmarks
    pub tracking: TrackingState,
    /// PR cache for bookmark → PR mappings
    pub pr_cache: PrCache,
    /// Platform service (GitHub/GitLab)
    pub platform: Box<dyn PlatformService>,
    /// Selected remote name
    pub remote_name: String,
    /// Default branch name (e.g., "main")
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
    /// - Get default branch
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
    #[allow(dead_code)] // Will be used by merge command
    pub fn has_tracked_bookmarks(&self) -> bool {
        !self.tracking.tracked_names().is_empty()
    }

    /// Get tracked bookmark names
    pub fn tracked_names(&self) -> Vec<&str> {
        self.tracking.tracked_names()
    }
}
