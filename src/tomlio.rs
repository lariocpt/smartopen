//! Read/splice/write of `~/.config/yazi/yazi.toml`.
//!
//! We own only the `[opener]` and `[open]` tables. To update them we parse the existing
//! file with `toml_edit`, swap those two tables for freshly rendered ones in the positions
//! they had (the rest — `[mgr]`/`[preview]`/comments/formatting — stays verbatim), and
//! re-serialize. Writes are atomic (temp + rename) and back up the prior file.

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
///
/// Each table goes back where its predecessor was, so a `[preview]` that followed
/// `[open]` still follows it — the old remove-and-append moved every later table above
/// ours, and a `diff` showed untouched tables shifting. A table the file never had goes
/// at the end.
pub fn render_to_string(existing: Option<&str>, spec: &Spec) -> Result<String> {
    let Some(text) = existing else {
        return Ok(render::fragment(spec));
    };
    let mut doc: DocumentMut = text
        .parse()
        .context("existing yazi.toml is not valid TOML")?;
    let mut fresh: DocumentMut = render::fragment(spec)
        .parse()
        .context("rendered [opener]/[open] fragment is not valid TOML")?;
    let mut next = max_position(&doc).map_or(0, |p| p + 1);
    for key in ["opener", "open"] {
        let position = doc
            .get(key)
            .and_then(|item| item.as_table())
            .and_then(|table| table.position());
        doc.remove(key);
        let Some(mut item) = fresh.remove(key) else {
            continue;
        };
        if let Some(table) = item.as_table_mut() {
            let assigned = position.unwrap_or_else(|| {
                next += 1;
                next - 1
            });
            table.set_position(Some(assigned));
            // A blank line before the table when another table precedes it, as between
            // any two tables in a file — and none when it is the first, so a file
            // created from the bare fragment re-renders to itself and `check` is quiet.
            let has_predecessor = positions(&doc).into_iter().any(|p| p < assigned);
            let prefix = table
                .decor()
                .prefix()
                .and_then(|p| p.as_str())
                .unwrap_or_default()
                .to_string();
            if has_predecessor && !prefix.starts_with('\n') {
                table.decor_mut().set_prefix(format!("\n{prefix}"));
            }
        }
        doc.insert(key, item);
    }
    Ok(doc.to_string())
}

/// The highest table position in the document, subtables included, so a new table can
/// be placed after every existing one.
fn max_position(table: &toml_edit::Table) -> Option<isize> {
    positions(table).into_iter().max()
}

/// Every table position in the document, subtables and arrays of tables included.
fn positions(table: &toml_edit::Table) -> Vec<isize> {
    let mut out: Vec<isize> = table.position().into_iter().collect();
    for (_, item) in table.iter() {
        if let Some(sub) = item.as_table() {
            out.extend(positions(sub));
        }
        if let Some(array) = item.as_array_of_tables() {
            for sub in array.iter() {
                out.extend(positions(sub));
            }
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    const EXISTING: &str = "[mgr]\nratio = [1, 4, 3]\n\n[opener]\nedit = [{ run = 'vi %s', block = true }]\n\n[open]\nrules = [{ url = '*', use = 'edit' }]\n\n[preview]\ntab_size = 4\n";

    fn offset(text: &str, needle: &str) -> usize {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} missing:\n{text}"))
    }

    #[test]
    fn managed_tables_go_back_where_they_were() {
        // The old remove-and-append moved `[preview]` above the rewritten tables, and a
        // `diff` showed a table nobody touched as changed.
        let out = render_to_string(Some(EXISTING), &Spec::builtin()).unwrap();
        assert!(offset(&out, "[mgr]") < offset(&out, "\n[opener]"), "{out}");
        assert!(
            offset(&out, "\n[opener]") < offset(&out, "\n[open]\n"),
            "{out}"
        );
        assert!(
            offset(&out, "\n[open]\n") < offset(&out, "[preview]"),
            "{out}"
        );
        assert!(out.contains("ratio = [1, 4, 3]") && out.contains("tab_size = 4"));
        assert!(!out.contains("vi %s"), "{out}");
        out.parse::<DocumentMut>().expect("valid TOML");
        // Applying to its own output changes nothing.
        assert_eq!(render_to_string(Some(&out), &Spec::builtin()).unwrap(), out);
    }

    #[test]
    fn tables_the_file_never_had_go_at_the_end() {
        let existing = "[mgr]\nratio = [1, 4, 3]\n\n[preview]\ntab_size = 4\n";
        let out = render_to_string(Some(existing), &Spec::builtin()).unwrap();
        assert!(
            offset(&out, "[preview]") < offset(&out, "\n[opener]"),
            "{out}"
        );
        assert!(
            offset(&out, "\n[opener]") < offset(&out, "\n[open]\n"),
            "{out}"
        );
        assert_eq!(render_to_string(Some(&out), &Spec::builtin()).unwrap(), out);
    }
}
