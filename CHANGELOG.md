# Changelog

All notable changes to smartopen are recorded here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The `## [X.Y.Z]` heading shape is load-bearing, not a style choice:
`.github/workflows/release.yml` extracts everything between one such heading and the
next to use as the GitHub Release notes, and refuses to publish a tag that has no
section here. `version` in `Cargo.toml` must match the tag being released.

## [Unreleased]

First public release. Until now this was a LAN-only tool named `opn`, built by Jenkins
for one Linux distribution; this release is the same picker made honest about the
platforms it runs on, published to crates.io and GitHub Releases, and reachable from a
file manager, a shell keybinding, or the command line with one config.

### Changed

- **Published as `smartopen`.** The crate, the repository and the primary binary are all
  `smartopen`. `opn` stays as a short alias binary — the two are the same program, and
  both are installed by every distribution channel.
- `Cargo.toml` carries the crates.io metadata (description, license, repository,
  keywords, categories, MSRV 1.88) and a release profile with thin LTO and stripping.
- `--version` reports `smartopen <version>` from both binaries.

### Added

- `LICENSE` (MIT) and this changelog.
