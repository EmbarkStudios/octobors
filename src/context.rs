// See https://github.com/actions/toolkit/blob/main/packages/github/src/context.ts
// for more details

use anyhow::{Context as _, Error, Result};
use octocrab::{
    models,
    params::{pulls::Sort, Direction},
};
use serde::Deserialize;

#[macro_export]
macro_rules! client_request {
    ($client:expr, $method:ident) => {
        $client.inner.$method(&$client.owner, &$client.repo)
    };
}

pub struct Client {
    pub inner: octocrab::Octocrab,
    pub owner: String,
    pub repo: String,
}

impl Client {
    pub fn new(token: String, owner: String, repo: String) -> Result<Self> {
        let inner = octocrab::OctocrabBuilder::new()
            .personal_token(token)
            .build()
            .context("failed to create client")?;
        Ok(Self { inner, owner, repo })
    }

    pub fn new_from_env() -> Result<Self, Error> {
        let token = read_env("GITHUB_TOKEN")?;

        let repo = read_env("GITHUB_REPOSITORY")?;
        let (owner, name) = {
            let mut it = repo.split('/');
            (
                it.next()
                    .context("unable to determine repo owner")?
                    .to_string(),
                it.next()
                    .context("unable to determine repo name")?
                    .to_string(),
            )
        };

        Self::new(token, owner, name)
    }

    /// Get the currently open pull requests for the repo.
    ///
    /// Only the most recently pull requests are included as pagination is not
    /// handled. This is OK as we are not interested in outdated PRs as they
    /// won't have been updated since we last checked.
    pub async fn get_pull_requests(&self) -> Result<Vec<models::pulls::PullRequest>> {
        Ok(self
            .inner
            .pulls(&self.owner, &self.repo)
            .list()
            .state(octocrab::params::State::Open)
            .direction(Direction::Descending)
            .sort(Sort::Updated)
            .send()
            .await
            .context("unable to retrieve pull requests")?
            .items)
    }
}

fn read_env(var_name: &str) -> Result<String> {
    std::env::var(var_name).with_context(|| format!("failed to read input '{}'", var_name))
}

#[derive(Debug)]
pub struct ActionContext {
    pub event_name: String,
    pub payload: WebhookPayload,
    pub metadata: Metadata,
}

#[derive(Deserialize, Debug)]
pub struct Metadata {
    pub sha: Option<String>,
    pub organization: Option<models::orgs::Organization>,
    pub repository: Option<models::Repository>,
    pub sender: Option<models::User>,
}

#[derive(Deserialize, Debug)]
pub struct PullRequest {
    pub action: models::pulls::PullRequestAction,
    pub pull_request: models::pulls::PullRequest,
}

#[derive(Deserialize, Debug)]
pub struct PullRequestReview {
    pub action: models::pulls::PullRequestReviewAction,
    pub pull_request: models::pulls::PullRequest,
    pub review: models::pulls::Review,
}

#[derive(Deserialize, Debug)]
pub struct Commit {
    pub sha: String,
}

#[derive(Deserialize, Debug)]
pub struct Branch {
    pub name: String,
    pub protected: bool,
    pub commit: Commit,
}

#[derive(Deserialize, Debug)]
pub struct Status {
    pub state: models::StatusState,
    pub context: String,
    pub branches: Vec<Branch>,
}

#[derive(Debug)]
pub enum WebhookPayload {
    Status(Status),
    PullRequest(Box<PullRequest>),
    PullRequestReview(Box<PullRequestReview>),
}

/// Configuration options available for the action
#[derive(Debug)]
pub struct Config {
    /// The label added when a PR does not have a body
    pub needs_description_label: Option<String>,
    /// The list of statuss that are required to be passed for the PR to be
    /// automerged
    pub required_statuses: Vec<String>,
    /// The label applied when all of the PR's required status checks have passed
    pub ci_passed_label: String,
    /// Label applied when a PR has 1 or more reviewers and all of them are accepted
    pub reviewed_label: Option<String>,
    /// Label that can be manually added to PRs to block automerge
    pub block_merge_label: Option<String>,
    /// The period in milliseconds between when a PR can be automerged, and when
    /// the action actually tries to perform the merge
    pub automerge_grace_period: Option<u64>,
    /// The method to use for merging the PR, defaults to `merge` if we fail
    /// to parse or it is unset by the user
    pub merge_method: octocrab::params::pulls::MergeMethod,
}

impl Config {
    /// Deserializes the configuration from the environment variables set by
    /// the action runner. We use this option instead of command line arguments
    /// as it is much easier to manage overall, and also is the same way that
    /// the node actions work.
    pub fn deserialize() -> Result<Self, Error> {
        fn read_input(name: &str) -> Result<String, Error> {
            read_env(&format!("INPUT_{}", name.to_ascii_uppercase()))
        }

        fn to_vec(input: String) -> Vec<String> {
            input
                .split(',')
                .filter_map(|s| {
                    let ctx_name = s.trim();

                    if ctx_name.is_empty() {
                        None
                    } else {
                        Some(ctx_name.to_owned())
                    }
                })
                .collect()
        }

        Ok(Self {
            needs_description_label: read_input("needs_description_label")
                .ok()
                .filter(|label| !label.is_empty()),
            required_statuses: {
                let rs = read_input("required_statuses").map(to_vec)?;

                if rs.is_empty() {
                    anyhow::bail!("must supply 1 or more valid 'required_statuses'");
                } else {
                    rs
                }
            },
            ci_passed_label: read_input("ci_passed_label").and_then(|label| {
                if label.is_empty() {
                    anyhow::bail!("'ci_passed_label' is required to be a valid value");
                } else {
                    Ok(label)
                }
            })?,
            reviewed_label: read_input("reviewed_label")
                .ok()
                .filter(|label| !label.is_empty()),
            block_merge_label: read_input("block_merge_label")
                .ok()
                .filter(|label| !label.is_empty()),
            automerge_grace_period: read_input("automerge_grace_period")
                .and_then(|gp| {
                    if gp.is_empty() {
                        anyhow::bail!("ignoring empty string");
                    } else {
                        gp.parse().map_err(|e| {
                            log::error!("Failed to parse '{}': {}", gp, e);
                            anyhow::anyhow!("")
                        })
                    }
                })
                .ok(),
            merge_method: read_input("merge_method")
                .map(|mm| {
                    use octocrab::params::pulls::MergeMethod as MM;

                    match mm.as_str() {
                        "merge" => MM::Merge,
                        "squash" => MM::Squash,
                        "rebase" => MM::Rebase,
                        unknown => {
                            log::error!(
                                "Unknown merge_method '{}' specified, falling back to 'merge'",
                                unknown
                            );
                            MM::Merge
                        }
                    }
                })
                .unwrap_or(octocrab::params::pulls::MergeMethod::Merge),
        })
    }
}
