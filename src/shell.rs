//! The shell a command line runs through, and how to quote for it.
//!
//! Placeholders such as `{path}` are substituted into a user-written command line that is
//! then handed to a shell, so every substituted value has to be quoted for THAT shell.
//! POSIX `sh` and Windows `cmd.exe` disagree about nearly everything here: `'…'` is not a
//! quote in cmd at all, `"…"` is the only quoting cmd has, and cmd expands `%VAR%` even
//! inside quotes with no way to escape it. Quoting is therefore a method on the shell,
//! and the tests exercise both shells explicitly rather than whichever one the test
//! machine happens to run.

use std::process::Command;

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    /// `sh -c`, on every Unix including macOS.
    Posix,
    /// `cmd /C`, on Windows.
    Cmd,
}

/// Why a value cannot be quoted. Every case is cmd.exe: POSIX `'…'` can hold anything.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuoteError {
    #[error(
        "cannot pass {0:?} through cmd.exe: a `%` is expanded as a variable even inside quotes, and cmd has no way to escape it"
    )]
    Percent(String),
    #[error("cannot pass {0:?} through cmd.exe: cmd command lines cannot contain a newline")]
    Newline(String),
    #[error(
        "cannot pass {0:?} through cmd.exe: a backslash right before a quote has no single reading in cmd"
    )]
    BackslashQuote(String),
}

impl Shell {
    /// The shell the running OS hands command lines to.
    pub fn current() -> Shell {
        if cfg!(windows) {
            Shell::Cmd
        } else {
            Shell::Posix
        }
    }

    /// A [`Command`] that runs `line` through this shell.
    pub fn command(self, line: &str) -> Command {
        match self {
            Shell::Posix => {
                let mut shell = Command::new("sh");
                shell.arg("-c").arg(line);
                shell
            }
            Shell::Cmd => {
                let mut shell = Command::new("cmd");
                // `/S /C "…"`: cmd strips exactly the one pair of quotes added here and
                // nothing else. Without `/S`, a line holding more than two `"` loses its
                // first and last — `"C:\Program Files\x.exe" "a b"` reached the program as
                // `C:\Program Files\x.exe" "a b`. yazi wraps its own command lines the
                // same way.
                shell.arg("/S").arg("/C");
                let wrapped = format!("\"{line}\"");
                // Rust re-quotes every argument for CreateProcess, but this is already a
                // complete cmd command line and has to reach cmd byte for byte.
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    shell.raw_arg(wrapped);
                }
                #[cfg(not(windows))]
                {
                    shell.arg(wrapped);
                }
                shell
            }
        }
    }

    /// Quote `value` so this shell passes it through as exactly one argument.
    pub fn quote(self, value: &str) -> Result<String, QuoteError> {
        match self {
            Shell::Posix => Ok(quote_posix(value)),
            Shell::Cmd => quote_cmd(value),
        }
    }
}

fn quote_posix(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    if value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'/' | b'.' | b'_' | b'-' | b':' | b'@' | b'+' | b',')
    }) {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

fn quote_cmd(value: &str) -> Result<String, QuoteError> {
    if value.contains('%') {
        return Err(QuoteError::Percent(value.to_string()));
    }
    if value.contains(['\n', '\r']) {
        return Err(QuoteError::Newline(value.to_string()));
    }
    // A `"` cannot occur in a Windows path, so this only ever comes from a typed parameter;
    // which of the C runtime's `\"` rules the child applies is not ours to know.
    if value.contains("\\\"") {
        return Err(QuoteError::BackslashQuote(value.to_string()));
    }

    if !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'\\' | b'/' | b'.' | b'_' | b'-' | b':')
        })
    {
        return Ok(value.to_string());
    }

    // `"` is cmd's only quoting. A quote inside is doubled: cmd's own quote tracking flips
    // twice and stays consistent, and the C runtime that parses the child's arguments
    // reads `""` inside a quoted argument as one literal `"`.
    //
    // The runtime also reads 2n backslashes before a `"` as n backslashes plus a
    // delimiter, so a value ending in `\` — `C:\my dir\` — would turn the closing quote
    // into a literal and swallow the rest of the line. Double the trailing run.
    let trailing = value.len() - value.trim_end_matches('\\').len();
    Ok(format!(
        "\"{}{}\"",
        value.replace('"', "\"\""),
        "\\".repeat(trailing)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_leaves_safe_values_bare() {
        assert_eq!(Shell::Posix.quote("/tmp/file.rs").unwrap(), "/tmp/file.rs");
    }

    #[test]
    fn posix_quotes_spaces_and_escapes_single_quotes() {
        assert_eq!(Shell::Posix.quote("a b").unwrap(), "'a b'");
        assert_eq!(Shell::Posix.quote("it's here").unwrap(), "'it'\\''s here'");
        assert_eq!(Shell::Posix.quote("").unwrap(), "''");
    }

    #[test]
    fn cmd_leaves_plain_windows_paths_bare() {
        assert_eq!(
            Shell::Cmd.quote(r"C:\Users\lario\file.rs").unwrap(),
            r"C:\Users\lario\file.rs"
        );
    }

    #[test]
    fn cmd_double_quotes_spaces_and_doubles_embedded_quotes() {
        assert_eq!(
            Shell::Cmd.quote(r"C:\my files\a.csv").unwrap(),
            r#""C:\my files\a.csv""#
        );
        assert_eq!(Shell::Cmd.quote(r#"say "hi""#).unwrap(), r#""say ""hi""""#);
        assert_eq!(Shell::Cmd.quote("").unwrap(), r#""""#);
    }

    #[test]
    fn cmd_doubles_a_trailing_backslash_so_the_closing_quote_survives() {
        assert_eq!(Shell::Cmd.quote(r"C:\my dir\").unwrap(), r#""C:\my dir\\""#);
        assert_eq!(
            Shell::Cmd.quote(r"C:\my dir\\").unwrap(),
            r#""C:\my dir\\\\""#
        );
        // Bare values need no quotes, so a trailing backslash there is left alone.
        assert_eq!(Shell::Cmd.quote(r"C:\dir\").unwrap(), r"C:\dir\");
        assert_eq!(
            Shell::Cmd.quote(r#"say \"hi"#),
            Err(QuoteError::BackslashQuote(r#"say \"hi"#.to_string()))
        );
    }

    #[test]
    fn cmd_refuses_what_it_cannot_escape() {
        assert_eq!(
            Shell::Cmd.quote("100%"),
            Err(QuoteError::Percent("100%".to_string()))
        );
        assert_eq!(
            Shell::Cmd.quote("two\nlines"),
            Err(QuoteError::Newline("two\nlines".to_string()))
        );
    }

    #[test]
    fn current_matches_the_build_target() {
        assert_eq!(
            Shell::current(),
            if cfg!(windows) {
                Shell::Cmd
            } else {
                Shell::Posix
            }
        );
    }
}
