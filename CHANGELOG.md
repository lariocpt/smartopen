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
- **Runs on macOS and Windows as well as Linux.** Command lines are quoted for the shell
  that will run them: `sh` gets `'…'`, `cmd.exe` gets `"…"` with embedded quotes doubled,
  and a value cmd cannot carry (a `%`, a newline) is refused rather than mangled. Target
  paths on Windows stay `C:\…` instead of the `\\?\C:\…` form most programs reject.
  `detach = true` now really detaches — its own process group on Unix, a detached console
  on Windows — so closing the terminal no longer takes the launched app with it. Looking
  up a bare command name honours `PATHEXT` on Windows.
- **Config lives in `~/.config/smartopen/` on macOS too**, not `~/Library/Application
  Support`: `$XDG_CONFIG_HOME` then `~/.config` on every Unix, `%APPDATA%` on Windows. A
  legacy `~/.config/opn/config.toml` is still honoured when that is all there is. yazi's
  and broot's paths resolve by the same rule, including their Windows locations.
- **`platform = "unix" | "linux" | "macos" | "windows"` on any command**, so one config
  can serve every machine: a command for another OS is neither offered nor reported by
  `--doctor`. The vocabulary is yazi's `for` field.
- **A starter config per OS.** `--init-config` writes a Linux, macOS or Windows sample
  whose tools exist there (`open -R`, `start ""`, `explorer /select,` …), so a fresh
  install followed by `--doctor` is not a wall of red.
