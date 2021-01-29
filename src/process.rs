use std::collections::{HashMap, HashSet};

use crate::{client_request, context};
use anyhow::{Context as _, Error, Result};
use chrono::{DateTime, Duration, Utc};
use models::{
    pulls::{PullRequest, ReviewState},
    IssueState, StatusState,
};
use octocrab::models;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct PR {
    pub id: u64,
    pub number: u64,
    pub commit_sha: String,
    draft: bool,
    state: models::IssueState,
    updated_at: DateTime<Utc>,
    labels: HashSet<String>,
    has_description: bool,
}

impl PR {
    pub fn from_octocrab_pull_request(pr: PullRequest) -> Self {
        let labels = pr
            .labels
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.name)
            .collect();
        Self {
            id: pr.id,
            number: pr.number,
            commit_sha: pr.head.sha,
            draft: pr.draft,
            state: pr.state,
            updated_at: pr.updated_at.unwrap_or(pr.created_at),
            has_description: pr.body.unwrap_or_default() != "",
            labels,
        }
    }
}

pub struct Analyzer<'a> {
    pr: &'a PR,
    client: &'a context::Client,
    config: &'a context::Config,
    // We optionally keep a local version of these fields using `RemoteData`
    // so we can pre-set the data with values in order to not hit the GitHub
    // API in unit tests
    reviews: RemoteData<Vec<Review>>,
    statuses: RemoteData<HashMap<String, StatusState>>,
}

impl<'a> Analyzer<'a> {
    pub fn new(pr: &'a PR, client: &'a context::Client, config: &'a context::Config) -> Self {
        Self {
            pr,
            client,
            config,
            reviews: RemoteData::Remote,
            statuses: RemoteData::Remote,
        }
    }

    /// Analyze a PR to determine what actions need to be undertaken.
    pub async fn required_actions(&self) -> Result<Actions> {
        let pr = &self.pr;
        let mut actions = Actions::noop();

        if pr.draft {
            log::info!("PR #{}: Draft, nothing to do", pr.number);
            return Ok(actions);
        }

        if pr.state == IssueState::Closed {
            log::info!("PR #{}: Closed, nothing to do", pr.number);
            return Ok(actions);
        }

        if pr.updated_at < Utc::now() - Duration::minutes(60) {
            log::info!("PR #{}: Inactive for over 1 hour, nothing to do", pr.number);
            return Ok(actions);
        }

        // Now that the basic checks have been passed we can gather information
        // from the GitHub API in order to do the full check. We do this second
        // so that we use the GitHub API as little as possible, we don't want to
        // hit the rate limit.
        let statuses_passed = self.pr_statuses_passed().await?;
        let pr_approved = self.pr_approved().await?;
        let mut description_ok = true;

        // Assign the "reviewed" label if there is one and the PR is approved
        if let Some(label) = &self.config.reviewed_label {
            actions.set_label(label, Presence::should_be_present(pr_approved));
        }

        // Assign the "needs-description" label if there is one and the PR lacks one
        if let Some(label) = &self.config.needs_description_label {
            description_ok = self.pr.has_description;
            actions.set_label(label, Presence::should_be_present(!self.pr.has_description));
        }

        // Assign the "ci-passed" label if CI passed
        actions.set_label(
            &self.config.ci_passed_label,
            Presence::should_be_present(statuses_passed),
        );

        actions.set_merge(
            !self.block_merge_label_applied() && description_ok && pr_approved && statuses_passed,
        );

        return Ok(actions);
    }

    async fn pr_approved(&self) -> Result<bool> {
        let review_not_required = self.config.reviewed_label.is_none();
        let mut waiting = false;
        let mut approved = review_not_required;
        let latest_reviews_per_person = self
            .get_pr_reviews()
            .await?
            .into_iter()
            .map(|review| (review.user_id, review.state))
            .collect::<HashMap<_, _>>();
        for review in latest_reviews_per_person.values() {
            match review {
                ReviewState::Approved => approved = true,
                ReviewState::Pending | ReviewState::ChangesRequested => waiting = true,
                _ => (),
            }
        }
        Ok(approved && !waiting)
    }

