/// Removes HTML comments (in the form of <!-- comments -->) from the given string.
/// If running into nested comments, aborts and returns the initial string.
fn remove_html_comments(body: String) -> String {
    let mut result = String::new();
    let mut unfinished_comment = false;

    let mut haystack = body.as_str();
    while let Some(start) = haystack.find("<!--") {
        result += &haystack[0..start];
        if let Some(mut end) = haystack[start..].find("-->") {
            // Let end be relative to haystack[0..].
            end += start;
            if haystack[(start + "<!--".len())..end].contains("<!--") {
                // Embedded comments, abort!
                return body;
            }
            haystack = &haystack[(end + "-->".len())..];
        } else {
            // No end to this comment, skip the rest of this string.
            unfinished_comment = true;
            break;
        }
    }

    if !unfinished_comment {
        result += haystack;
    }

    // Whitespacing shenanigans:
    // - within a single line, make sure there aren't multiple consecutive whitespaces
    let lines = result
        .trim()
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>();

    // - overall, make sure that paragraph are split across at most two blank lines.
    let mut result = Vec::new();
    let mut prev_was_empty = false;
    for line in lines {
        if line.trim().is_empty() {
            if prev_was_empty {
                continue;
            }
            prev_was_empty = true;
        } else {
            prev_was_empty = false;
        }
        result.push(line);
    }
    result.join("\n")
}

fn format_commit_message(body: String, html_url: String) -> String {
    // Remove HTML comments from the body.
    let body = remove_html_comments(body);
    format!("{}\n\n{}", body, html_url)
}

/// Queues the pull request for merging
pub async fn queue(
    client: &crate::context::Client,
    pr: &crate::process::Pr,
    config: &crate::context::RepoConfig,
) -> Result<(), anyhow::Error> {
    let pr_number = pr.number;
    let prh = client.inner.pulls(&client.owner, &config.name);

    let mut retry_count = 0u32;

    while retry_count < 3 {
        // See https://docs.github.com/en/free-pro-team@latest/rest/guides/getting-started-with-the-git-database-api#checking-mergeability-of-pull-requests
        // for why we rerequest the PR instead of using a small graphql query
        let pr = prh.get(pr.number).await?;

        use octocrab::models::pulls::MergeableState;

        match pr.mergeable_state {
            Some(MergeableState::Unknown) | None => {
                // Github started calculating the merge state of the PR if it hadn't
                // already done so before our request, so if it didn't finish, we need
                // to poll it again
                tracing::warn!("Merge state is unknown, retrying");

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
                                Some(body) => format_commit_message(body, pr.html_url.to_string()),
                                None => pr.html_url.to_string(),
                            });

                        match merge.send().await {
                            Ok(res) => {
                                // Even though the response contains a 'merged' boolean, the API docs
                                // seem to indicate that this would never be false, so we just assume it merged
                                tracing::info!(
                                    "Successfully merged: {}",
                                    res.sha.unwrap_or_default()
                                );

                                None
                            }
                            Err(err) => Some(format!("Failed to merge PR: {:#}", err)),
                        }
                    }
                    MergeableState::Unknown => unreachable!(),
                    _ => {
                        tracing::warn!("Ignoring unknown merge state {:?}", ms);
                        return Ok(());
                    }
                };

                if let Some(abort_reason) = abort_reason {
                    tracing::warn!("not able to automerge: {}", abort_reason);
                }
            }
        }

        return Ok(());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn remove_comments() {
        use super::remove_html_comments;

        assert_eq!(
            remove_html_comments(
                "Hello <!-- and very surprisingly this is followed by --> world!".to_owned()
            ),
            "Hello world!"
        );

        assert_eq!(
            remove_html_comments("Hello <!-- no end so it gets all removed".to_owned()),
            "Hello"
        );

        assert_eq!(
            remove_html_comments("This <!-- is <!-- a known limitation --> -->".to_owned()),
            "This <!-- is <!-- a known limitation --> -->"
        );

        assert_eq!(
            remove_html_comments(
                r#" A <!-- ignore this --> meaningful <!-- ignore that too --> merge comment <!-- yo yo yo --> <!-- yo yo yo -->. "#
                .to_owned()
            ),
            "A meaningful merge comment .",
        );

        // Preserve paragraph structure, even in the presence of paragraphs.
        assert_eq!(
            remove_html_comments(
                r#"<!-- start with a comment that's going to be removed --> A multi-line message.

<!-- with a comment in the middle -->

Overall paragraph structure should be preserved.<!--

and end with a comment

-->"#
                    .to_owned()
            ),
            r#"A multi-line message.

Overall paragraph structure should be preserved."#
        );

        // Maintain paragraph structure, but remove unneeded blank lines.
        assert_eq!(
            remove_html_comments(
                r#"A multi-line message.



But without comments.

Paragraph structure should be preserved."#
                    .to_owned()
            ),
            r#"A multi-line message.

But without comments.

Paragraph structure should be preserved."#
        );
    }
}
