// See https://github.com/actions/toolkit/blob/main/packages/github/src/context.ts
// for more details

use anyhow::{Context as _, Error};
use octocrab::models;
use std::env::var;

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActionContext {
    payload: WebhookPayload,
    event_name: String,
}

#[derive(serde::Deserialize, Debug)]
pub enum WebhookPayload {
    Repository(models::Repository),
    Issue(models::issues::Issue),
    pub issue: Option<models::issues::Issue>,
    pub pull_request: Option<models::pulls::PullRequest>,
    pub sender: Option<models::User>,
    pub action: Option<String>,
    pub installation: Option<models::Installation>,
    pub comment: Option<models::issues::Comment>,
    //pub status: Option<models::st
}

#[derive(serde::Serialize)]
pub enum Payload {}

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

    let event_name = var("GITHUB_EVENT_NAME").context("failed to read GITHUB_EVENT_NAME")?;


}