    async fn pr_statuses_passed(&self) -> Result<bool> {
        let statuses = self.get_pr_statuses().await?;
        for required in &self.config.required_statuses {
            if statuses.get(required) != Some(&StatusState::Success) {
                return Ok(false);
            }
        }
        return Ok(true);
    }

    fn block_merge_label_applied(&self) -> bool {
        match &self.config.block_merge_label {
            None => false,
            Some(label) => self.pr.labels.contains(label),
        }
    }

    async fn get_pr_reviews(&self) -> Result<Vec<Review>> {
        match &self.reviews {
            RemoteData::Local(reviews) => Ok(reviews.clone()),
            RemoteData::Remote => Ok(client_request!(self.client, pulls)
                .list_reviews(self.pr.number)
                .await
                .context("Could not get reviews for PR")?
                .items
                .iter()
                .flat_map(Review::from_octocrab_review)
                .collect()),
        }
    }

    async fn get_pr_statuses(&self) -> Result<HashMap<String, StatusState>> {
        match &self.statuses {
            RemoteData::Local(statuses) => Ok(statuses.clone()),
            RemoteData::Remote => Ok(client_request!(self.client, repos)
                .combined_status_for_ref(&octocrab::params::repos::Reference::Commit(
                    self.pr.commit_sha.clone(),
                ))
                .await
                .context("Could not get statuses for commit")?
                .statuses
                .into_iter()
                .flat_map(|status| Some((status.context?, status.state)))
                .collect()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Review {
    user_id: i64,
    state: ReviewState,
}

impl Review {
    fn from_octocrab_review(review: &octocrab::models::pulls::Review) -> Option<Self> {
        Some(Self {
            user_id: review.user.id,
            state: review.state?,
        })
    }
}

pub enum RemoteData<T> {
    Remote,
    Local(T),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Presence {
    Present,
    Absent,
}

impl Presence {
    fn should_be_present(should: bool) -> Self {
        if should {
            Self::Present
        } else {
            Self::Absent
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Actions {
    merge: bool,
    add_labels: HashSet<String>,
    remove_labels: HashSet<String>,
}

impl Actions {
    pub fn noop() -> Self {
        Self::default()
    }

    pub fn set_label(&mut self, label: &str, precence: Presence) -> &mut Self {
        match precence {
            Presence::Present => self.add_labels.insert(label.to_string()),
            Presence::Absent => self.remove_labels.insert(label.to_string()),
        };
        self
    }

    pub fn set_merge(&mut self, should_merge: bool) -> &mut Self {
        self.merge = should_merge;
        self
    }
}

enum EventTrigger {
    /// Something changed about the PR itself
    PullRequest {
        action: models::pulls::PullRequestAction,
    },
    /// Something changed with a review on a PR
    Review {
        action: models::pulls::PullRequestReviewAction,
        review: Box<models::pulls::Review>,
    },
    /// The status of a check was changed
    Status {
        context: String,
        state: models::StatusState,
    },
}

struct PREvent {
    pr: models::pulls::PullRequest,
    trigger: EventTrigger,
}

/// Gets one or more pull requests that the event we are processing actually
/// applies to
async fn get_pull_requests(
    client: &context::Client,
    event: context::ActionContext,
    cfg: &context::Config,
) -> Result<Option<Vec<PREvent>>, Error> {
    match event.payload {
        context::WebhookPayload::PullRequest(pr) => Ok(Some(vec![PREvent {
            pr: pr.pull_request,
            trigger: EventTrigger::PullRequest { action: pr.action },
        }])),
        context::WebhookPayload::PullRequestReview(prr) => Ok(Some(vec![PREvent {
            pr: prr.pull_request,
            trigger: EventTrigger::Review {
                action: prr.action,
                review: Box::new(prr.review),
            },
        }])),
        // Status events are different from most other pull request related events in that they
        // are delivered to the repo, not the individual pr, so we have to figure out which PR(s)
        // the status event actually applies to
        context::WebhookPayload::Status(status) => {
            if cfg.required_statuses.contains(&status.context) {
                let prh = client_request!(client, pulls);

                let mut prs = Vec::new();

                for branch in &status.branches {
                    let branch_prs = prh
                        .list()
                        .state(octocrab::params::State::Open)
                        .head(format!("{}:{}", client.owner, branch.name))
                        .send()
                        .await
                        .with_context(|| {
                            format!(
                                "unable to retrieve pull requests for branch '{}'",
                                branch.name
                            )
                        })?;

                    prs.extend(branch_prs.into_iter().map(|pr| PREvent {
                        pr,
                        trigger: EventTrigger::Status {
                            context: status.context.clone(),
                            state: status.state,
                        },
                    }));
                }

                Ok(Some(prs))
            } else {
                log::info!(
                    "ignoring status for '{}' as it is not a required check",
                    status.context
                );
                Ok(None)
            }
        }
    }
}

pub async fn process_event(
    client: context::Client,
    event: crate::context::ActionContext,
    cfg: context::Config,
) -> Result<(), Error> {
    let prs_to_check = get_pull_requests(&client, event, &cfg).await?;

    match prs_to_check {
        Some(prs) => {
            // We _could_ do this in parallel, but the number of PRs is going to
            // be 1 in 99% of cases, and it's just not worth it. Especially when
            // standard github runners are just 2 cores anyways
            for pr_state in prs {
                process_pr(&client, pr_state, &cfg).await?;
            }
        }
        None => {
            log::info!("no pull requests were available to process");
        }
    }

    Ok(())
}

struct MergeState {
    needs_description: Option<bool>,
    ci_passed: Option<bool>,
    reviewed: Option<bool>,
}

async fn process_pr(
    client: &context::Client,
    pr: PREvent,
    cfg: &context::Config,
) -> Result<(), Error> {
    let pr_number = pr.pr.number;

    // Ignore draft PRs altogether, marking a PR as a draft is the easiest
    // way for the author to communicate they don't want their PR automerged
    if pr.pr.draft {
        log::info!(
            "PR #{} is marked as a draft, aborting any further processing",
            pr_number
        );
        return Ok(());
    }

    if pr.pr.state == models::IssueState::Closed {
        log::info!(
            "PR #{} is closed, aborting any further processing",
            pr_number
        );
        return Ok(());
    }

    let mut merge_state = MergeState {
        needs_description: None,
        ci_passed: None,
        reviewed: None,
    };

    match &pr.trigger {
        EventTrigger::PullRequest { action } => {
            on_pr_event(client, &pr, cfg, &mut merge_state, *action).await?;
        }
        EventTrigger::Review { action, review } => {
            on_review_state_event(client, &pr, cfg, &mut merge_state, Some((*action, review)))
                .await?;
        }
        EventTrigger::Status { context, state } => {
            on_status_event(client, &pr, cfg, &mut merge_state, context, *state).await?;
        }
    }

    let mut labels: Vec<_> = pr
        .pr
        .labels
        .as_ref()
        .map(|labels| labels.iter().map(|l| l.name.clone()).collect())
        .unwrap_or_default();

    let mut labels_to_add = Vec::new();
    let mut labels_to_remove = Vec::new();

    if let (Some(needs_description), Some(label)) =
        (merge_state.needs_description, &cfg.needs_description_label)
    {
        if needs_description {
            labels_to_add.push(label);
        } else {
            labels_to_remove.push(label);
        }
    }

    if let (Some(reviewed), Some(label)) = (merge_state.reviewed, &cfg.reviewed_label) {
        if reviewed {
            labels_to_add.push(label);
        } else {
            labels_to_remove.push(label);
        }
    }

    if let Some(ci_passed) = merge_state.ci_passed {
        if ci_passed {
            labels_to_add.push(&cfg.ci_passed_label);
        } else {
            labels_to_remove.push(&cfg.ci_passed_label);
        }
    }

    add_labels(client, pr_number, &mut labels, &labels_to_add).await?;
    remove_labels(client, pr_number, &mut labels, &labels_to_remove).await?;

    // We explicitly ignore beginning an automerge from a status event due to how
    // status events work differently from most other events. Status (and check_run)
    // events are delivered to the repo/default branch, _not_ the PR, so if a user
    // is watching their PR they won't see the action running before their PR is
    // automerged, it will just seem to happen out of nowhere. So instead we rely
    // on the action running with the `pull_request.labeled` event so the status
    // event can set the `ci_passed` label, then the PR action will run and possibly
    // automerge the PR and just give better visibility to the user. As well
    // as make the sequence of events easier to see in the actions UI.
    if matches!(pr.trigger, EventTrigger::Status { .. }) {
        log::info!(
            "PR#{} was a status update, appropriate labels have been added or removed",
            pr_number
        );
    } else if get_mergeable_state(pr_number, &labels, &cfg) {
        log::warn!(
            "PR #{} has met all automerge requirements, queuing for merge...",
            pr_number
        );
        crate::merge::queue(&client, pr.pr, &cfg).await?;
    }

    Ok(())
}

/// Determines whether the PR is automergeable based on the current set of labels.
/// Prints out a warning for any of the automerge conditions that aren't met.
pub fn get_mergeable_state(pr_number: u64, labels: &[String], cfg: &context::Config) -> bool {
    if let Some(block_merge_label) = &cfg.block_merge_label {
        if has_label(labels, block_merge_label).is_some() {
            log::warn!(
                "PR #{} has the '{}' label which blocks automerging",
                pr_number,
                block_merge_label,
            );

            return false;
        }
    }

    if let Some(needs_description_label) = &cfg.needs_description_label {
        if has_label(labels, needs_description_label).is_some() {
            log::warn!(
                "PR #{} does not have a description, but one is required",
                pr_number
            );
            return false;
        }
    }

    if let Some(reviewed_label) = &cfg.reviewed_label {
        if has_label(labels, reviewed_label).is_none() {
            log::warn!("PR #{} needs 1 or more review approvals", pr_number);
            return false;
        }
    }

    if has_label(labels, &cfg.ci_passed_label).is_none() {
        log::warn!("PR #{} needs CI to pass", pr_number);
        return false;
    }

    true
}

#[inline]
fn has_label(labels: &[String], name: &str) -> Option<usize> {
    labels.iter().position(|label| label == name)
}

/// Adds one or more labels to the PR. Only adds labels that aren't already present.
async fn add_labels(
    client: &context::Client,
    pr_number: u64,
    labels: &mut Vec<String>,
    to_add: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(), Error> {
    // Only add the label(s) that are not actually present
    let to_add: Vec<_> = to_add
        .into_iter()
        .filter_map(|new_label| match has_label(labels, new_label.as_ref()) {
            None => Some(new_label.as_ref().to_owned()),
            _ => None,
        })
        .collect();

    if to_add.is_empty() {
        return Ok(());
    }

    log::debug!("Adding labels {:?}", to_add);

    let ih = client_request!(client, issues);

    ih.add_labels(pr_number, &to_add)
        .await
        .with_context(|| format!("failed to add label(s): {:?}", to_add))?;

    for new_label in to_add {
        labels.push(new_label);
    }

    Ok(())
}

/// Removes one or more labels from the PR. Only removes labels that are actually
/// on the PR.
async fn remove_labels(
    client: &context::Client,
    pr_number: u64,
    labels: &mut Vec<String>,
    to_remove: impl IntoIterator<Item = impl AsRef<str> + std::fmt::Debug>,
) -> Result<(), Error> {
    // Only remove the label(s) that are actually present
    let to_remove: Vec<_> = to_remove
        .into_iter()
        .filter_map(|old_label| {
            has_label(&labels, old_label.as_ref()).map(|i| {
                labels.remove(i);
                old_label
            })
        })
        .collect();

    if to_remove.is_empty() {
        return Ok(());
    }

    log::debug!("Removing labels {:?}", to_remove);

    let ih = client_request!(client, issues);

    for old_label in to_remove {
        if let Err(e) = ih.remove_label(pr_number, old_label.as_ref()).await {
            log::debug!("Error removing label '{}': {:#}", old_label.as_ref(), e);
        }
    }

    Ok(())
}

/// Changes the state of `needs_description`, `ci_passed`, or `reviewed` depending
/// the particular PR action that occurred
async fn on_pr_event(
    client: &context::Client,
    pr: &PREvent,
    cfg: &context::Config,
    merge_state: &mut MergeState,
    action: models::pulls::PullRequestAction,
) -> Result<(), Error> {
    use models::pulls::PullRequestAction as PRAction;
    let pr_number = pr.pr.number;

    #[allow(clippy::wildcard_in_or_patterns)]
    match action {
        // Event that can change the waiting for author label if a
        // description is required and it is added/removed/present
        PRAction::Opened | PRAction::Reopened | PRAction::Edited => {
            if cfg.needs_description_label.is_none() {
                merge_state.needs_description = Some(false);
            } else {
                merge_state.needs_description = Some(
                    pr.pr
                        .body
                        .as_ref()
                        .map(|body| body.is_empty())
                        .unwrap_or(true),
                );
            }
        }
        PRAction::Closed => {
            log::info!(
                "PR #{} was closed, aborting any further processing",
                pr_number
            );
        }
        // Event triggered when the user pushes commits to the PR branch,
        // so we remove any CI passed labels, as CI will need to run
        // again, and will set those labels once it is finished
        PRAction::Synchronize => {
            log::info!(
                "PR #{} was synchronized, marking CI as not passed",
                pr_number
            );

            merge_state.ci_passed = Some(false);
        }
        // Review state has changed
        PRAction::ReviewRequested | PRAction::ReviewRequestRemoved | PRAction::ReadyForReview => {
            on_review_state_event(client, pr, cfg, merge_state, None).await?;
        }
        PRAction::Labeled | PRAction::Unlabeled => {
            log::info!("PR #{} had its labels changed", pr_number);
        }
        // Events which have no bearing on whether the PR will be automerged
        PRAction::Assigned | PRAction::Unassigned | PRAction::Locked | PRAction::Unlocked | _ => {
            log::info!(
                "PR #{} action '{:?}' has no bearing on automerge",
                pr_number,
                action,
            );
        }
    }

    Ok(())
}

/// Changes the state of `reviewed` based on the the state of every requested
/// review
async fn on_review_state_event(
    client: &context::Client,
    pr: &PREvent,
    cfg: &context::Config,
    merge_state: &mut MergeState,
    review: Option<(
        models::pulls::PullRequestReviewAction,
        &models::pulls::Review,
    )>,
) -> Result<(), Error> {
    let pr_number = pr.pr.number;

    // If reviews aren't required, then uhh, yah, we're done here
    if cfg.reviewed_label.is_none() {
        log::info!("PR #{} does not require reviews", pr_number,);
        return Ok(());
    }

    // First we check if there are any pending reviewers, as then
    // we can skip any potential additional queries since it's not
    // possible for the review state to be approved
    if !pr.pr.requested_reviewers.is_empty() {
        log::info!(
            "PR #{} still has '{}' pending review(s)",
            pr_number,
            pr.pr.requested_reviewers.len()
        );
        return Ok(());
    }

    if let Some((action, review)) = review {
        match action {
            // If a review is submitted and it's requesting changes, we can
            // early out since we know that all reviews are not approved
            models::pulls::PullRequestReviewAction::Submitted
                if review.state == Some(models::pulls::ReviewState::ChangesRequested) =>
            {
                log::info!(
                    "PR #{} reviewer '{}' requested changes",
                    pr_number,
                    review.user.login
                );
                return Ok(());
            }
            // Anyone, including the PR author, can do drive by review
            // comments, but these never affect the actual automerge state
            models::pulls::PullRequestReviewAction::Submitted
                if review.state == Some(models::pulls::ReviewState::Commented) =>
            {
                log::info!(
                    "PR #{} was commented on by '{}', ignoring",
                    pr_number,
                    review.user.login
                );
                return Ok(());
            }
            // Otherwise we do a query of all of the reviews
            _ => {}
        }
    }

    log::debug!("Checking state of all review for PR#{}", pr_number);

    let ph = client_request!(client, pulls);

    let reviews = ph.list_reviews(pr_number).await?;

    // We need to keep track of the last dis/approve state for each individual
    // reviewer, as every state change is stored in chronological order, and
    // the reviewer might have approved then requested changes, then approved
    // again
    if reviews.items.is_empty() {
        log::debug!("No reviews are available for PR#{}", pr_number);
        merge_state.reviewed = Some(false);
    } else {
        let mut review_states = std::collections::HashMap::new();

        let mut insert_review = |rev: models::pulls::Review| match rev.state {
            Some(models::pulls::ReviewState::Commented) | None => {
                log::debug!("Ignoring comment from '{}'", rev.user.login);
            }
            Some(state) => {
                review_states.insert(rev.user.id, (rev.user, state));
            }
        };

        for rev in reviews.items {
            insert_review(rev);
        }

        while let Some(page) = client
            .inner
            .get_page::<models::pulls::Review>(&reviews.next)
            .await?
        {
            for rev in page {
                insert_review(rev);
            }
        }

        let all_reviews_approved = review_states.into_iter().all(|(_, (user, rev))| {
            let is_approved = rev == models::pulls::ReviewState::Approved;

            if !is_approved {
                log::debug!("Latest review from user '{}' is '{:?}'", user.login, rev,);
            }

            is_approved
        });

        merge_state.reviewed = Some(all_reviews_approved);
    }

    Ok(())
}

/// Changes the value of `ci_passed` depending on the state of all of the
/// required checks on the PR
async fn on_status_event(
    client: &context::Client,
    pr: &PREvent,
    cfg: &context::Config,
    merge_state: &mut MergeState,
    context: &str,
    state: models::StatusState,
) -> Result<(), Error> {
    // If only a single check is required, we can just use the state
    // directly, however if multiple are required, we'll need to do
    // an additional query for _all_ of the checks, as there is no
    // way to store our own state on a PR
    if cfg.required_statuses.len() == 1 && context == cfg.required_statuses[0] {
        #[allow(clippy::wildcard_in_or_patterns)]
        match state {
            StatusState::Success => {
                merge_state.ci_passed = Some(true);
            }
            StatusState::Error | StatusState::Failure | StatusState::Pending | _ => {
                merge_state.ci_passed = Some(false);
            }
        }
    } else {
        let rh = client_request!(client, repos);
        let status = rh
            .combined_status_for_ref(&octocrab::params::repos::Reference::Commit(
                pr.pr.head.sha.clone(),
            ))
            .await?;

        let all_checks_passed = cfg.required_statuses.iter().all(|rc| {
            match status
                .statuses
                .iter()
                .find(|stat| stat.context.as_ref() == Some(rc))
            {
                Some(status) => status.state == StatusState::Success,
                None => false,
            }
        });

        merge_state.ci_passed = Some(all_checks_passed);
    }

    Ok(())
}
