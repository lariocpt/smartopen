//! Which commands a config offers for a target — or for several targets at once.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{
    CommandEntry, Config, ExtensionAssociation, FolderAssociation, MatchRule, UrlAssociation,
};
use crate::platform::Host;
use crate::target::Target;

/// The commands offered for `targets` on this machine.
///
/// For one target: every matching association's commands, merged in config order,
/// deduplicated by label, without the ones marked for another platform, highest
/// `priority` first. For several: only the commands ALL of them share, in the first
/// target's order — a command that cannot take every file is not offered for the set.
pub fn matching_commands(
    config: &Config,
    config_path: &Path,
    targets: &[Target],
) -> Result<Vec<CommandEntry>> {
    matching_commands_on(config, config_path, targets, Host::current())
}

/// [`matching_commands`] for an explicit host, so the platform filter is testable anywhere.
pub fn matching_commands_on(
    config: &Config,
    config_path: &Path,
    targets: &[Target],
    host: Host,
) -> Result<Vec<CommandEntry>> {
    let mut targets = targets.iter();
    let Some(first) = targets.next() else {
        return Ok(Vec::new());
    };

    let mut commands = commands_for_target(config, config_path, first, host)?;
    for target in targets {
        let shared: HashSet<String> = commands_for_target(config, config_path, target, host)?
            .into_iter()
            .map(|command| command.label.to_lowercase())
            .collect();
        commands.retain(|command| shared.contains(&command.label.to_lowercase()));
    }

    sort_by_priority(&mut commands);
    Ok(commands)
}

fn commands_for_target(
    config: &Config,
    config_path: &Path,
    target: &Target,
    host: Host,
) -> Result<Vec<CommandEntry>> {
    let mut commands = Vec::new();
    let mut seen_labels = HashSet::new();

    for association in &config.extension {
        if matches_extension_association(association, target) {
            push_unique_commands(&mut commands, &mut seen_labels, &association.commands, host);
        }
    }

    for association in &config.folder {
        if matches_folder_association(association, config_path, target)? {
            push_unique_commands(&mut commands, &mut seen_labels, &association.commands, host);
        }
    }

    for association in &config.url {
        if matches_url_association(association, target) {
            push_unique_commands(&mut commands, &mut seen_labels, &association.commands, host);
        }
    }

    for association in &config.association {
        if matches_rule(&association.match_rule, target) {
            push_unique_commands(&mut commands, &mut seen_labels, &association.commands, host);
        }
    }

    Ok(commands)
}

/// The shortcuts offered on this machine, highest priority first, config order otherwise.
pub fn shortcuts_here(config: &Config) -> Vec<CommandEntry> {
    shortcuts_on(config, Host::current())
}

pub fn shortcuts_on(config: &Config, host: Host) -> Vec<CommandEntry> {
    let mut shortcuts: Vec<CommandEntry> = config
        .shortcut
        .iter()
        .filter(|command| applies_on(command, host))
        .cloned()
        .collect();
    sort_by_priority(&mut shortcuts);
    shortcuts
}

/// Higher `priority` first; the sort is stable, so equal priorities keep config order.
fn sort_by_priority(commands: &mut [CommandEntry]) {
    commands.sort_by_key(|command| std::cmp::Reverse(command.priority));
}

/// The one command to run without a menu: exactly one of the offered commands is marked
/// `default`. Two defaults is ambiguity, and ambiguity gets a menu.
pub fn default_command(commands: &[CommandEntry]) -> Option<&CommandEntry> {
    let mut defaults = commands.iter().filter(|command| command.default);
    let first = defaults.next()?;
    defaults.next().is_none().then_some(first)
}

fn applies_on(command: &CommandEntry, host: Host) -> bool {
    command
        .platform
        .is_none_or(|platform| platform.applies_on(host))
}

fn push_unique_commands(
    commands: &mut Vec<CommandEntry>,
    seen_labels: &mut HashSet<String>,
    new_commands: &[CommandEntry],
    host: Host,
) {
    for command in new_commands {
        if !applies_on(command, host) {
            continue;
        }
        let label_key = command.label.to_lowercase();
        if seen_labels.insert(label_key) {
            commands.push(command.clone());
        }
    }
}

// Extension and folder associations are about things on disk; a URL is neither. Generic
// `[[association]]` rules can still reach URLs through `mime = "x-scheme-handler/*"`.

