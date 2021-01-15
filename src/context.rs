// See https://github.com/actions/toolkit/blob/main/packages/github/src/context.ts
// for more details

use anyhow::{Context as _, Error};
use octocrab::models;
use serde::Deserialize;
use std::env::var;

pub struct Client {
    pub inner: octocrab::Octocrab,
    pub owner: String,
    pub repo: String,
}

impl Client {
    pub fn new(octo: octocrab::Octocrab) -> Result<Self, Error> {
        let repo_name =
            std::env::var("GITHUB_REPOSITORY").context("unable to determine repository")?;
        let (owner, name) = {
            let mut it = repo_name.split('/');
            (
                it.next().context("unable to determine repo owner")?,
                it.next().context("unable to determine repo name")?,
            )
        };

        Ok(Self {
            inner: octo,
            owner: owner.to_owned(),
            repo: name.to_owned(),
        })
    }
}

#[macro_export]
macro_rules! client_request {
    ($client:expr, $method:ident) => {
        $client.inner.$method(&$client.owner, &$client.repo)
    };
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

pub fn deserialize_action_context(
    path: Option<impl AsRef<std::path::Path>>,
) -> Result<ActionContext, Error> {
    let path = match path {
        Some(path) => path.as_ref().to_owned(),
        None => var("GITHUB_EVENT_PATH")
            .context("unable to read GITHUB_EVENT_PATH")?
            .into(),
    };

    let event_data = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "Unable to read Github action event from '{}'",
            path.display()
        )
    })?;

    log::debug!("Event data: {}", event_data);

    let event_name = var("GITHUB_EVENT_NAME").context("failed to read GITHUB_EVENT_NAME")?;

    log::debug!("Action triggered by '{}' event", event_name);

    let payload: WebhookPayload = match event_name.as_str() {
        "pull_request" => serde_json::from_str::<PullRequest>(&event_data)
            .map(|pr| WebhookPayload::PullRequest(Box::new(pr))),
        "pull_request_review" => serde_json::from_str::<PullRequestReview>(&event_data)
            .map(|prr| WebhookPayload::PullRequestReview(Box::new(prr))),
        "status" => serde_json::from_str::<Status>(&event_data).map(WebhookPayload::Status),
        unknown => {
            anyhow::bail!("ignoring event '{}'", unknown);
        }
    }?;

    let metadata = serde_json::from_str(&event_data)?;

    Ok(ActionContext {
        event_name,
        payload,
        metadata,
    })
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
    pub merge_method: Option<octocrab::params::pulls::MergeMethod>,
}

impl Config {
    /// Deserializes the configuration from the environment variables set by
    /// the action runner. We use this option instead of command line arguments
    /// as it is much easier to manage overall, and also is the same way that
    /// the node actions work.
    pub fn deserialize() -> Result<Self, Error> {
        fn read_input(name: &str) -> Result<String, Error> {
            std::env::var(&format!("INPUT_{}", name.to_ascii_uppercase()))
                .with_context(|| format!("failed to read input '{}'", name))
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
            needs_description_label: read_input("needs-description-label").ok().filter(|label| !label.is_empty()),
            required_statuses: {
                let rs = read_input("required-statuses").map(to_vec)?;

                if rs.is_empty() {
                    anyhow::bail!("must supply 1 or more valid 'required-statuses'");
                } else {
                    rs
                }
            },
            ci_passed_label: read_input("ci-passed-label").and_then(|label| {
                if label.is_empty() {
                    anyhow::bail!("'ci-passed-label' is required to be a valid value");
                } else {
                    Ok(label)
                }
            })?,
            reviewed_label: read_input("reviewed-label").ok().filter(|label| !label.is_empty()),
            block_merge_label: read_input("block-merge-label").ok().filter(|label| !label.is_empty()),
            automerge_grace_period: read_input("automerge-grace-period")
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
            merge_method: read_input("merge-method").and_then(|mm| {
                use octocrab::params::pulls::MergeMethod as MM;

                match mm.as_str() {
                    "merge" => Ok(MM::Merge),
                    "squash" => Ok(MM::Squash),
                    "rebase" => Ok(MM::Rebase),
                    unknown => {
                        log::error!("Unknown merge_method '{}' specified, falling back to default of 'merge'", unknown);
                        Err(anyhow::anyhow!(""))
                    }
                }
            }).ok(),
        })
    }
}
