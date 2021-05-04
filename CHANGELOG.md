# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

[Unreleased]: https://github.com/EmbarkStudios/octobors/compare/1.1.0...HEAD
[1.1.0]: https://github.com/EmbarkStudios/octobors/releases/tag/1.1.0
[1.0.9]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.9
[1.0.8]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.8
[1.0.7]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.7
[1.0.6]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.6
[1.0.5]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.5
[1.0.4]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.4
[1.0.3]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.3
[1.0.2]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.2
[1.0.1]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.1
[1.0.0]: https://github.com/EmbarkStudios/octobors/releases/tag/1.0.0
[0.3.0]: https://github.com/EmbarkStudios/octobors/releases/tag/0.3.0
[0.2.0]: https://github.com/EmbarkStudios/octobors/releases/tag/0.2.0

## Unreleased
### [1.1.0] - 2021-05-04
- The `extra_headers` config key can be used to add additional API headers to every request to GitHub.
- The `github_api_base` config key can be used to use a different base URL for requests to GitHub.

### [1.0.9] - 2021-03-10
- Corrected the URL used in PR merge messages.

## [1.0.8] - 2021-02-26
### Removed
- Octobors no longer leaves a comment when unable to merge. This was found to
  be too noisy.

## [1.0.7] - 2021-02-23
### Added
- Octobors now correctly handles replies to comments on reviews.

## [1.0.6] - 2021-02-22
### Added
- Octobors now logs additional information.

## [1.0.5] - 2021-02-15
### Changed
- Octobors now checks PRs up to 1 hour after the PR's updated-at timestamp in
  order to check for status events that may not update the timestamp.

## [1.0.4] - 2021-02-12
### Added
- The CI passed label is now optional.

## [1.0.3] - 2021-02-10
### Added
- More concise and information rich logging.

## [1.0.2] - 2021-02-09
### Added
- Updated `octocrab` to 0.8.11.

### Fixed
- Requested reviews are now correctly taken into account when deciding
  whether to merge a PR.

## [1.0.1] - 2021-02-08
### Added
- The `GITHUB_TOKEN` environment variable is now trimmed of whitespace.

## [1.0.0] - 2021-02-08
### Added
- Cron based design.

### Removed
- GitHub Actions based design.

## [0.3.0] - 2021-02-03
### Added
- PR number and URL are now included in merge title and message.

## [0.2.0] - 2021-01-15
### Added
- Initial release
