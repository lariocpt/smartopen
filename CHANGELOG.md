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
- **The picker is a real picker.** Fuzzy matching with the matched characters
  highlighted — label hits outrank description hits, which outrank hits on the command
  text. `1`–`9` pick a row directly while the list is unfiltered, and a command can carry
  `key = "e"` so `Alt+e` picks it even mid-filter. `Home`/`End`/`PageUp`/`PageDown`,
  `Ctrl-n`/`Ctrl-p`, and `Ctrl-u` to clear the filter. `j` and `k` are letters again —
  they used to move the cursor, which made "json" and "kill" impossible to type.
- **Recently used floats up.** With no filter typed, commands you pick often and recently
  come first (a one-week half-life), with the config's order as the tiebreak. The record
  lives in `$XDG_STATE_HOME/smartopen/state.toml` (`%LOCALAPPDATA%` on Windows); pass
  `--no-history` or set `SMARTOPEN_NO_HISTORY=1` to neither read nor write it.
- **Exit codes are the launched command's.** `smartopen` exits with whatever the command
  it ran exited with (128+signal if a signal killed it), instead of turning every
  non-zero exit into a generic error and exit 1.
- **A misspelt config key is an error.** Every table rejects unknown fields, so
  `extension = ` where `extensions = ` was meant fails at load with the key named, instead
  of producing a rule that silently never matches.
- **`--json`** for `--list` and `--doctor`, for scripts and for the wizard.
- **`--doctor` reports and exits 0.** A missing viewer is a finding, not a failure — a
  config may name optional tools on purpose. `--strict` makes a missing command exit 1,
  for CI. The report also names the platform and says which commands were skipped as
  belonging to another OS.
- **`smartopen completions <bash|zsh|fish|powershell|elvish>`** and **`smartopen man`**,
  each naming whichever binary was invoked, so `opn completions zsh` completes `opn`.
- **Files without a telling name.** `Makefile`, `Dockerfile`, a script with no extension:
  the first 8 KiB decide. A `#!` line names the interpreter (`shebang = ["python*"]`),
  magic bytes and a short extension table give a MIME type (`mime = ["text/*"]`), in
  yazi's vocabulary — `inode/directory`, `inode/empty`, `image/png`. Both are new keys
  on `[association.match]`, and the picker's detail pane shows what was detected.
- **URLs.** `smartopen https://github.com/lariocpt/smartopen` matches a new `[[url]]`
  section by `schemes` and host globs (`hosts = ["*.github.com"]`), with `{url}`,
  `{scheme}` and `{host}` placeholders. Anything starting with a two-or-more-character
  scheme is a URL, so `C:\…` is still a path.
- **Several targets at once.** `smartopen a.csv b.csv` offers only the commands every
  target matches; `{path}` is the first and the new `{paths}` is all of them, quoted.
- **`priority = 10`** sorts a command up the menu (equal priorities keep config order),
  and **`default = true`** runs it without a menu when it is the one default among the
  matches. `--menu` forces the menu anyway.
- **Project configs.** A `.smartopen.toml` (or `.opn.toml`) found above the working
  directory or the target — stopping at `.git`, home, or the root — is layered over the
  user config, its associations and shortcuts first. A repo can ship its own shortcuts,
  and they appear only inside it. `--no-project` ignores them; `--config` names the one in
  effect.
