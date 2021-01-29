# 🆙 octobors

[![Embark](https://img.shields.io/badge/embark-open%20source-blueviolet.svg)](https://embark.dev)
[![Embark](https://img.shields.io/badge/discord-ark-%237289da.svg?logo=discord)](https://discord.gg/dAuKfZS)
[![dependency status](https://deps.rs/repo/github/EmbarkStudios/octobors/status.svg)](https://deps.rs/repo/github/EmbarkStudios/octobors)
[![Build status](https://github.com/EmbarkStudios/octobors/workflows/CI/badge.svg)](https://github.com/EmbarkStudios/octobors/actions)

GitHub action for automerging PRs based on a few rules.

## Why?

We made our own automerge bot because we couldn't find an existing solution that did exactly
what we wanted.

1. [Github's auto-merge](https://docs.github.com/en/free-pro-team@latest/github/collaborating-with-issues-and-pull-requests/automatically-merging-a-pull-request) relies on branch protection rules, some of which we think are kind of broken (required review count), and some of which we don't want to use for other reasons. You also still have to click a button! The horror!
1. [Mergify](https://mergify.io/) - External service that we don't want to use for our private repos
1. [automerge-action](https://github.com/pascalgn/automerge-action) - Only provided part of the functionality we wanted, and still relies too heavily on branch protections.

## Setup

### Example

```yaml
name: automerge
# See below for why each of these events is needed
on:
  pull_request:
    types:
      - synchronize
      - opened
      - reopened
      - edited
      - unlabeled
      - labeled
      - ready_for_review
      - review_request_removed
      - review_requested
  pull_request_review:
    types:
      - submitted
  status: {}
jobs:
  automerge:
    # This action is a docker container action, so it only runs on Linux
    runs-on: ubuntu-latest
    steps:
      - name: automerge
        uses: "EmbarkStudios/octobors@0.3.0"
        with:
          github-token: "${{ secrets.BOT_TOKEN }}"
          merge-method: squash
          needs-description-label: "s: needs description"
          required-statuses: "awesome-test"
          ci-passed-label: "s: ci passed ✔️"
          reviewed-label: "s: reviewed 🔬"
          block-merge-label: "s: block automerge"
          automerge-grace-period: 10000
```

### Inputs

#### `github-token` **required**

The token to use for all API requests made by the action. It's recommended to
**NOT** use the default `GITHUB_TOKEN` for your repo, but rather a PAT from
a Github account. See [`labeled`](#labeled) for more info.

#### `merge-method`

The method to use when merging to the PR to the base branch, one of:

* `merge` **default**
* `squash`
* `rebase`

#### `needs-description-label`

Setting this input means you require every PR to have a description (body) for
it to be merged.

#### `required-statuses` **required**

List of `,` separated status checks that must be passing for the PR to be
automerged.

#### `ci-passed-label` (default `s: ci-passed`)

Label added to PRs once all of the [required](#required-statuses) statuses have
passed.

#### `reviewed-label`

Label added to PRs once all requested reviews have been approved, requiring at
least 1 review. If not specified, reviews are not required for automerging.

#### `block-merge-label`

Label that can be manually applied to PRs so that they can't be automerged. You
can also stop automerging by marking a PR as a draft.

#### `automerge-grace-period`

Grace period (in seconds) from when a PR has entered into an automergeable
state, and when a merge is first attempted.

### Events

#### [pull_request](https://docs.github.com/en/free-pro-team@latest/actions/reference/events-that-trigger-workflows#pull_request)

##### `synchronize`

Event triggered when new commits are pushed to the PR branch, we use this to
immediately remove the `ci-passed-label` since any CI will need to be rerun on
the new commit(s).

##### `opened`, `reopened`, and `edited`

Only needed if `needs-description-label`, these events just inform the action
to check the state of the PR description.

##### `labeled`

This event is needed as a UX enhancement. The [`status`](https://docs.github.com/en/free-pro-team@latest/actions/reference/events-that-trigger-workflows#status) event
is only delivered to the repo itself, not a specific PR, so the Actions UI
becomes extremely confusing the more status checks you have since you can't tell
immediately what PR they pertain to. So instead, when all the required status
checks have passed, this action only adds the `ci-passed-label` to the PR, which
in turn triggers this action again, but for the specific PR that the status
pertained to, giving a better user experience.

Note: It is required for you to use a `github-token` _other_ than the default
`GITHUB_TOKEN` available on the repo, otherwise the action won't be able to
trigger itself, as explained [here](https://docs.github.com/en/free-pro-team@latest/actions/reference/events-that-trigger-workflows#triggering-new-workflows-using-a-personal-access-token)

##### `unlabeled`

If the `block-merge-label` is set, this lets the action know that label might
have been removed and we might be able to merge the PR now.

##### `ready_for_review`, `review_request_removed`, and `review_requested`

If the `reviewed-label` is set, these events let the action know that it needs
to recheck the state of the reviews.

#### [`pull_request_review.submitted`](https://docs.github.com/en/free-pro-team@latest/actions/reference/events-that-trigger-workflows#pull_request_review)

If if the `reviewed-label` is set, this event just lets us know a new review was
submitted that might change the overall review state.

#### [`status`](https://docs.github.com/en/free-pro-team@latest/actions/reference/events-that-trigger-workflows#status)

Event used to determine if all of the required status checks have now passed and
we can add the `ci-passed-label` to the issue. See [`labeled`](#labeled) for
more info.

## Alternatives

* [Github auto-merge](https://docs.github.com/en/free-pro-team@latest/github/collaborating-with-issues-and-pull-requests/automatically-merging-a-pull-request)
* [Mergify](https://mergify.io/)
* [automerge-action](https://github.com/pascalgn/automerge-action)

## Contributing

[![Contributor Covenant](https://img.shields.io/badge/contributor%20covenant-v1.4-ff69b4.svg)](../main/CODE_OF_CONDUCT.md)

We welcome community contributions to this project.

Please read our [Contributor Guide](CONTRIBUTING.md) for more information on how to get started.

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
