//! The thing being opened: a file, a directory, or a URL.
//!
//! One struct rather than an enum, because nearly every consumer wants the same handful
//! of strings (`{path}`, `{name}`, `{ext}` …) whatever the kind; the `kind` is there for
//! the matcher, and `url` for the URL-only placeholders.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::mime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    File,
    Dir,
    Url,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlParts {
    pub raw: String,
    pub scheme: String,
    pub host: String,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub kind: TargetKind,
    /// The canonical path — or, for a URL, the URL itself, so `{path}` still means
    /// "the thing you pointed at".
    pub path: PathBuf,
    pub dir: PathBuf,
    pub name: String,
    pub stem: String,
    pub ext: String,
    pub is_dir: bool,
    pub is_empty: bool,
    /// yazi's vocabulary: `inode/directory`, `text/x-python`, `x-scheme-handler/https` …
    pub mime: String,
    /// The interpreter named by a `#!` line, for files that have one.
    pub shebang: Option<String>,
    pub url: Option<UrlParts>,
}

impl Target {
    /// A command-line argument: a URL if it starts with a scheme, otherwise a path.
    pub fn from_arg(arg: &str) -> Result<Self> {
        match parse_url(arg) {
            Some(url) => Ok(Self::from_url(url)),
            None => Self::from_path(Path::new(arg)),
        }
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let path = canonicalize(path)
            .with_context(|| format!("failed to resolve path {}", path.display()))?;
        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to read metadata for {}", path.display()))?;
        let is_dir = metadata.is_dir();
        let is_empty = metadata.is_file() && metadata.len() == 0;

        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        let ext = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let dir = if is_dir {
            path.clone()
        } else {
            path.parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        };

        let (mime, shebang) = if is_dir {
            ("inode/directory".to_string(), None)
        } else {
            let detected = mime::detect_file(&path, &ext, is_empty);
            (detected.mime, detected.shebang)
        };

        Ok(Self {
            kind: if is_dir {
                TargetKind::Dir
            } else {
                TargetKind::File
            },
            path,
            dir,
            name,
            stem,
            ext,
            is_dir,
            is_empty,
            mime,
            shebang,
            url: None,
        })
    }

    fn from_url(url: UrlParts) -> Self {
        // The URL's path component supplies name/stem/ext, so `https://x/a/report.pdf`
        // can still match an extension rule if a config wants that.
        let after_scheme = &url.raw[url.scheme.len() + 1..];
        let (has_authority, rest) = match after_scheme.strip_prefix("//") {
            Some(rest) => (true, rest),
            None => (false, after_scheme),
        };
        let rest = rest.split_once(['?', '#']).map_or(rest, |(p, _)| p);
        // With an authority (`https://host/a/b`), the path is what follows the host;
        // without one (`mailto:someone@x.y`), the whole remainder is the name.
        let path_part = if has_authority {
            rest.split_once('/').map_or("", |(_, path)| path)
        } else {
            rest
        };
        let last_segment = path_part.rsplit('/').next().unwrap_or("");

        // A bare `https://example.com` has no path segment: the host is the name, and
        // neither its dots nor `mailto:` addresses are an extension.
        let (last, stem, ext) = if last_segment.is_empty() {
            (url.host.clone(), url.host.clone(), String::new())
        } else if !has_authority {
            (
                last_segment.to_string(),
                last_segment.to_string(),
                String::new(),
            )
        } else {
            let (stem, ext) = match last_segment.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_lowercase()),
                _ => (last_segment.to_string(), String::new()),
            };
            (last_segment.to_string(), stem, ext)
        };

        Self {
            kind: TargetKind::Url,
            path: PathBuf::from(&url.raw),
            dir: PathBuf::new(),
            name: last,
            stem,
            ext,
            is_dir: false,
            is_empty: false,
            mime: format!("x-scheme-handler/{}", url.scheme),
            shebang: None,
            url: Some(url),
        }
    }

    pub fn is_url(&self) -> bool {
        self.kind == TargetKind::Url
    }

    /// Every target as a `file://` or its own URL — what `{url}` renders for a file.
    pub fn as_url_string(&self) -> String {
        match &self.url {
            Some(url) => url.raw.clone(),
            None => {
                let raw = self.path.display().to_string();
                if raw.starts_with('/') {
                    format!("file://{raw}")
                } else {
                    // Windows: file:///C:/… wants forward slashes.
                    format!("file:///{}", raw.replace('\\', "/"))
                }
            }
        }
    }

    /// A file target that does not touch the filesystem, for tests in other modules.
    #[cfg(test)]
    pub fn fake_file(path: &str) -> Self {
        let path = PathBuf::from(path);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        Self {
            kind: TargetKind::File,
            dir: path.parent().unwrap().to_path_buf(),
            path,
            name,
            stem,
            ext,
            is_dir: false,
            is_empty: false,
            mime: "text/plain".to_string(),
            shebang: None,
            url: None,
        }
    }

    #[cfg(test)]
    pub fn fake_dir(path: &str) -> Self {
        let path = PathBuf::from(path);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Self {
            kind: TargetKind::Dir,
            dir: path.clone(),
            path,
            stem: name.clone(),
            name,
            ext: String::new(),
            is_dir: true,
            is_empty: false,
            mime: "inode/directory".to_string(),
            shebang: None,
            url: None,
        }
    }
}

