use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{Association, CommandEntry, MatchRule};

#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    pub dir: PathBuf,
    pub name: String,
    pub stem: String,
    pub ext: String,
    pub is_dir: bool,
}

impl Target {
    pub fn from_path(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path)
            .with_context(|| format!("failed to resolve path {}", path.display()))?;
        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to read metadata for {}", path.display()))?;
        let is_dir = metadata.is_dir();

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

        Ok(Self {
            path,
            dir,
            name,
            stem,
            ext,
            is_dir,
        })
    }
}

pub fn matching_commands(associations: &[Association], target: &Target) -> Vec<CommandEntry> {
    let mut commands = Vec::new();
    let mut seen_labels = HashSet::new();

    for association in associations {
        if !matches_rule(&association.match_rule, target) {
            continue;
        }

        for command in &association.commands {
            let label_key = command.label.to_lowercase();
            if seen_labels.insert(label_key) {
                commands.push(command.clone());
            }
        }
    }

    commands
}

fn matches_rule(rule: &MatchRule, target: &Target) -> bool {
    let has_extensions = !rule.extensions.is_empty();
    let has_names = !rule.names.is_empty();
    let has_dirs = rule.dirs.is_some();

    if !has_extensions && !has_names && !has_dirs {
        return false;
    }

    if has_extensions
        && !rule
            .extensions
            .iter()
            .any(|extension| normalize_extension(extension) == target.ext)
    {
        return false;
    }

    if has_names
        && !rule
            .names
            .iter()
            .any(|name| matches_name(name, &target.name, &target.stem))
    {
        return false;
    }

    if let Some(matches_dirs) = rule.dirs
        && matches_dirs != target.is_dir
    {
        return false;
    }

    true
}

fn normalize_extension(extension: &str) -> String {
    extension.trim_start_matches('.').to_lowercase()
}

fn matches_name(rule_name: &str, file_name: &str, stem: &str) -> bool {
    let rule_name = rule_name.to_lowercase();
    rule_name == file_name.to_lowercase() || rule_name == stem.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Association, CommandEntry, MatchRule};

    fn command(label: &str) -> CommandEntry {
        CommandEntry {
            label: label.to_string(),
            description: String::new(),
            icon: String::new(),
            run: format!("echo {label}"),
            cwd: None,
        }
    }

    fn target() -> Target {
        Target {
            path: PathBuf::from("/tmp/thumbnail.JPG"),
            dir: PathBuf::from("/tmp"),
            name: "thumbnail.JPG".to_string(),
            stem: "thumbnail".to_string(),
            ext: "jpg".to_string(),
            is_dir: false,
        }
    }

    #[test]
    fn extension_rules_ignore_case_and_leading_dot() {
        let rule = MatchRule {
            extensions: vec![".JPG".to_string()],
            ..MatchRule::default()
        };

        assert!(matches_rule(&rule, &target()));
    }

    #[test]
    fn name_and_extension_rules_both_apply_when_present() {
        let rule = MatchRule {
            extensions: vec!["jpg".to_string()],
            names: vec!["thumbnail".to_string()],
            dirs: None,
        };

        assert!(matches_rule(&rule, &target()));
    }

    #[test]
    fn commands_are_deduped_by_label() {
        let associations = vec![
            Association {
                match_rule: MatchRule {
                    extensions: vec!["jpg".to_string()],
                    ..MatchRule::default()
                },
                commands: vec![command("Open")],
            },
            Association {
                match_rule: MatchRule {
                    names: vec!["thumbnail".to_string()],
                    ..MatchRule::default()
                },
                commands: vec![command("open"), command("Inspect")],
            },
        ];

        let commands = matching_commands(&associations, &target());

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].label, "Open");
        assert_eq!(commands[1].label, "Inspect");
    }
}