fn matches_extension_association(association: &ExtensionAssociation, target: &Target) -> bool {
    if target.is_dir || target.is_url() || association.extensions.is_empty() {
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

fn matches_url_association(association: &UrlAssociation, target: &Target) -> bool {
    let Some(url) = &target.url else {
        return false;
    };

    let scheme_ok = association.schemes.is_empty()
        || association
            .schemes
            .iter()
            .any(|scheme| scheme.eq_ignore_ascii_case(&url.scheme));
    let host_ok = association.hosts.is_empty()
        || association
            .hosts
            .iter()
            .any(|pattern| wildcard_matches(&pattern.to_lowercase(), &url.host));

    scheme_ok && host_ok
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
    let has_mime = !rule.mime.is_empty();
    let has_shebang = !rule.shebang.is_empty();

    if !has_extensions
        && !has_names
        && !has_name_patterns
        && !has_dirs
        && !has_empty
        && !has_mime
        && !has_shebang
    {
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

    if has_mime
        && !rule
            .mime
            .iter()
            .any(|pattern| wildcard_matches(&pattern.to_lowercase(), &target.mime.to_lowercase()))
    {
        return false;
    }

    if has_shebang {
        let Some(interpreter) = &target.shebang else {
            return false;
        };
        if !rule
            .shebang
            .iter()
            .any(|pattern| wildcard_matches(&pattern.to_lowercase(), &interpreter.to_lowercase()))
        {
            return false;
        }
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

/// `*` and `?` globbing, byte-wise. Case folding is the caller's job.
pub fn wildcard_matches(pattern: &str, value: &str) -> bool {
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
        Association, CommandEntry, Config, ExtensionAssociation, FolderAssociation,
    };
    use crate::platform::Platform;

    fn command(label: &str) -> CommandEntry {
        CommandEntry {
            label: label.to_string(),
            run: format!("echo {label}"),
            ..CommandEntry::default()
        }
    }

    fn command_for(label: &str, platform: Platform) -> CommandEntry {
        CommandEntry {
            platform: Some(platform),
            ..command(label)
        }
    }

    fn file_target() -> Target {
        Target::fake_file("/tmp/thumbnail.JPG")
    }

    fn folder_target() -> Target {
        Target::fake_dir("/tmp/project")
    }

    fn generic(rule: MatchRule, commands: Vec<CommandEntry>) -> Association {
        Association {
            match_rule: rule,
            commands,
        }
    }

    fn labels(commands: Vec<CommandEntry>) -> Vec<String> {
        commands.into_iter().map(|c| c.label).collect()
    }

    fn matching(config: &Config, targets: &[Target]) -> Vec<String> {
        labels(
            matching_commands_on(config, Path::new("/tmp/config.toml"), targets, Host::Linux)
                .expect("matching should not fail"),
        )
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
            ..MatchRule::default()
        };

        assert!(matches_rule(&rule, &file_target()));
    }

    #[test]
    fn commands_are_deduped_by_label() {
        let config = Config {
            association: vec![
                generic(
                    MatchRule {
                        extensions: vec!["jpg".to_string()],
                        ..MatchRule::default()
                    },
                    vec![command("Open")],
                ),
                generic(
                    MatchRule {
                        names: vec!["thumbnail".to_string()],
                        ..MatchRule::default()
                    },
                    vec![command("open"), command("Inspect")],
                ),
            ],
            ..Config::default()
        };

        assert_eq!(matching(&config, &[file_target()]), ["Open", "Inspect"]);
    }

    #[test]
    fn extension_associations_match_files_by_extension() {
        let config = Config {
            extension: vec![ExtensionAssociation {
                extensions: vec!["jpg".to_string()],
                names: Vec::new(),
                commands: vec![command("Preview image")],
            }],
            ..Config::default()
        };

        assert_eq!(matching(&config, &[file_target()]), ["Preview image"]);
    }

    #[test]
    fn folder_associations_match_directories() {
        let config = Config {
            folder: vec![FolderAssociation {
                names: Vec::new(),
                paths: Vec::new(),
                commands: vec![command("Open folder")],
            }],
            ..Config::default()
        };

        assert_eq!(matching(&config, &[folder_target()]), ["Open folder"]);
        assert!(matching(&config, &[file_target()]).is_empty());
    }

    #[test]
    fn named_folder_associations_filter_by_folder_name() {
        let config = Config {
            folder: vec![FolderAssociation {
                names: vec!["project".to_string()],
                paths: Vec::new(),
                commands: vec![command("Project menu")],
            }],
            ..Config::default()
        };

        assert_eq!(matching(&config, &[folder_target()]), ["Project menu"]);
    }

    #[test]
    fn commands_for_another_platform_are_not_offered() {
        let config = Config {
            extension: vec![ExtensionAssociation {
                extensions: vec!["jpg".to_string()],
                names: Vec::new(),
                commands: vec![
                    command("Everywhere"),
                    command_for("Finder", Platform::Macos),
                    command_for("Explorer", Platform::Windows),
                    command_for("xdg-open", Platform::Unix),
                ],
            }],
            shortcut: vec![
                command("Shell"),
                command_for("PowerShell", Platform::Windows),
            ],
            ..Config::default()
        };
        let on = |host: Host| {
            labels(
                matching_commands_on(&config, Path::new("/tmp/c.toml"), &[file_target()], host)
                    .unwrap(),
            )
        };

        assert_eq!(on(Host::Macos), ["Everywhere", "Finder", "xdg-open"]);
        assert_eq!(on(Host::Windows), ["Everywhere", "Explorer"]);
        assert_eq!(labels(shortcuts_on(&config, Host::Linux)), ["Shell"]);
        assert_eq!(
            labels(shortcuts_on(&config, Host::Windows)),
            ["Shell", "PowerShell"]
        );
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

    #[test]
    fn mime_and_shebang_rules_see_what_extensions_cannot() {
        let mut script = Target::fake_file("/tmp/deploy");
        script.mime = "text/x-python".to_string();
        script.shebang = Some("python3".to_string());

        let by_mime = MatchRule {
            mime: vec!["text/*".to_string()],
            ..MatchRule::default()
        };
        let by_shebang = MatchRule {
            shebang: vec!["python*".to_string()],
            ..MatchRule::default()
        };
        let wrong_shebang = MatchRule {
            shebang: vec!["bash".to_string()],
            ..MatchRule::default()
        };
        let needs_shebang = MatchRule {
            shebang: vec!["*".to_string()],
            ..MatchRule::default()
        };

        assert!(matches_rule(&by_mime, &script));
        assert!(matches_rule(&by_shebang, &script));
        assert!(!matches_rule(&wrong_shebang, &script));
        assert!(
            !matches_rule(&needs_shebang, &file_target()),
            "no shebang, no match"
        );
        assert!(matches_rule(
            &MatchRule {
                mime: vec!["inode/directory".to_string()],
                ..MatchRule::default()
            },
            &folder_target()
        ));
    }

    #[test]
    fn url_associations_match_by_scheme_and_host_glob() {
        let config = Config {
            url: vec![
                UrlAssociation {
                    schemes: vec!["https".to_string()],
                    hosts: vec!["*.github.com".to_string(), "github.com".to_string()],
                    commands: vec![command("gh")],
                },
                UrlAssociation {
                    schemes: Vec::new(),
                    hosts: Vec::new(),
                    commands: vec![command("Browser")],
                },
            ],
            extension: vec![ExtensionAssociation {
                extensions: vec!["pdf".to_string()],
                names: Vec::new(),
                commands: vec![command("zathura")],
            }],
            association: vec![generic(
                MatchRule {
                    mime: vec!["x-scheme-handler/*".to_string()],
                    ..MatchRule::default()
                },
                vec![command("Any handler")],
            )],
            ..Config::default()
        };

        let github = Target::from_arg("https://github.com/lariocpt/smartopen").unwrap();
        assert_eq!(
            matching(&config, &[github]),
            ["gh", "Browser", "Any handler"]
        );

        let pdf = Target::from_arg("https://example.com/report.pdf").unwrap();
        assert_eq!(
            matching(&config, &[pdf]),
            ["Browser", "Any handler"],
            "extension associations are for files on disk, not URLs"
        );

        let mail = Target::from_arg("mailto:x@y.z").unwrap();
        assert_eq!(matching(&config, &[mail]), ["Browser", "Any handler"]);
    }

    #[test]
    fn several_targets_get_only_the_commands_they_share() {
        let config = Config {
            extension: vec![
                ExtensionAssociation {
                    extensions: vec!["jpg".to_string()],
                    names: Vec::new(),
                    commands: vec![command("Edit"), command("Preview image")],
                },
                ExtensionAssociation {
                    extensions: vec!["md".to_string()],
                    names: Vec::new(),
                    commands: vec![command("Render"), command("edit")],
                },
            ],
            ..Config::default()
        };
        let jpg = file_target();
        let md = Target::fake_file("/tmp/notes.md");

        assert_eq!(matching(&config, &[jpg.clone(), md.clone()]), ["Edit"]);
        assert_eq!(
            matching(&config, &[md, jpg]),
            ["edit"],
            "first target's spelling and order"
        );
        assert!(matching(&config, &[]).is_empty());
    }

    #[test]
    fn priority_orders_and_a_lone_default_is_found() {
        let low = CommandEntry {
            priority: -1,
            ..command("Low")
        };
        let high = CommandEntry {
            priority: 10,
            default: true,
            ..command("High")
        };
        let config = Config {
            extension: vec![ExtensionAssociation {
                extensions: vec!["jpg".to_string()],
                names: Vec::new(),
                commands: vec![low, command("Plain"), high.clone()],
            }],
            shortcut: vec![
                command("B"),
                CommandEntry {
                    priority: 1,
                    ..command("A")
                },
            ],
            ..Config::default()
        };

        assert_eq!(
            matching(&config, &[file_target()]),
            ["High", "Plain", "Low"]
        );
        assert_eq!(labels(shortcuts_on(&config, Host::Linux)), ["A", "B"]);

        let offered =
            matching_commands_on(&config, Path::new("/c"), &[file_target()], Host::Linux).unwrap();
        assert_eq!(
            default_command(&offered).map(|c| &c.label),
            Some(&"High".to_string())
        );

        let two_defaults = vec![
            high.clone(),
            CommandEntry {
                default: true,
                ..command("Other")
            },
        ];
        assert!(
            default_command(&two_defaults).is_none(),
            "two defaults is a menu"
        );
        assert!(default_command(&[command("None")]).is_none());
    }
}
