pub async fn queue(
    client: &crate::context::Client,
    pr: octocrab::models::pulls::PullRequest,
    config: &crate::process::Config,
) -> Result<(), anyhow::Error> {
    if let Some(gp) = config.automerge_grace_period {
        tokio::time::sleep(std::time::Duration::from_millis(gp)).await;
    }

    let pr_number = pr.number;
    let prh = crate::client_request!(client, pulls);

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

        if !crate::process::get_mergeable_state(pr.number, &labels, config) {
            log::info!(
                "PR #{} was mutated into an umergeable state after it was queued, aborting merge",
                pr_number,
            );
            return Ok(());
        }

        // Github started calculating the merge state of the PR if it hadn't
        // already done so before our request, so if it didn't finish, we need
        // to poll it again
        match pr.mergeable {
            Some(is_mergeable) => {
                if !is_mergeable {
                    log::warn!("Github indicates PR#{} is not mergeable", pr_number);
                    return Ok(());
                }

                let mut merge = prh
                    .merge(pr_number)
                    .title(pr.title)
                    .sha(pr.head.sha)
                    .method(
                        config
                            .merge_method
                            .unwrap_or(octocrab::params::pulls::MergeMethod::Merge),
                    );

                if let Some(body) = pr.body {
                    merge = merge.message(body);
                }

                match merge.send().await {
                    Ok(res) => {
                        // Even though the response contains a 'merged' boolean, the API docs
                        // seem to indicate that this would never be false, so we just assume it merged
                        log::info!(
                            "Successfully merged PR#{}: {}",
                            pr_number,
                            res.sha.unwrap_or_default()
                        );
                    }
                    Err(err) => {
                        log::info!("Failed to merge PR#{}: {:#}", pr_number, err);
                    }
                }

                return Ok(());
            }
            None => {
                log::info!("Merge state for PR#{} is unknown, retrying", pr_number);
            }
        }

        retry_count += 1;
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }

    Ok(())
}
