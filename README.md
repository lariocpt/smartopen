# smartopen

`smartopen` is a configurable Rust CLI for opening files through command menus.
It reads a TOML config, matches a path against file associations, and either runs
the only matching command or shows an interactive picker. With no path, it shows
configured shortcuts.

## Requirements

- Rust toolchain with `rustc` and `cargo`
- The external programs referenced by your config, such as `$EDITOR`, `npm`,
  `cat`, `ls`, or image editors
- Optional for contributors: `rustfmt`

Install Rust with rustup:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

If you use distro packages, install `rustfmt` separately when you want
`cargo fmt`.

## Build

```sh
cargo build
```

Install the local binary:

```sh
cargo install --path .
```

## Usage

```sh
smartopen --config         # print config path
smartopen --init-config    # create starter config
smartopen --edit-config    # create/open config in $EDITOR
smartopen --list           # list associations and shortcuts
smartopen file.rs          # match file commands
smartopen                  # show shortcut launcher
```

Config lives at:

```text
~/.config/smartopen/config.toml
```

Print a starter config without writing it:

```sh
smartopen --sample-config
```

There is also a checked-in example at `examples/config.toml`.

Use a temporary or project-local config:

```sh
smartopen --config-path ./smartopen.toml --list
```

Run by label without the interactive picker:

```sh
smartopen --command "Print file" file.rs
smartopen --command "Cargo test"
```

Preview what would run:

```sh
smartopen --dry-run --command "Print file" file.rs
```

## Config

```toml
[[association]]
match = { extensions = ["rs"] }

[[association.command]]
label = "Open in editor"
description = "Edit this Rust file"
icon = "[edit]"
run = "${EDITOR:-nano} {path}"

[[association.command]]
label = "Print file"
description = "Write file contents to the terminal"
icon = "[cat]"
run = "cat {path}"

[[shortcut]]
label = "Cargo test"
description = "Run the Rust test suite"
icon = "[test]"
run = "cargo test"
cwd = "."
```

Supported command placeholders:

- `{path}`: absolute matched path
- `{dir}`: containing directory, or the directory itself for directory matches
- `{name}`: file name
- `{stem}`: file name without extension
- `{ext}`: extension without the leading dot

Commands run through the system shell, so pipes, globs, and shell environment
expansion work. Placeholder values are shell-quoted before substitution.

Association match fields:

- `extensions`: extension values with or without the leading dot, matched
  case-insensitively
- `names`: exact file name or file stem, matched case-insensitively
- `dirs`: `true` for directories, `false` for non-directories

When several match fields are present on one association, all of them must
match. Commands from all matching associations are merged and deduplicated by
label.