/// Resolve to an absolute path with symlinks followed — the form every placeholder uses.
/// On Windows `std::fs::canonicalize` returns `\\?\C:\…`, which most programs reject on
/// their command line; `dunce` gives back the plain `C:\…` spelling wherever it is valid.
fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    #[cfg(windows)]
    {
        dunce::canonicalize(path)
    }
    #[cfg(not(windows))]
    {
        fs::canonicalize(path)
    }
}

/// `scheme:rest` with a scheme of two or more characters, so `C:\Users` is never mistaken
/// for one. Returns `None` for anything that is not URL-shaped.
pub fn parse_url(arg: &str) -> Option<UrlParts> {
    let colon = arg.find(':')?;
    let scheme = &arg[..colon];
    if scheme.len() < 2
        || !scheme.chars().next()?.is_ascii_alphabetic()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    {
        return None;
    }
    // A bare `scheme:` with nothing after it is not a URL either.
    let rest = &arg[colon + 1..];
    if rest.is_empty() {
        return None;
    }

    let host = rest
        .strip_prefix("//")
        .map(|authority| {
            authority
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("")
                .rsplit('@')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();

    Some(UrlParts {
        raw: arg.to_string(),
        scheme: scheme.to_lowercase(),
        host: host.to_lowercase(),
    })
}

/// `Target::from_arg` for a list, failing on the first argument that is neither a URL nor
/// an existing path.
pub fn targets_from_args(args: &[String]) -> Result<Vec<Target>> {
    if args.is_empty() {
        bail!("no target given");
    }
    args.iter().map(|arg| Target::from_arg(arg)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_need_a_scheme_of_two_or_more_characters() {
        assert!(parse_url("https://example.com/a").is_some());
        assert!(parse_url("mailto:someone@example.com").is_some());
        assert!(
            parse_url(r"C:\Users\lario\file.rs").is_none(),
            "drive letters are paths"
        );
        assert!(parse_url("./notes:today.md").is_none());
        assert!(parse_url("https:").is_none());
        assert!(parse_url("plain-file.txt").is_none());
    }

    #[test]
    fn url_parts_are_lowercased_and_stripped_of_credentials_and_ports() {
        let url = parse_url("HTTPS://user:pw@Example.COM:8443/path/to/report.PDF?x=1#f").unwrap();
        assert_eq!(url.scheme, "https");
        assert_eq!(url.host, "example.com");
        assert_eq!(
            url.raw,
            "HTTPS://user:pw@Example.COM:8443/path/to/report.PDF?x=1#f"
        );
    }

    #[test]
    fn a_url_target_exposes_path_pieces_and_a_scheme_handler_mime() {
        let target = Target::from_arg("https://example.com/docs/report.PDF?dl=1").unwrap();
        assert_eq!(target.kind, TargetKind::Url);
        assert_eq!(target.name, "report.PDF");
        assert_eq!(target.stem, "report");
        assert_eq!(target.ext, "pdf");
        assert_eq!(target.mime, "x-scheme-handler/https");
        assert_eq!(
            target.path,
            PathBuf::from("https://example.com/docs/report.PDF?dl=1")
        );
        assert_eq!(
            target.as_url_string(),
            "https://example.com/docs/report.PDF?dl=1"
        );

        let bare = Target::from_arg("https://example.com").unwrap();
        assert_eq!(bare.name, "example.com");
        assert_eq!(bare.ext, "");
    }

    #[test]
    fn mailto_has_no_host_but_still_a_name() {
        let target = Target::from_arg("mailto:someone@example.com").unwrap();
        assert_eq!(target.url.as_ref().unwrap().host, "");
        assert_eq!(target.name, "someone@example.com");
        assert_eq!(target.mime, "x-scheme-handler/mailto");
    }

    #[test]
    fn a_file_renders_as_a_file_url() {
        let file = Target::fake_file("/tmp/a b.txt");
        assert_eq!(file.as_url_string(), "file:///tmp/a b.txt");
    }

    #[test]
    fn a_missing_path_is_an_error_not_a_url() {
        assert!(Target::from_arg("/definitely/not/here.rs").is_err());
        assert!(targets_from_args(&[]).is_err());
    }
}
