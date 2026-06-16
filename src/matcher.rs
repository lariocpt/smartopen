use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{CommandEntry, Config, ExtensionAssociation, FolderAssociation, MatchRule};

#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    pub dir: PathBuf,
    pub name: String,
    pub stem: String,
    pub ext: String,
    pub is_dir: bool,
    pub is_empty: bool,
}

impl Target {
    pub fn from_path(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path)
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

        Ok(Self {
            path,
            dir,
            name,
            stem,
            ext,
            is_dir,
            is_empty,
        })
    }
}

pub fn matching_commands(
    config: &Config,
    config_path: &Path,
    target: &Target,
) -> Result<Vec<CommandEntry>> {
    let mut commands = Vec::new();
    let mut seen_labels = HashSet::new();

    for association in &config.extension {
        if matches_extension_association(association, target) {
            push_unique_commands(&mut commands, &mut seen_labels, &association.commands);
        }
    }

    for association in &config.folder {
        if matches_folder_association(association, config_path, target)? {
            push_unique_commands(&mut commands, &mut seen_labels, &association.commands);
        }
    }

    for association in &config.association {
        if matches_rule(&association.match_rule, target) {
            push_unique_commands(&mut commands, &mut seen_labels, &association.commands);
        }
    }

    Ok(commands)
}

fn push_unique_commands(
    commands: &mut Vec<CommandEntry>,
    seen_labels: &mut HashSet<String>,
    new_commands: &[CommandEntry],
) {
    for command in new_commands {
        let label_key = command.label.to_lowercase();
        if seen_labels.insert(label_key) {
            commands.push(command.clone());
        }
    }
}

fn matches_extension_association(association: &ExtensionAssociation, target: &Target) -> bool {
    if target.is_dir || association.extensions.is_empty() {
        return false;
    }

    if !association
        .extensions
        .iter()
        .any(|extension| normalize_extension(extension) == target.ext)
    {
        return false;
    }

    association.names.is_empty()
        || association
            .names
            .iter()
            .any(|name| matches_name(name, &target.name, &target.stem))
}

fn matches_folder_association(
    association: &FolderAssociation,
    config_path: &Path,
    target: &Target,
) -> Result<bool> {
    if !target.is_dir {
        return Ok(false);
    }

    if association.names.is_empty() && association.paths.is_empty() {
        return Ok(true);
    }

    if !association.names.is_empty()
        && !association
            .names
            .iter()
            .any(|name| matches_name(name, &target.name, &target.stem))
    {
        return Ok(false);
    }

    if association.paths.is_empty() {
        return Ok(true);
    }

    for path in &association.paths {
        let path = resolve_config_relative_path(config_path, path)?;
        let path = fs::canonicalize(&path).unwrap_or(path);
        if path == target.path {
            return Ok(true);
        }
    }

    Ok(false)
}

fn resolve_config_relative_path(config_path: &Path, path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path)
        .with_context(|| format!("failed to expand folder association path '{path}'"))?
        .into_owned();
    let path = PathBuf::from(expanded);

    if path.is_absolute() {
        return Ok(path);
    }

    Ok(config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path))
}

