# Working on smartopen

The guide for anyone — human or agent — changing this repo. `CLAUDE.md` is a symlink to
this file. The README explains what the tool does; this explains what will bite you.

## Shape of the thing

One crate, two binaries. `src/bin/smartopen.rs` and `src/bin/opn.rs` are three lines each
and both call `smartopen::main_exit_code()`; `opn` is the alias the estate's yazi, broot
and niri configs invoke. `Cargo.toml` names the crate `smartopen` — the name on crates.io,
the repository and the primary binary. (`opn` on crates.io is somebody else's macOS-`open`
clone; the repo was called `opn` and `cli-rust-menu` before.)

The core path is `config` → `target` → `matcher` → `menu` → `runner`:

- `config.rs` — the TOML model. Every table is `deny_unknown_fields`, and serialisation
  skips defaults so `--list --json` and `shortcuts import` emit only what was set.
- `target.rs` — a file, directory or URL, with `mime.rs` reading the first 8 KiB for a
  shebang and a MIME type in yazi's vocabulary.
- `matcher.rs` — which commands a config offers; several targets intersect.
- `menu.rs` — the ratatui picker (`fuzzy.rs`, `history.rs`) and the wizard's checklist.
- `runner.rs` — placeholders → `shell.rs` quoting → `sh -c` / `cmd /C`; `terminal.rs`
  for `terminal = true`.
- `launcher.rs` (`when`), `params.rs` (`{{name}}`), `shell_widget.rs`, `import.rs`.
- The navigator surface: `spec.rs` (the association model), `engine.rs` (delegate
  everything to the menu, or explicit per-type viewers), `render.rs` (yazi TOML),
  `tomlio.rs` (surgical splice into `yazi.toml`, atomic writes, backups), `broot.rs`,
  `navigators.rs` (the `yazi …`/`broot …` subcommands).
- `catalog.rs` + `installer.rs` + `wizard.rs` — the first-run wizard, driven by the data
  in `catalog/`.

## Rules that are load-bearing

**Every substituted value is quoted for the shell that will run it.** `{path}` and
`{{param}}` go through `Shell::quote`: POSIX `'…'` on Unix, `"…"` with doubled quotes on
Windows, and a `%` or newline for cmd is refused rather than mangled. Never build a command
line by string concatenation elsewhere; tests exercise both shells explicitly, not the
host's default.

**yazi passes files through `%s`, not `"$@"`.** yazi 26 substitutes `%s` (every selected
file, shell-escaped) and marks positional passing "TODO: remove"; `"$@"` arrives empty.
`render.rs::yazi_args` translates the spec's POSIX spelling. broot's dispatcher really is
run by `sh`, so the spec keeps `"$@"`/`"$1"`. The navigator test found this; read
[`yazi-fs`'s `Splatter`] before touching it.

**A broot verb contains no `{` except `{file}`.** broot substitutes every `{name}` in a
verb's text, so `${TERMINAL:-ghostty}` reached the shell as `$`. `broot.rs::debrace`
hoists `${VAR:-default}` into a prelude; a test counts the braces.

**The picker draws on the tty, never stdout.** `--print` hands stdout to the shell
widget's `$(…)`; frames must not end up in that variable. `/dev/tty` (`CONOUT$` on
Windows), stderr as the fallback.

**Config paths follow XDG on macOS too.** `~/.config`, not `~/Library/Application
Support`; `%APPDATA%` on Windows; yazi agrees. broot does not: on macOS it reads
`~/Library/Application Support/org.dystroy.broot`, and `BROOT_CONFIG_DIR` overrides it
everywhere — `paths::broot_config_dir` resolves the way broot does, because a config
written where broot does not look is worse than an error. Every resolver returns
`Option` — never fall back to `.`.

**Every install source in `catalog/tools.toml` is a verified claim.** A `cargo` key means
the crate of that name IS the tool (crates.io); a `pacman` key means `pacman -Si` finds
it. `catalog.rs::NEVER_CARGO` lists the names whose crates are unrelated squatters
(`micro`, `glow`, `chafa`, `mpv`, `helix`, `hl`, plus the estate's `qo`/`surge`/
`redthread`), and a test refuses a `cargo` key for them. Missing means "not there", not
"didn't look".

