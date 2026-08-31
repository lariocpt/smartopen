//! What kind of thing a file is, when its name does not say.
//!
//! `Makefile`, `Dockerfile`, a script with no extension — extension matching cannot see
//! them. This module reads the first few kilobytes and answers two questions: what
//! interpreter a `#!` line names, and what MIME type the bytes look like. The vocabulary
//! is yazi's (`inode/directory`, `inode/empty`, `text/plain`, …) so a `mime = "text/*"`
//! rule reads the same in both tools.
//!
//! Detection order: shebang → magic bytes (the `infer` crate) → a small extension table →
//! `text/plain` if the head is valid UTF-8 with no NUL → `application/octet-stream`.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// How much of a file is read for detection. Shebangs are on line one; every magic
/// number `infer` knows sits well inside this.
const HEAD_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub mime: String,
    /// The interpreter's basename from a `#!` line: `python3`, `bash`, `node`.
    pub shebang: Option<String>,
}

pub fn detect_file(path: &Path, ext: &str, is_empty: bool) -> Detected {
    if is_empty {
        return Detected {
            mime: "inode/empty".to_string(),
            shebang: None,
        };
    }

    let head = read_head(path).unwrap_or_default();
    detect_bytes(&head, ext)
}

/// The pure part, so it is testable on byte strings.
pub fn detect_bytes(head: &[u8], ext: &str) -> Detected {
    let shebang = shebang_interpreter(head);

    let mime = if let Some(interpreter) = &shebang {
        mime_for_interpreter(interpreter)
    } else if let Some(kind) = infer::get(head) {
        kind.mime_type().to_string()
    } else if let Some(known) = mime_for_extension(ext) {
        known.to_string()
    } else if looks_like_text(head) {
        "text/plain".to_string()
    } else {
        "application/octet-stream".to_string()
    };

    Detected { mime, shebang }
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut buffer = vec![0u8; HEAD_BYTES];
    let mut filled = 0;
    // A short read is not the end; loop until the buffer is full or the file ends.
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    buffer.truncate(filled);
    Ok(buffer)
}

/// `#!/usr/bin/env python3` → `python3`; `#!/bin/bash -e` → `bash`.
fn shebang_interpreter(head: &[u8]) -> Option<String> {
    let rest = head.strip_prefix(b"#!")?;
    let line_end = rest
        .iter()
        .position(|&b| b == b'\n' || b == b'\r')
        .unwrap_or(rest.len());
    let line = String::from_utf8_lossy(&rest[..line_end]);
    let mut words = line.split_whitespace();
    let program = words.next()?;
    let program_name = program.rsplit('/').next()?;

    // `env` is a trampoline: the interpreter is its first non-flag argument.
    let interpreter = if program_name == "env" {
        words.find(|word| !word.starts_with('-'))?
    } else {
        program_name
    };
    Some(interpreter.rsplit('/').next()?.to_string())
}

fn mime_for_interpreter(interpreter: &str) -> String {
    // Version suffixes fold away: python3.12 is python.
    let base = interpreter.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    match base {
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" => "text/x-shellscript",
        "python" | "pypy" => "text/x-python",
        "perl" => "text/x-perl",
        "ruby" => "text/x-ruby",
        "node" | "deno" | "bun" => "text/javascript",
        "php" => "text/x-php",
        "lua" => "text/x-lua",
        "awk" | "gawk" => "text/x-awk",
        _ => "text/x-script",
    }
    .to_string()
}

/// Types `infer` cannot see (it works on magic bytes, and text has none) whose extension
/// is unambiguous. Kept short on purpose: this is the fallback for the fallback.
fn mime_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext.to_lowercase().as_str() {
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "xhtml" => "application/xhtml+xml",
        "css" => "text/css",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" => "application/json",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" => "text/typescript",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/x-yaml",
        "xml" => "application/xml",
        "sh" | "bash" | "zsh" => "application/x-shellscript",
        "py" => "text/x-python",
        "rs" => "text/x-rust",
        "go" => "text/x-go",
        "c" | "h" => "text/x-c",
        "cpp" | "cc" | "hpp" => "text/x-c++",
        "java" => "text/x-java",
        "rb" => "text/x-ruby",
        "lua" => "text/x-lua",
        "sql" => "application/sql",
        "txt" | "log" => "text/plain",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // A multi-byte character cut off by the 8 KiB boundary is still text.
        Err(error) => error.error_len().is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shebang_names_the_interpreter_through_env() {
        assert_eq!(
            shebang_interpreter(b"#!/usr/bin/env python3\nprint(1)\n"),
            Some("python3".to_string())
        );
        assert_eq!(
            shebang_interpreter(b"#!/usr/bin/env -S deno run\n"),
            Some("deno".to_string())
        );
        assert_eq!(
            shebang_interpreter(b"#!/bin/bash -e\r\n"),
            Some("bash".to_string())
        );
        assert_eq!(shebang_interpreter(b"# not a shebang"), None);
        assert_eq!(shebang_interpreter(b""), None);
    }

    #[test]
    fn shebang_decides_the_mime_type() {
        let d = detect_bytes(b"#!/usr/bin/env python3.12\n", "");
        assert_eq!(d.mime, "text/x-python");
        assert_eq!(d.shebang.as_deref(), Some("python3.12"));
        assert_eq!(detect_bytes(b"#!/bin/sh\n", "").mime, "text/x-shellscript");
        assert_eq!(detect_bytes(b"#!/usr/bin/bc\n", "").mime, "text/x-script");
    }

    #[test]
    fn magic_bytes_beat_the_extension() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(detect_bytes(png, "txt").mime, "image/png");
    }

    #[test]
    fn known_extensions_cover_what_magic_cannot() {
        assert_eq!(detect_bytes(b"# Title\n", "md").mime, "text/markdown");
        assert_eq!(detect_bytes(b"a,b\n1,2\n", "csv").mime, "text/csv");
        assert_eq!(detect_bytes(b"fn main() {}\n", "rs").mime, "text/x-rust");
    }

    #[test]
    fn plain_text_and_binary_fall_through() {
        assert_eq!(detect_bytes(b"hello world\n", "").mime, "text/plain");
        assert_eq!(
            detect_bytes(b"\x00\x01\x02binary", "").mime,
            "application/octet-stream"
        );
        // A UTF-8 sequence cut at the buffer boundary is still text.
        assert_eq!(
            detect_bytes("héllo".as_bytes()[..2].as_ref(), "").mime,
            "text/plain"
        );
    }

    #[test]
    fn empty_files_are_inode_empty_without_reading() {
        let d = detect_file(Path::new("/definitely/not/here"), "rs", true);
        assert_eq!(d.mime, "inode/empty");
    }

    #[test]
    fn an_unreadable_file_is_not_an_error() {
        let d = detect_file(Path::new("/definitely/not/here"), "rs", false);
        assert_eq!(d.mime, "text/x-rust", "the extension table still answers");
    }
}
