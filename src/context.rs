use anyhow::{Context as _, Result};
use http::header::HeaderName;
use octocrab::{
    models,
    params::{pulls::Sort, Direction},
};
use std::fmt;

pub struct Client {
    pub inner: octocrab::Octocrab,
    pub owner: String,
}

impl Client {
    pub fn new(
        token: String,
        owner: String,
        github_api_base: Option<&str>,
        extra_headers: &[(String, String)],
    ) -> Result<Self> {
        let mut builder = octocrab::OctocrabBuilder::new();
        if let Some(base_url) = github_api_base {
            builder = builder.base_url(base_url)?;
        };
        for (key, value) in extra_headers {
            let name = HeaderName::from_lowercase(key.to_lowercase().as_bytes())?;
            builder = builder.add_header(name, value.to_string());
        }
        let inner = builder
            .personal_token(token)
            .build()
            .context("failed to create client")?;
        Ok(Self { inner, owner })
    }

    /// Get the currently open pull requests for the repo.
    ///
    /// Only the most recently pull requests are included as pagination is not
    /// handled. This is OK as we are not interested in outdated PRs as they
    /// won't have been updated since we last checked.
    pub async fn get_pull_requests(&self, repo: &str) -> Result<Vec<models::pulls::PullRequest>> {
        Ok(self
            .inner
            .pulls(&self.owner, repo)
            .list()
            .state(octocrab::params::State::Open)
            .direction(Direction::Descending)
            .sort(Sort::Updated)
            .send()
            .await
            .context("unable to retrieve pull requests")?
            .items)
    }

    /// Get the reviews for a PR
    pub async fn get_pull_request_reviews(
        &self,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<models::pulls::Review>> {
        let mut reviews = Vec::new();
        let page = self
            .inner
            .pulls(&self.owner, repo)
            .list_reviews(pr_number)
            .await
            .context("Could not get reviews for PR")?;
        let mut page = Some(page);
        while let Some(previous) = page {
            reviews.extend(previous.items);
            page = self.inner.get_page(&previous.next).await?;
        }
        tracing::info!(?reviews, "reviews we got from github api");
        Ok(reviews)
    }

    /// Get the statuses for a PR
    pub async fn get_pull_request_statuses(
        &self,
        repo: &str,
        pr: &crate::process::Pr,
    ) -> Result<Vec<models::Status>> {
        let reference = octocrab::params::repos::Reference::Commit(pr.commit_sha.clone());
        Ok(self
            .inner
            .repos(&self.owner, repo)
            .combined_status_for_ref(&reference)
            .await
            .context("Could not get statuses for commit")?
            .statuses)
    }
}

/// Configuration options available for the action
#[derive(serde::Deserialize)]
pub struct Config {
    /// The user or organisation that owns the repos
    pub owner: String,

    /// The repos to be run on, and their config
    pub repos: Vec<RepoConfig>,

    /// Whether to skip applying the changes or not.
    pub dry_run: bool,

    /// The base URL to use GitHub API.  This may be useful if you are using a
    /// proxy for the GitHub API or an enterprise installation.
    pub github_api_base: Option<String>,

    /// Extra headers to add to each request made to GitHub's API.
    #[serde(default)]
    pub extra_headers: Vec<(String, String)>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Config {
            owner,
            repos,
            dry_run,
            github_api_base,
            // not included since it contains secrets that we don't want in logs
            extra_headers: _,
        } = self;

        f.debug_struct("Config")
            .field("owner", owner)
            .field("repos", repos)
            .field("dry_run", dry_run)
            .field("github_api_base", github_api_base)
            .field("extra_headers", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RepoConfig {
    /// The name of the repo
    pub name: String,

    /// The label added when a PR does not have a body
    pub needs_description_label: Option<String>,

    /// The list of statuss that are required to be passed for the PR to be
    /// automerged
    pub required_statuses: Vec<String>,

    /// The label applied when all of the PR's required status checks have passed
    pub ci_passed_label: Option<String>,

    /// Label applied when a PR has 1 or more reviewers and all of them are accepted
    pub reviewed_label: Option<String>,

    /// Label that can be manually added to PRs to not block on reviews, for trivial changes
    ///
    /// If there's a `block_merge_label` set, it has priority over this label being set.
    pub trivial_review_label: Option<String>,

    /// Label that can be manually added to PRs to block automerge
    pub block_merge_label: Option<String>,

    /// The period in seconds between when a PR can be automerged, and when
    /// the action actually tries to perform the merge
    pub automerge_grace_period: Option<u64>,

    /// The method to use for merging the PR, defaults to `merge` if we fail
    /// to parse or it is unset by the user
    #[serde(default)]
    pub merge_method: MergeMethod,

    /// Whether a "comment" review counts as requesting changes. False by default.
    #[serde(default)]
    pub comment_requests_change: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl Default for MergeMethod {
    fn default() -> Self {
        Self::Merge
    }
}

impl From<MergeMethod> for octocrab::params::pulls::MergeMethod {
    fn from(m: MergeMethod) -> Self {
        use octocrab::params::pulls::MergeMethod as MM;
        match m {
            MergeMethod::Merge => MM::Merge,
            MergeMethod::Squash => MM::Squash,
            MergeMethod::Rebase => MM::Rebase,
        }
    }
}
