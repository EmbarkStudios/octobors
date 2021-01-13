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
