# opn

`opn` is a configurable Rust CLI for opening files through command menus.
It reads a TOML config, matches a path against file and folder associations, and
shows an interactive picker for the target. With no path, it shows configured
shortcuts.

When a target matches exactly one command, `opn` runs it directly. When a target
matches several commands, `opn` opens the picker.

The interactive picker has a command list on the left and an action panel on the
right. Moving the cursor updates the panel with the selected command's
description, rendered shell command, working directory, and target path.

## Requirements

- Rust toolchain with `rustc` and `cargo`
- The external programs referenced by your config, such as `$EDITOR`, `npm`,
  `cat`, `ls`, or image editors
- Optional for contributors: `rustfmt` and `clippy`

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
opn --config         # print config path
opn --init-config    # create starter config
opn --edit-config    # create/open config in $EDITOR
opn --list           # list associations and shortcuts
opn --doctor         # check menu art and command availability
opn file.rs          # match file commands
opn                  # show shortcut launcher
```

Inside the menu, type to filter, use arrow keys to move, press Enter to run the
selected action, or Esc to cancel.

Config lives at:

```text
~/.config/opn/config.toml
```

Print a starter config without writing it:

```sh
opn --sample-config
```

There are also checked-in examples at `examples/config.toml`,
`examples/config-with-art.toml`, and `examples/art/`.
The main example config mirrors the file associations from the sibling
`yazi-opener-config` project.

Use a temporary or project-local config:

```sh
opn --config-path ./opn.toml --list
```

Run by label without the interactive picker:

```sh
opn --command "Print file" file.rs
opn --command "Cargo test"
```

Preview what would run:

```sh
opn --dry-run --command "Print file" file.rs
```

Check whether configured commands are installed:

```sh
opn --doctor
opn --config-path examples/config.toml --doctor
```

Dynamic shell commands such as `${EDITOR:-nano}` are reported as dynamic because
the final executable is chosen by the shell at runtime.

## Config

```toml
[[extension]]
extensions = ["rs"]

[[extension.command]]
label = "Open in editor"
description = "Edit this Rust file"
icon = "[edit]"
run = "${EDITOR:-nano} {path}"

[[extension.command]]
label = "Print file"
description = "Write file contents to the terminal"
icon = "[cat]"
run = "cat {path}"

[[folder]]

[[folder.command]]
label = "List directory"
description = "Show directory contents"
icon = "[ls]"
run = "ls -la {path}"

[[extension]]
extensions = ["csv"]

[[extension.command]]
label = "Edit CSV"
description = "Open the CSV in csvi, a terminal spreadsheet-style editor"
icon = "[csv]"
run = "csvi {path}"

[[extension.command]]
label = "Query CSV"
description = "Open the CSV in qo for interactive SQL-style querying"
icon = "[sql]"
run = "qo {path}"

[[extension.command]]
label = "Preview CSV"
description = "Preview the CSV in xan's terminal table viewer"
icon = "[xan]"
run = "xan view {path}"

[[extension]]
extensions = ["html", "htm"]

[[extension.command]]
label = "Open in Gram"
description = "Open the HTML file in Gram"
icon = "[gram]"
run = "gram {path}"

[[extension.command]]
label = "Edit in Micro"
description = "Open the HTML file in micro"
icon = "[edit]"
run = "micro {path}"

[[shortcut]]
label = "Cargo test"
description = "Run the Rust test suite"
icon = "[test]"
run = "cargo test"
cwd = "."
```

Add a banner above interactive menus with an art file:

```toml
[menu]
art_file = "art/compact.txt"
```

If `art_file` is omitted, `opn` uses its built-in default ASCII banner.
Relative art paths resolve from the config file's directory.

For a normal user config, copy one of the examples into the config directory:

```sh
mkdir -p ~/.config/opn/art
cp examples/art/compact.txt ~/.config/opn/art/banner.txt
```

Then set:

```toml
[menu]
art_file = "art/banner.txt"
```

Supported command placeholders:

- `{path}`: absolute matched path
- `{dir}`: containing directory, or the directory itself for directory matches
- `{name}`: file name
- `{stem}`: file name without extension
- `{ext}`: extension without the leading dot

Commands run through the system shell, so pipes, globs, and shell environment
expansion work. Placeholder values are shell-quoted before substitution.

Extension association fields:

- `extensions`: extension values with or without the leading dot, matched
  case-insensitively
- `names`: optional exact file name or file stem filter, matched
  case-insensitively

Folder association fields:

- `names`: optional exact folder name filter, matched case-insensitively
- `paths`: optional folder paths, with relative paths resolved from the config
  file's directory

Generic association match fields:

- `extensions`: extension values with or without the leading dot, matched
  case-insensitively
- `names`: exact file name or file stem, matched case-insensitively
- `name_patterns`: file name or stem globs such as `.env.*` or `*.env`,
  matched case-insensitively
- `dirs`: `true` for directories, `false` for non-directories
- `empty`: `true` for empty files, `false` for non-empty targets

Commands from all matching extension, folder, and generic associations are
merged and deduplicated by label.
