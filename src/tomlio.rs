//! Read/splice/write of `~/.config/yazi/yazi.toml`.
//!
//! We own only the `[opener]` and `[open]` tables. To update them we parse the existing
//! file with `toml_edit`, drop those two tables, re-serialize the remainder (which keeps
//! `[mgr]`/`[preview]`/comments/formatting verbatim), then append our freshly rendered
//! sections as text. Writes are atomic (temp + rename) and back up the prior file.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use thiserror::Error;
use toml_edit::DocumentMut;

use crate::render;
use crate::spec::Spec;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Created,
    Updated,
    InSync,
}

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error(
        "{path} already defines [opener]/[open] that differ from this spec.\n  \
         Preview with `diff`, then re-run `apply --force` to replace them (a backup is written first)."
    )]
    ManagedConflict { path: String },
}

/// Read a file, returning `None` if it does not exist.
pub fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::from(e).context(format!("reading {}", path.display()))),
    }
}

/// Produce the full new file contents: existing file with our two tables replaced
/// (or a fresh file containing only our sections when `existing` is `None`).
pub fn render_to_string(existing: Option<&str>, spec: &Spec) -> Result<String> {
    match existing {
        None => Ok(render::fragment(spec)),
        Some(text) => {
            let mut doc: DocumentMut = text
                .parse()
                .context("existing yazi.toml is not valid TOML")?;
            doc.remove("opener");
            doc.remove("open");
            let base = doc.to_string();
            let base = base.trim_end_matches(['\n', ' ', '\t']);
            let frag = render::fragment(spec);
            if base.is_empty() {
                Ok(frag)
            } else {
                Ok(format!("{base}\n\n{frag}"))
            }
        }
    }
}

/// Does this TOML text already define `[opener]` or `[open]`? (false if it doesn't parse).
pub fn has_managed_tables(text: &str) -> bool {
    match text.parse::<DocumentMut>() {
        Ok(d) => d.contains_key("opener") || d.contains_key("open"),
        Err(_) => false,
    }
}

/// Write the spec's sections into `path`. Idempotent; backs up before changing unless
/// `no_backup`. Refuses to replace pre-existing `[opener]`/`[open]` unless `force`.
pub fn apply(path: &Path, spec: &Spec, no_backup: bool, force: bool) -> Result<Outcome> {
    let existing = read_optional(path)?;
    let new_text = render_to_string(existing.as_deref(), spec)?;

    if existing.as_deref() == Some(new_text.as_str()) {
        return Ok(Outcome::InSync);
    }

    let had_managed = existing.as_deref().is_some_and(has_managed_tables);
    if had_managed && !force {
        return Err(ApplyError::ManagedConflict {
            path: path.display().to_string(),
        }
        .into());
    }

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    if existing.is_some() && !no_backup {
        let bak = backup(path)?;
        eprintln!("backed up {} -> {}", path.display(), bak.display());
    }
    atomic_write(path, &new_text)?;

    Ok(if existing.is_some() {
        Outcome::Updated
    } else {
        Outcome::Created
    })
}

pub(crate) fn backup(path: &Path) -> Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("yazi.toml");
    let bak = path.with_file_name(format!("{name}.bak-{secs}"));
    fs::copy(path, &bak).with_context(|| format!("backing up to {}", bak.display()))?;
    Ok(bak)
}

pub(crate) fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("yazi.toml");
    let tmp = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}
