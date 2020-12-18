use octocrab::models;
use anyhow::Error;

#[derive(Debug)]
pub struct Config {
    /// Whether reviews are required for the PR to be automerged
    pub requires_review: bool,
    /// The list of checks that are required to be passed for the PR to be
    /// automerged
    pub required_checks: Vec<String>,
    /// The list of labels that are applied when all of the required checks
    /// have passed on the PR
    pub checks_passed_labels: Vec<String>,
    /// The list of labels that are applied when a PR is waiting on one or more
    /// reviewers
    pub waiting_for_review_labels: Vec<String>,
    /// The list of labels that are applied when the PR is deemed ready for
    /// merging to the trunk
    pub ready_for_merge_labels: Vec<String>,
    /// The list of labels that are applied when the PR needs further action
    /// from the author to advance
    pub waiting_for_author_labels: Vec<String>,
}

struct PRState {
    pr: models::pulls::PullRequest,
    status: Option<models::StatusState>,
}

async fn get_pull_requests(client: &octocrab::Octocrab, event: &crate::context::ActionContext, cfg: &Config) -> Result<Vec<PrState>, Error> {
    // Status events are different from most other pull request related events in that they
    // are delivered to the repo, not the individual pr, so we have to figure out which PR(s)
    // it actually applies to
    if event.event_name == "status" {
        even.st
    } else {
        vec![event.
    }
}

async pub fn process_event(client: octocrab::Octocrab, event: crate::context::ActionContext, cfg: Config) -> Result<(), Error> {
    
    let prs_to_check = get_pull_requests(&client, &event, &cfg).await?;
}