fn matches_rule(rule: &MatchRule, target: &Target) -> bool {
    let has_extensions = !rule.extensions.is_empty();
    let has_names = !rule.names.is_empty();
    let has_name_patterns = !rule.name_patterns.is_empty();
    let has_dirs = rule.dirs.is_some();
    let has_empty = rule.empty.is_some();

    if !has_extensions && !has_names && !has_name_patterns && !has_dirs && !has_empty {
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

    if has_name_patterns
        && !rule
            .name_patterns
            .iter()
            .any(|pattern| matches_name_pattern(pattern, &target.name, &target.stem))
    {
        return false;
    }

    if let Some(matches_dirs) = rule.dirs
        && matches_dirs != target.is_dir
    {
        return false;
    }

    if let Some(matches_empty) = rule.empty
        && matches_empty != target.is_empty
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

fn matches_name_pattern(pattern: &str, file_name: &str, stem: &str) -> bool {
    let pattern = pattern.to_lowercase();
    wildcard_matches(&pattern, &file_name.to_lowercase())
        || wildcard_matches(&pattern, &stem.to_lowercase())
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    wildcard_matches_bytes(pattern.as_bytes(), value.as_bytes())
}

fn wildcard_matches_bytes(pattern: &[u8], value: &[u8]) -> bool {
    match (pattern, value) {
        ([], []) => true,
        ([], _) => false,
        ([b'*', rest @ ..], []) => wildcard_matches_bytes(rest, value),
        ([b'*', rest @ ..], [_, value_rest @ ..]) => {
            wildcard_matches_bytes(rest, value) || wildcard_matches_bytes(pattern, value_rest)
        }
        ([b'?', rest_pattern @ ..], [_, rest_value @ ..]) => {
            wildcard_matches_bytes(rest_pattern, rest_value)
        }
        ([pattern_first, rest_pattern @ ..], [value_first, rest_value @ ..])
            if pattern_first == value_first =>
        {
            wildcard_matches_bytes(rest_pattern, rest_value)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        Association, CommandEntry, Config, ExtensionAssociation, FolderAssociation, MatchRule,
        MenuConfig,
    };

    fn command(label: &str) -> CommandEntry {
        CommandEntry {
            label: label.to_string(),
            description: String::new(),
            icon: String::new(),
            run: format!("echo {label}"),
            cwd: None,
        }
    }

    fn file_target() -> Target {
        Target {
            path: PathBuf::from("/tmp/thumbnail.JPG"),
            dir: PathBuf::from("/tmp"),
            name: "thumbnail.JPG".to_string(),
            stem: "thumbnail".to_string(),
            ext: "jpg".to_string(),
            is_dir: false,
            is_empty: false,
        }
    }

    fn folder_target() -> Target {
        Target {
            path: PathBuf::from("/tmp/project"),
            dir: PathBuf::from("/tmp/project"),
            name: "project".to_string(),
            stem: "project".to_string(),
            ext: String::new(),
            is_dir: true,
            is_empty: false,
        }
    }

    fn config() -> Config {
        Config {
            menu: MenuConfig::default(),
            extension: Vec::new(),
            folder: Vec::new(),
            association: Vec::new(),
            shortcut: Vec::new(),
        }
    }

    #[test]
    fn extension_rules_ignore_case_and_leading_dot() {
        let rule = MatchRule {
            extensions: vec![".JPG".to_string()],
            ..MatchRule::default()
        };

        assert!(matches_rule(&rule, &file_target()));
    }

    #[test]
    fn name_and_extension_rules_both_apply_when_present() {
        let rule = MatchRule {
            extensions: vec!["jpg".to_string()],
            names: vec!["thumbnail".to_string()],
            name_patterns: Vec::new(),
            dirs: None,
            empty: None,
        };

        assert!(matches_rule(&rule, &file_target()));
    }

    #[test]
    fn commands_are_deduped_by_label() {
        let mut config = config();
        config.association = vec![
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

        let commands = matching_commands(&config, Path::new("/tmp/config.toml"), &file_target())
            .expect("commands should match");

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].label, "Open");
        assert_eq!(commands[1].label, "Inspect");
    }

    #[test]
    fn extension_associations_match_files_by_extension() {
        let mut config = config();
        config.extension = vec![ExtensionAssociation {
            extensions: vec!["jpg".to_string()],
            names: Vec::new(),
            commands: vec![command("Preview image")],
        }];

        let commands = matching_commands(&config, Path::new("/tmp/config.toml"), &file_target())
            .expect("commands should match");

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].label, "Preview image");
    }

    #[test]
    fn folder_associations_match_directories() {
        let mut config = config();
        config.folder = vec![FolderAssociation {
            names: Vec::new(),
            paths: Vec::new(),
            commands: vec![command("Open folder")],
        }];

        let commands = matching_commands(&config, Path::new("/tmp/config.toml"), &folder_target())
            .expect("commands should match");

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].label, "Open folder");
    }

    #[test]
    fn named_folder_associations_filter_by_folder_name() {
        let mut config = config();
        config.folder = vec![FolderAssociation {
            names: vec!["project".to_string()],
            paths: Vec::new(),
            commands: vec![command("Project menu")],
        }];

        let commands = matching_commands(&config, Path::new("/tmp/config.toml"), &folder_target())
            .expect("commands should match");

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].label, "Project menu");
    }

    #[test]
    fn generic_rules_match_name_patterns() {
        let rule = MatchRule {
            name_patterns: vec!["*.jpg".to_string(), ".env.*".to_string()],
            dirs: Some(false),
            ..MatchRule::default()
        };

        assert!(matches_rule(&rule, &file_target()));
    }

    #[test]
    fn generic_rules_can_match_empty_files() {
        let mut target = file_target();
        target.is_empty = true;
        let rule = MatchRule {
            empty: Some(true),
            dirs: Some(false),
            ..MatchRule::default()
        };

        assert!(matches_rule(&rule, &target));
    }
}
