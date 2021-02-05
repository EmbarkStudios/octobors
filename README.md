# 🆙 octobors

[![Embark](https://img.shields.io/badge/embark-open%20source-blueviolet.svg)](https://embark.dev)
[![Embark](https://img.shields.io/badge/discord-ark-%237289da.svg?logo=discord)](https://discord.gg/dAuKfZS)
[![dependency status](https://deps.rs/repo/github/EmbarkStudios/octobors/status.svg)](https://deps.rs/repo/github/EmbarkStudios/octobors)
[![Build status](https://github.com/EmbarkStudios/octobors/workflows/CI/badge.svg)](https://github.com/EmbarkStudios/octobors/actions)

A Rust program for automerging PRs based on a few rules.

## Why?

We made our own automerge program because we couldn't find an existing solution that did exactly
what we wanted.

1. [Github's auto-merge](https://docs.github.com/en/free-pro-team@latest/github/collaborating-with-issues-and-pull-requests/automatically-merging-a-pull-request) relies on branch protection rules, some of which we think are kind of broken (required review count), and some of which we don't want to use for other reasons. You also still have to click a button! The horror!
1. [Mergify](https://mergify.io/) - External service that we don't want to use for our private repos
1. [automerge-action](https://github.com/pascalgn/automerge-action) - Only provided part of the functionality we wanted, and still relies too heavily on branch protections.

## Usage

```shell
export GITHUB_TOKEN="my-github-secret-token"
octobors path/to/config.toml
```

Run the octobors binary, giving it a path to a config file containing the
repos you wish to process. A GitHub token with write permission to the repo
must be found in the `GITHUB_TOKEN` environment variable.

See [config/example.toml](config/example.toml) for the configuration that Octobors expects.

We recommend running this on a periodic schedule every minute using cron,
Kubernetes Cronjobs, or similar. Unfortunately GitHub actions schedules can
be delayed by up-to 30 minutes and so are not suitable, at least not unless
you are happy to wait a long time for PRs to get merged.

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
