use crate::{client_request, context};
use anyhow::{Context as _, Error};
use models::StatusState;
use octocrab::models;

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
    /// The method to use for merging to the PR
    pub merge_method: Option<octocrab::params::pulls::MergeMethod>,
}

impl Config {
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

async fn get_pull_requests(
    client: &context::Client,
    event: context::ActionContext,
    cfg: &Config,
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
    cfg: Config,
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

async fn process_pr(client: &context::Client, pr: PREvent, cfg: &Config) -> Result<(), Error> {
    let pr_number = pr.pr.number;

    // If the PR is a draft, we just mark it as waiting on the author and
    // abort any further processing, as marking a PR as a draft is the cleanest
    // and most easily recognizable way to indicate the PR should not be automerged
    if pr.pr.draft {
        log::info!(
            "PR #{} is marked as a draft, aborting any further processing",
            pr_number
        );
        return Ok(());
    }

    if pr.pr.state == models::IssueState::Closed {
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

    // Check the state of the description
    if let (Some(needs_description), Some(label)) =
        (merge_state.needs_description, &cfg.needs_description_label)
    {
        if needs_description {
            labels_to_add.push(label);
        } else {
            labels_to_remove.push(label);
        }
    }

    // Check the review state
    if let (Some(reviewed), Some(label)) = (merge_state.reviewed, &cfg.reviewed_label) {
        if reviewed {
            labels_to_add.push(label);
        } else {
            labels_to_remove.push(label);
        }
    }

    // Check the CI status
    if let Some(ci_passed) = merge_state.ci_passed {
        if ci_passed {
            labels_to_add.push(&cfg.ci_passed_label);
        } else {
            labels_to_remove.push(&cfg.ci_passed_label);
        }
    }

    add_labels(client, pr_number, &mut labels, &labels_to_add).await?;
    remove_labels(client, pr_number, &mut labels, &labels_to_remove).await?;

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

pub fn get_mergeable_state(pr_number: u64, labels: &[String], cfg: &Config) -> bool {
    if let Some(block_merge_label) = &cfg.block_merge_label {
        if has_label(labels, block_merge_label).is_some() {
            log::info!(
                "PR #{} has the '{}' label which blocks automerging",
                pr_number,
                block_merge_label,
            );

            return false;
        }
    }

    if let Some(needs_description_label) = &cfg.needs_description_label {
        if has_label(labels, needs_description_label).is_some() {
            log::info!(
                "PR #{} does not have a description, but one is required",
                pr_number
            );

            return false;
        }
    }

    if let Some(reviewed_label) = &cfg.reviewed_label {
        if has_label(labels, reviewed_label).is_none() {
            log::info!("PR #{} needs 1 or more review approvals", pr_number);

            return false;
        }
    }

    if has_label(labels, &cfg.ci_passed_label).is_none() {
        log::info!("PR #{} needs CI to pass", pr_number);
        return false;
    }

    true
}

#[inline]
fn has_label(labels: &[String], name: &str) -> Option<usize> {
    labels.iter().position(|label| label == name)
}

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

async fn on_pr_event(
    client: &context::Client,
    pr: &PREvent,
    cfg: &Config,
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

async fn on_review_state_event(
    client: &context::Client,
    pr: &PREvent,
    cfg: &Config,
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
    // reviewer, as every state changes is stored in chronological order
    if reviews.is_empty() {
        log::debug!("No reviews are available for PR#{}", pr_number);
        merge_state.reviewed = Some(false);
    } else {
        let mut review_states = std::collections::HashMap::new();

        for rev in &reviews {
            match rev.state {
                Some(models::pulls::ReviewState::Commented) | None => {
                    log::debug!("Ignoring comment from '{}'", rev.user.login);
                }
                Some(state) => {
                    review_states.insert(rev.user.id, (&rev.user, state));
                }
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

async fn on_status_event(
    client: &context::Client,
    pr: &PREvent,
    cfg: &Config,
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