**Nothing hides behind `#![allow(unused)]`.** It was removed in the first public-release
commit; dead code is deleted, not allowed.

**A launcher's bookkeeping must never stop it launching.** History (`history.rs`) treats an
unreadable file as empty and a failed save as one stderr line.

## Tests: five layers, each answering one question

| layer | proves | run |
|---|---|---|
| L1 unit | quoting for both shells, paths per OS, matching, catalogue validity, resolver | `cargo test --lib` |
| L2 CLI | every subcommand on all three OSes, in a sandboxed home | `cargo test --test cli` |
| L3 navigators | **real yazi + real broot in a pty**: Enter reaches smartopen; the zsh widget pastes without running | `SMARTOPEN_NAVIGATOR_TESTS=1 cargo test --test navigators -- --test-threads=1` |
| L4 distros | alpine (no glibc), bookworm, ubuntu 22.04, fedora, arch via `docs/install.sh` | `test/containers/run.sh` (podman) |
| L5 published | the released archives downloaded back and smoked on every runner | `release.yml` |

L3 needs `yazi`, `broot` and `zsh` on PATH. It answers yazi's DA1 terminal query
(`ESC[0c` → `ESC[?62;22c`) because yazi waits for it before reading keys, and sets
`BROOT_CONFIG_DIR` because broot does not follow XDG on macOS. **The suite is
`cfg(unix)`; the Windows navigators job installs the tools and runs nothing** — it is
`continue-on-error` and says so. Making the harness drive ConPTY, then three green tags,
is the path to promoting it; delete this paragraph when that happens.

`tests/cli.rs` and `tests/navigators.rs` sandbox `HOME`, `XDG_*`, `APPDATA` and
`LOCALAPPDATA` into a tempdir. **Do the same when smoke-testing by hand**: `wizard --yes`,
`yazi apply` and `broot apply` write to the real `~/.config` otherwise — it happened once.

A phase is not done until `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`
(also for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`) and every layer you can
run locally pass.

## Docs are part of the deliverable

`README.md` describes what the binary actually does — every flag, key and placeholder in
it exists. `docs/index.html` is the whole website, one self-contained file served by Pages
with no build step (rules in `docs/README.md`). `docs/install.sh` is the public installer
and is **POSIX sh**, not bash: `sh -n`, `dash -n` and `shellcheck -s sh` in CI. If you add
or change a capability, update the README and the site, and never document behaviour you
have not run.

## Releasing

`version` in `Cargo.toml` is the only version string in the project.

```
1. bump version in Cargo.toml (Cargo.lock follows)
2. rename "## [Unreleased]" in CHANGELOG.md to "## [X.Y.Z] - YYYY-MM-DD"
3. PR into main, CI green
4. git tag -a vX.Y.Z-rc.1 -m 'smartopen X.Y.Z rc1' && git push origin vX.Y.Z-rc.1   # rehearse
5. git tag -a vX.Y.Z -m 'smartopen X.Y.Z' && git push origin vX.Y.Z
```

`.github/workflows/release.yml` refuses a tag that disagrees with `Cargo.toml`, refuses a
tag with no changelog section, reruns the full gate, creates the release as a **draft**,
uploads eight archives, generates and attests one `SHA256SUMS`, smokes the published bytes
on seven runners, and only then flips the draft — to `latest`, or to prerelease for an
`-rc` tag, which never becomes `latest`. `cargo publish` runs last and needs the
`CARGO_REGISTRY_TOKEN` secret.

Two planes, one source of truth: the GitHub Release is the bytes. The LAN Jenkins job
**mirrors** it (download, verify `SHA256SUMS`, `gh attestation verify`, smoke, republish
unchanged, assert byte identity) — it never builds. Consumers on the estate install from
the public release; the plane is a cache for machines with no route out.

## Personal-repo convention

Branch and open a PR into `main`. Never commit straight to `main`.

[`yazi-fs`'s `Splatter`]: https://github.com/sxyazi/yazi/blob/main/yazi-fs/src/splatter.rs
