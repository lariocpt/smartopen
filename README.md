# smartopen

> Open anything from the terminal — through a menu you wrote, from yazi, broot, your
> shell, or the command line — on Linux, macOS and Windows.

`smartopen` reads one TOML file that says which commands can open which files, folders and
URLs, and which shortcuts you want a keystroke away. Point it at a file and it runs the one
matching command, or shows a picker when there are several. Run it bare and it is a
launcher for your shortcuts — the ones that make sense *here*, in this directory, on this
machine.

```sh
curl -fsSL https://lariocpt.github.io/smartopen/install.sh | sh    # Linux, macOS
cargo install smartopen                                            # anywhere with Rust
smartopen wizard                                                   # then: set it up
```

`opn` is installed alongside as a three-character alias — the same program.

[**Website**](https://lariocpt.github.io/smartopen/) · [Releases](https://github.com/lariocpt/smartopen/releases) · [Changelog](CHANGELOG.md)

## What it looks like

```
$ smartopen notes.md
┌ Choose a command ─────────┐ ┌ Action ────────────────────────────────┐
│ █                         │ │ Render Markdown                        │
├ Items (3) ────────────────┤ │                                        │
│> 1  [md]   Render Markdown│ │ Rendered view in glow                  │
│  2  [edit] Edit           │ │                                        │
│  3  [finder] Reveal       │ │ Command                                │
│                           │ │ glow -p /home/u/docs/notes.md          │
│                           │ │                                        │
│                           │ │ Executable                             │
│                           │ │ glow: found at /usr/bin/glow           │
│                           │ │                                        │
│                           │ │ Target                                 │
│                           │ │ /home/u/docs/notes.md                  │
│                           │ │ type: file                             │
│                           │ │ extension: md                          │
│                           │ │ mime: text/markdown                    │
└───────────────────────────┘ └────────────────────────────────────────┘
 type to filter · ↑↓ move · 1-9 pick · Alt+key · Enter run · Esc cancel
```

Type to fuzzy-filter, `1`–`9` to pick, `Enter` to run. What you pick often floats up.

## Why not just `xdg-open`

`xdg-open`, `open` and `start` pick *one* application per type and hand over. That is the
right thing for a desktop. In a terminal you usually have several tools for the same file
— a Markdown file might want `glow`, `mdfried`, your editor, or Finder — and the choice
depends on what you are doing. smartopen keeps those choices in one file and offers them
in one menu, and the same file drives every way you reach it:

| you are in… | what happens |
|---|---|
| **yazi** | `smartopen yazi apply` makes Enter on any file open this menu |
| **broot** | `smartopen broot apply` binds Enter on files to the same menu (broot has no per-type opener of its own) |
| **your shell** | `smartopen shell zsh` binds `Ctrl-G`: pick a shortcut, it lands on the prompt line, unexecuted |
| **a keybinding** | `smartopen` with no arguments is a shortcut launcher |
| **a script** | `smartopen --command "Render Markdown" file.md`, `--dry-run`, `--print`, `config doctor --json` |

| | `smartopen` | `xdg-open` / `open` | `handlr` | `rifle` (ranger) | `openwith-fzf` |
|---|---|---|---|---|---|
| several commands per type | yes | no | via rofi/dmenu | yes | yes |
| Linux · macOS · Windows | yes | each its own | Linux | Linux (Python) | Linux (bash) |
| matches on MIME, shebang, name, URL | yes | MIME | MIME | conditions | ext |
| a shortcut launcher too | yes, context-aware | no | no | no | no |
| drives yazi and broot | yes | — | — | ranger only | ranger |
| one static binary | yes | — | yes | no | no |
| several files at once | `{paths}` | no | `%F` | `"$@"` | yes |
| sets the desktop's default app (`mimeapps.list`) | no | yes | yes | no | no |

That last row is the honest one: smartopen's associations live in its own TOML and are
read by smartopen, yazi and broot — not by your browser's "open with" or a double-click.
Keep `xdg-open` (or `handlr`) for that; point it at `smartopen` only for the types you
want a menu for.

## Platform support

| | Linux | macOS | Windows |
|---|---|---|---|
| Shell commands run through | `sh -c` | `sh -c` | `cmd /C` |
| Placeholders quoted for | `'…'` | `'…'` | `"…"` |
| Config | `~/.config/smartopen/config.toml` (`$XDG_CONFIG_HOME`) | same — **not** `~/Library/Application Support` | `%APPDATA%\smartopen\config.toml` |
| Starter config names tools that exist there | yes | yes (`open -R`, Terminal) | yes (`start`, `explorer`, `wt`) |
| `terminal = true` opens | `$TERMINAL` / ghostty / foot / kitty / alacritty / wezterm / xterm | Terminal.app, or `$TERMINAL`'s own CLI (ghostty, kitty, alacritty, wezterm) | Windows Terminal or `cmd` |
| yazi + broot integration | yes | yes (broot reads `~/Library/Application Support/org.dystroy.broot`, and smartopen writes there) | yazi yes; broot no — broot runs verbs without a shell, and the Enter verb needs `sh` |
| Verified by | daily use + CI + real yazi/broot in a pty + five distros in containers | CI + real yazi/broot in a pty | CI (sandboxed CLI suite only) |

CI compiles all eight targets (Linux gnu/musl, macOS, Windows; x86-64 and arm64) on every
push, runs the sandboxed CLI suite on all three OSes, presses Enter in a **real** yazi and
a **real** broot under a pty on Linux and macOS, and installs the musl build in five Linux
distributions through the public installer. Linux is the developed path and the one that
gets used every day; the other two pass the automated checks and see less human use —
reports welcome.

## Install

```sh
curl -fsSL https://lariocpt.github.io/smartopen/install.sh | sh   # Linux, macOS: static musl / native
cargo install smartopen                                           # crates.io
brew install lariocpt/smartopen/smartopen                         # macOS / Linuxbrew
cargo install --path .                                            # from a clone
```

An AUR package (`smartopen-bin`) is written and its checksums match the release, but it
has not been submitted yet — `yay -S smartopen-bin` will not resolve until it is.

Or take an archive from [Releases](https://github.com/lariocpt/smartopen/releases): every
archive holds `smartopen`, `opn`, `README.md` and `LICENSE`; the release's `SHA256SUMS`
covers all of them, and the installer verifies it before anything becomes executable.
Take a **`-musl`** archive if you are choosing by hand: those are statically linked, with
no glibc floor at all, so they run on Alpine and on old enterprise distros alike. The
`-gnu` archives are dynamically linked and need glibc 2.34 or newer; the installer above
always picks musl on Linux.

Building from source needs Rust 1.88 or newer. The dependency tree is pure Rust: no C
toolchain, no `pkg-config`, no system libraries.

## First run

```sh
smartopen wizard
```

The wizard starts with the navigators — set up yazi and broot, installing them first if
they are missing — then walks popular file types one checklist at a time, with the
terminal tools that open them: ✓ installed, ↓ would be installed, ✗ not installable here.
Before anything is written or run it shows the exact TOML and the exact package-manager
commands; installs default to **no**. It is also offered automatically the first time you
run `smartopen` without a config. Whatever you tick, the config it writes also answers a
URL (`[[url]]`), a script by its shebang, any other text file and an empty one — the same
baseline `config init` gives.

```sh
smartopen wizard --dry-run      # just the review
smartopen wizard --yes          # take every recommendation (the review still prints)
smartopen wizard --no-install   # write config only
smartopen tools list            # the catalogue: what is installed, how to get the rest
smartopen config init           # or: a plain starter config for this OS, no questions
```

Every install source in the catalogue was verified: a `cargo` source means the crate of
that name *is* the tool (on crates.io, `micro`, `glow`, `chafa`, `mpv`, `helix` and `hl`
are unrelated crates sharing the name — a test refuses those), a `pacman` source means it
is in the Arch repositories.

## Usage

```sh
smartopen FILE|FOLDER|URL…      # open: one match runs, several show the menu
smartopen                       # the shortcut launcher
smartopen --command LABEL …     # run a command by label, no menu
smartopen --dry-run …           # show what would run
smartopen --print …             # print the chosen command line (what the shell widget uses)
smartopen --menu …              # always show the menu, even for one match
smartopen --all                 # include shortcuts hidden by their `when`, greyed, with why
smartopen --param NAME=VALUE …  # preset a {{parameter}}
smartopen --no-project          # ignore .smartopen.toml files
smartopen --no-history          # neither read nor record picks (also SMARTOPEN_NO_HISTORY=1)

smartopen config path|edit|init|sample|list [--json]|doctor [--json] [--strict]
smartopen yazi   apply [--force] [--no-backup]|diff|check|print|print-spec
smartopen broot  apply [--force] [--no-backup]|diff|check|print
smartopen shell  zsh|bash|fish
smartopen shortcuts import navi|pet|tldr FILE [--write]
smartopen tools  list [--json]
smartopen completions bash|zsh|fish|powershell|elvish
smartopen man
```

A file literally named `config`, `yazi` or another subcommand needs `./` in front.

**In the picker:** type to filter (fuzzy; `git:` narrows to a group) · `↑`/`↓` or
`Ctrl-n`/`Ctrl-p` · `1`–`9` pick a row while unfiltered · `Alt+key` picks a command's
`key` · `Home`/`End`/`PageUp`/`PageDown` · `Ctrl-u` clears · `Enter` runs · `Esc` cancels.
With nothing typed, what you pick often and recently comes first (one-week half-life).

`smartopen` exits with the exit code of the command it ran.

## Configuration

`smartopen config path` prints where the file is. `smartopen config doctor` says whether
every command's program is installed (`--strict` makes a missing one exit 1; `--json` for
scripts). A misspelt key is an error at load time, with the valid keys listed.

### Associations

```toml
[[extension]]                          # by extension
extensions = ["md", "markdown"]
names = ["README"]                     # optional: only these names/stems

[[extension.command]]
label = "Render Markdown"
description = "Rendered view in glow"
icon = "[md]"
run = "glow -p {path}"

[[folder]]                             # directories
names = ["src"]                        # optional; `paths` pins to specific directories
[[folder.command]]
label = "Open in yazi"
run = "yazi {path}"

[[url]]                                # URLs, by scheme and host glob
schemes = ["https"]
hosts = ["github.com", "*.github.com"]
[[url.command]]
label = "GitHub CLI"
run = "gh browse {url}"

[[association]]                        # anything: every listed condition must hold
[association.match]
extensions = ["sh"]
names = ["Makefile"]                   # exact name or stem, case-insensitive
name_patterns = [".env.*", "*.env"]    # globs
dirs = false                           # true = directories only
empty = false                          # true = empty files only
mime = ["text/*", "image/png"]         # from the bytes + name (yazi's vocabulary)
shebang = ["python*", "bash"]          # the interpreter named by a #! line
[[association.command]]
label = "Run"
run = "python3 {path}"
```

`inode/directory`, `inode/empty` and `x-scheme-handler/<scheme>` are MIME types too, so a
generic rule can match folders, empties and URLs. Commands from every matching association
are merged in config order and deduplicated by label.

### Commands

Every command — in an association or a shortcut — takes:

| key | meaning |
|---|---|
| `label` | what the menu shows; `--command` matches it case-insensitively |
| `run` | the command line; runs through `sh -c` (`cmd /C` on Windows) so pipes and `$VARS` work |
| `description`, `icon` | shown in the menu |
| `cwd` | working directory (`~` and `$VAR` expand) |
| `detach = true` | fire and forget: own process group, no waiting — for GUI apps |
| `terminal = true` | run in a **new terminal window**, resolved per OS |
| `platform = "unix" \| "linux" \| "macos" \| "windows"` | offer only there, so one config serves every machine |
| `key = "e"` | `Alt+e` picks it, even mid-filter |
| `priority = 10` | sorts up the menu; equal priorities keep config order |
| `default = true` | runs without a menu when it is the only default among the matches |
| `confirm = true` | shows the rendered line and asks first (`--yes` skips) |
| `group = "git"` | shown as a `git ›` prefix on the row, and a `git:` filter prefix |
| `when` | conditions — see the launcher below |
| `param` | `{{parameters}}` — see the launcher below |

**Placeholders** are substituted, each one shell-quoted for the platform's shell:

| placeholder | value |
|---|---|
| `{path}` | absolute path (or the URL) of the first target |
| `{paths}` | every target, quoted and space-joined — with several targets, a command that uses only `{path}` runs on the first and says so on stderr |
| `{dir}` | its directory (the directory itself for a folder) |
| `{name}`, `{stem}`, `{ext}` | file name, name without extension, extension |
| `{url}`, `{scheme}`, `{host}` | the URL; a file renders as `file://…`, `file`, and empty |

Several targets at once — `smartopen a.csv b.csv` — offer only the commands every target
matches.

### Menu banner

```toml
[menu]
art_file = "art/banner.txt"     # relative to the config file; omit for the built-in
```

### Project configs

A `.smartopen.toml` (or `.opn.toml`) found above the working directory or the target —
stopping at `.git`, your home, or the root — is layered over your config with its entries
first. A repo can ship its own shortcuts, and they appear only inside it. `--no-project`
ignores them; `smartopen config path` names the one in effect. A project config alone is
enough to run.

## The launcher

`smartopen` with no arguments shows your `[[shortcut]]`s. What makes it more than a
bookmark list is that a shortcut can know **where you are**, **ask you things**, and
**remember your answers**.

```toml
[[shortcut]]
label = "Cargo test"
run = "cargo test"
group = "rust"
[shortcut.when]
cwd_has = ["Cargo.toml"]               # any of these, in cwd or above, up to .git

[[shortcut]]
label = "Checkout branch"
run = "git checkout {{branch}}"
group = "git"
[shortcut.when]
cwd_has = [".git"]
has = ["git"]                          # executables that must be on PATH
[shortcut.param.branch]
choices = "git branch --format='%(refname:short)'"   # stdout lines become the picker
default = "last"                       # the answer you gave last time comes first

[[shortcut]]
label = "Deploy"
run = "ssh {{host}} 'systemctl restart app'"
confirm = true
[shortcut.when]
env = ["SSH_AUTH_SOCK", "DEPLOY_ENV=prod*"]   # VAR set, or VAR=glob
[shortcut.param.host]
prompt = "Which host"
default = "web-1"

[[shortcut]]
label = "Git UI in a new window"
run = "gitui"
terminal = true
cwd = "."
```

- **`when`** — `cwd_has`, `cwd_matches` (globs on the absolute directory), `env`, `has`.
  Every condition must hold. `--all` shows the hidden ones greyed with the condition that
  hid them.
- **`{{name}}`** parameters are asked for before the command runs — a picker when the
  parameter has `choices`, a prompt otherwise — and shell-quoted like target placeholders.
  `--param name=value` presets one. Answers are remembered per shortcut for
  `default = "last"`.
- **`terminal = true`** spells "open in this directory and run this" correctly for
  `$TERMINAL`, ghostty, foot, kitty, alacritty, wezterm and xterm, for Terminal.app, and
  for Windows Terminal.

### The shell widget

```sh
eval "$(smartopen shell zsh)"     # ~/.zshrc; also bash, fish
```

`Ctrl-G` opens the launcher and puts the chosen command on your prompt line, unexecuted —
edit it, then run it, and it lands in your history like anything else you typed.

This is not a snippet manager. navi, pet and intelli-shell hold thousands of commands,
with richer variable syntax and a community behind them — keep one of those for that.
smartopen's shortcuts are the ten commands *this* repo needs: gated on where you are,
quoted so a value with a `;` in it stays a value, and shipped in the repo's own
`.smartopen.toml`. The import below is how the two meet.

### Bringing your existing snippets

```sh
smartopen shortcuts import navi ~/.local/share/navi/cheats/git.cheat
smartopen shortcuts import pet  ~/.config/pet/snippet.toml
smartopen shortcuts import tldr ~/.cache/tldr/pages/common/tar.md
smartopen shortcuts import navi git.cheat --write     # append to the config (backed up first)
```

Tags become groups, `<arg>` and `<arg=default>` become parameters, navi's `$ arg:` lines
become `choices`, and a tldr page becomes a group gated on the program being installed.

## File managers

```sh
smartopen yazi apply      # [opener]/[open] in yazi.toml: every file opens through this menu
smartopen broot apply     # an Enter verb in broot's config, also invocable as :smartopen
smartopen yazi diff       # what apply would change; `check` exits 1 on drift, for dotfiles CI
smartopen yazi print-spec # the built-in per-type spec, editable; use it with --spec
```

By default the navigators delegate every file to whichever binary wrote the config
(`opn yazi apply` writes a `yazi.toml` that calls `opn`; `--bin` overrides). `--rules`
writes explicit per-type viewers from the built-in spec instead — mdfried for Markdown,
doxx for Word, xleak for spreadsheets, xan for CSV — so yazi's own `O` menu offers them
without smartopen in the loop. yazi's `Enter` still gets the whole menu either way.

Both writers are surgical: `yazi.toml`'s other tables and comments are preserved
byte-for-byte, broot's `conf.hjson` gains one import, and a backup is written before any
change. Both honour yazi 26's `%s` argument passing and broot's `{…}` substitution rules —
details that the navigator tests found the hard way.

## Development

```sh
cargo test                                                      # unit + CLI, sandboxed
SMARTOPEN_NAVIGATOR_TESTS=1 cargo test --test navigators        # real yazi + broot in a pty
test/containers/run.sh                                          # five distros, via podman
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

[`AGENTS.md`](AGENTS.md) is the guide for anyone — human or agent — changing this repo:
what is load-bearing, how the tests are layered, and how a release is cut.

## License

MIT — see [LICENSE](LICENSE).
