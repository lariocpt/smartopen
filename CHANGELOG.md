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

### Fixed

- **yazi never received the file.** yazi 26 hands an opener its files through `%s`
  (every selected file, shell-escaped); the `"$@"` the inherited spec used arrives empty,
  because yazi no longer passes files as positional arguments. Every opener the yazi
  renderer writes — the `smartopen` delegate and the per-type viewers alike — now uses
  `%s`. Found by the navigator test that presses Enter in a real yazi.
- **broot ate the verb's braces.** broot substitutes every `{name}` in a verb, so
  `${TERMINAL:-ghostty}` reached the shell as `$` and the dispatcher's `${1##*/}` as
  nothing. The verb no longer contains a brace other than `{file}`; `${VAR:-default}`
  forms are hoisted into a prelude. Found by the same test, in a real broot.
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
- **The launcher knows where you are.** `[shortcut.when]` offers a shortcut only while
  its conditions hold: `cwd_has = ["Cargo.toml"]` (searched upward to the `.git`
  boundary), `cwd_matches`, `env = ["TMUX", "SSH_CONNECTION=*"]`, and `has = ["gitui"]`
  so a command vanishes on a machine without the tool instead of failing. `--all` shows
  the hidden ones greyed with the condition that hid them.
- **Parameters.** `{{branch}}` in a command is asked for before it runs; `[shortcut.param.branch]`
  can set a `prompt`, a `default` (or `"last"` — what you answered last time), and a
  `choices` command whose output lines are offered in the picker, last answer first.
  Values are shell-quoted like target placeholders. `--param branch=main` presets one.
- **`terminal = true`** runs a command in a new terminal window, spelled correctly for
  `$TERMINAL` or whichever of ghostty/foot/kitty/alacritty/wezterm/xterm is installed,
  Terminal.app on macOS, Windows Terminal (or `start cmd`) on Windows.
- **A shell widget.** `smartopen shell zsh|bash|fish` prints a snippet that binds `Ctrl-G`
  to open the picker and put the chosen command on the prompt line, unexecuted, so it can
  be edited and lands in history. `--print` is the mode it uses.
- **`confirm = true`** shows the rendered command and asks before running (`--yes` skips).
- **`group = "git"`** shows in the picker and filters with a `git:` prefix.
- **Import from navi, pet and tldr.** `smartopen shortcuts import navi|pet|tldr <file>`
  prints `[[shortcut]]` TOML — tags become groups, `<arg>` and `<arg=default>` become
  parameters, navi's `$ arg:` lines become `choices`, and a tldr page becomes a group
  gated on the program being installed. `--write` appends it to the config.
- **Subcommands.** `smartopen config path|edit|init|sample|list|doctor`,
  `smartopen yazi apply|diff|check|print|print-spec`, `smartopen broot apply|diff|check|print`,
  plus `completions`, `man`, `shell` and `shortcuts import`. The old flags (`--doctor`,
  `--setup-yazi`, …) still work, hidden. A file literally named `config`, `yazi` or another
  subcommand needs a `./` prefix.
- **broot integration is back.** `smartopen broot apply` writes an Enter verb (also
  invocable as `:smartopen`) to `smartopen.hjson` and imports it from `conf.hjson`,
  replacing a `yazi-opener-config` import if one is there. `--setup-broot` had been
  removed; `niricritty`'s bake called it anyway and failed silently. It works again.
- **The navigator config delegates to the binary that wrote it.** `opn yazi apply` produces
  a `yazi.toml` that calls `opn`; `--bin` overrides. `--rules` writes explicit per-type
  viewers from the built-in spec instead, `--spec` swaps in your own, and `--target`
  points at a different `yazi.toml` or broot directory.
- **A setup wizard.** `smartopen wizard` — offered on the first run when there is no
  config — starts with the navigators (set up yazi and broot, installing them first if
  they are missing), then walks popular file types one checklist at a time with the
  terminal tools that open them: what is installed is ticked, what would be installed is
  marked, and the recommendation comes first. A review shows the exact TOML and the exact
  package-manager commands before anything is written or run; installs default to no.
  `--dry-run` stops at the review, `--yes` takes every recommendation, `--no-install`
  writes config only. An existing config keeps its shortcuts and `[menu]`.
- **A public installer.** `curl -fsSL https://lariocpt.github.io/smartopen/install.sh | sh`
  resolves the latest release, downloads the archive for this machine, verifies it
  against the release's `SHA256SUMS`, and puts `smartopen` and `opn` in `~/.local/bin`.
  POSIX sh, nothing is executable before it is verified, `--prefix`, `--version`,
  `--source=apps` for the LAN mirror, `--download-only`. Linux gets the static musl build.
- **A verified tool catalogue.** `smartopen tools list` shows every catalogue tool, whether
  it is installed, and the one command that installs it here — chosen from the package
  managers on `PATH` (`paru`/`yay` before `pacman`, then `cargo`, `brew`, `eget`, `pipx`;
  `brew` first on macOS; `winget`/`scoop` on Windows). Every install source in the
  catalogue was checked: on crates.io, `micro`, `glow`, `chafa`, `mpv`, `helix` and `hl`
  are unrelated crates sharing the name, and a test refuses a `cargo` source for them.
