/// Queues the pull request for merging
pub async fn queue(
    client: &crate::context::Client,
    pr: octocrab::models::pulls::PullRequest,
    config: &crate::context::Config,
) -> Result<(), anyhow::Error> {
    // Wait some amount of time before actually attempting the merge if the
    // grace_period has been set
    if let Some(gp) = config.automerge_grace_period {
        tokio::time::sleep(std::time::Duration::from_millis(gp)).await;
    }

    let pr_number = pr.number;
    let prh = crate::client_request!(client, pulls);
    let ish = crate::client_request!(client, issues);

    let mut retry_count = 0u32;

    while retry_count < 3 {
        // See https://docs.github.com/en/free-pro-team@latest/rest/guides/getting-started-with-the-git-database-api#checking-mergeability-of-pull-requests
        // for why we rerequest the PR instead of using a small graphql query
        let pr = prh.get(pr.number).await?;

        let labels: Vec<_> = pr
            .labels
            .as_ref()
            .map(|labels| labels.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default();

        use octocrab::models::pulls::MergeableState;

        match pr.mergeable_state {
            Some(MergeableState::Unknown) | None => {
                // Github started calculating the merge state of the PR if it hadn't
                // already done so before our request, so if it didn't finish, we need
                // to poll it again
                log::warn!("Merge state for PR#{} is unknown, retrying", pr_number);

                retry_count += 1;
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;

                continue;
            }
            Some(ms) => {
                let abort_reason = match ms {
                    MergeableState::Draft => Some("PR is a draft and can't be merged".to_owned()),
                    MergeableState::Behind => Some(format!(
                        "PR branch '{}' is behind '{}' and needs to be updated",
                        pr.head.ref_field, pr.base.ref_field,
                    )),
                    MergeableState::Dirty => {
                        Some("Github is unable to create a merge commit for the PR".to_owned())
                    }
                    MergeableState::Blocked => {
                        Some("1 or more required checks are pending".to_owned())
                    }
                    // So Github might set the state as "unstable" since the automerge
                    // action is currently running, but if we got here then the CI
                    // statuses we actually cared about have all passed, so we "should"
                    // be ok
                    MergeableState::Clean | MergeableState::HasHooks | MergeableState::Unstable => {
                        let merge = prh
                            .merge(pr_number)
                            .title(format!("{} (#{})", pr.title, pr_number))
                            .sha(pr.head.sha)
                            .method(config.merge_method)
                            .message(match pr.body {
                                Some(body) => format!("{}\n\n{}", body, pr.url),
                                None => pr.url,
                            });

                        match merge.send().await {
                            Ok(res) => {
                                // Even though the response contains a 'merged' boolean, the API docs
                                // seem to indicate that this would never be false, so we just assume it merged
                                log::info!(
                                    "Successfully merged PR#{}: {}",
                                    pr_number,
                                    res.sha.unwrap_or_default()
                                );

                                None
                            }
                            Err(err) => Some(format!("Failed to merge PR: {:#}", err)),
                        }
                    }
                    MergeableState::Unknown => unreachable!(),
                    _ => {
                        log::warn!("Ignoring unknown merge state {:?} for PR#{}", ms, pr_number);
                        return Ok(());
                    }
                };

                if let Some(abort_reason) = abort_reason {
                    log::warn!(
                        "PR#{} was not able to automerge: {}",
                        pr_number,
                        abort_reason
                    );

                    // Depending on how fast events get processed this might
                    // end up commenting multiple times
                    let _ = ish
                        .create_comment(pr_number, format!("automerge aborted: {}", abort_reason))
                        .await;
                }
            }
        }

        return Ok(());
    }

    Ok(())
}
