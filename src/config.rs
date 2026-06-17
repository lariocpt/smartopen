use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use directories::BaseDirs;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub menu: MenuConfig,
    #[serde(default)]
    pub extension: Vec<ExtensionAssociation>,
    #[serde(default)]
    pub folder: Vec<FolderAssociation>,
    #[serde(default)]
    pub association: Vec<Association>,
    #[serde(default)]
    pub shortcut: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MenuConfig {
    #[serde(default)]
    pub art_file: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Association {
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionAssociation {
    pub extensions: Vec<String>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FolderAssociation {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchRule {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub name_patterns: Vec<String>,
    #[serde(default)]
    pub dirs: Option<bool>,
    #[serde(default)]
    pub empty: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandEntry {
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    pub run: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Launch detached (fire-and-forget): no wait, no inherited stdio. For GUI apps that
    /// shouldn't block the menu or surface a non-zero exit (the opener's `orphan`).
    #[serde(default)]
    pub detach: bool,
}

pub const DEFAULT_MENU_ART: &str = include_str!("../examples/art/default.txt");
pub const SAMPLE_CONFIG: &str = include_str!("../examples/config.toml");

pub fn default_config_path() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().ok_or_else(|| anyhow!("could not determine home directory"))?;
    Ok(base_dirs.config_dir().join("opn").join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        bail!(
            "no config found at {}\ncreate one with: opn --init-config",
            path.display()
        );
    }

    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read config at {}", path.display()))?;
    let config = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config at {}", path.display()))?;

    Ok(config)
}

pub fn init_config(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("config already exists at {}", path.display());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create config at {}", path.display()))?;
    file.write_all(SAMPLE_CONFIG.as_bytes())
        .with_context(|| format!("failed to write config at {}", path.display()))?;

    Ok(())
}

pub fn load_menu_art(config: &Config, config_path: &Path) -> Result<String> {
    let Some(art_file) = config.menu.art_file.as_deref() else {
        return Ok(DEFAULT_MENU_ART.to_string());
    };

    let art_path = resolve_config_relative_path(config_path, art_file)?;
    fs::read_to_string(&art_path)
        .with_context(|| format!("failed to read menu art at {}", art_path.display()))
}

fn resolve_config_relative_path(config_path: &Path, path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path)
        .with_context(|| format!("failed to expand path '{path}'"))?
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

pub fn describe_config(config: &Config, path: &Path) -> String {
    let mut output = String::new();

    output.push_str(&format!("Config: {}\n", path.display()));
    output.push_str("\nMenu:\n");
    if let Some(art_file) = &config.menu.art_file {
        output.push_str(&format!("  art_file: {art_file}\n"));
    } else {
        output.push_str("  art_file: (built-in default)\n");
    }

    output.push_str("\nExtension Associations:\n");
    if config.extension.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for (index, association) in config.extension.iter().enumerate() {
            output.push_str(&format!(
                "  {}. extensions={:?}{}\n",
                index + 1,
                association.extensions,
                describe_optional_names(&association.names)
            ));
            for command in &association.commands {
                output.push_str(&format!(
                    "     - {}{}\n",
                    command.label,
                    describe_command_details(command)
                ));
            }
        }
    }

    output.push_str("\nFolder Associations:\n");
    if config.folder.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for (index, association) in config.folder.iter().enumerate() {
            output.push_str(&format!(
                "  {}. {}\n",
                index + 1,
                describe_folder_match(association)
            ));
            for command in &association.commands {
                output.push_str(&format!(
                    "     - {}{}\n",
                    command.label,
                    describe_command_details(command)
                ));
            }
        }
    }

    output.push_str("\nGeneric Associations:\n");
    if config.association.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for (index, association) in config.association.iter().enumerate() {
            output.push_str(&format!(
                "  {}. match {}\n",
                index + 1,
                describe_match_rule(&association.match_rule)
            ));
            for command in &association.commands {
                output.push_str(&format!(
                    "     - {}{}\n",
                    command.label,
                    describe_command_details(command)
                ));
            }
        }
    }

    output.push_str("\nShortcuts:\n");
    if config.shortcut.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for (index, shortcut) in config.shortcut.iter().enumerate() {
            output.push_str(&format!(
                "  {}. {}{}\n",
                index + 1,
                shortcut.label,
                describe_command_details(shortcut)
            ));
        }
    }

    output
}

fn describe_match_rule(rule: &MatchRule) -> String {
    let mut parts = Vec::new();

    if !rule.extensions.is_empty() {
        parts.push(format!("extensions={:?}", rule.extensions));
    }
    if !rule.names.is_empty() {
        parts.push(format!("names={:?}", rule.names));
    }
    if !rule.name_patterns.is_empty() {
        parts.push(format!("name_patterns={:?}", rule.name_patterns));
    }
    if let Some(dirs) = rule.dirs {
        parts.push(format!("dirs={dirs}"));
    }
    if let Some(empty) = rule.empty {
        parts.push(format!("empty={empty}"));
    }

    if parts.is_empty() {
        "(empty)".to_string()
    } else {
        parts.join(", ")
    }
}

fn describe_folder_match(association: &FolderAssociation) -> String {
    if association.paths.is_empty() && association.names.is_empty() {
        return "any folder".to_string();
    }

    let mut parts = Vec::new();
    if !association.paths.is_empty() {
        parts.push(format!("paths={:?}", association.paths));
    }
    if !association.names.is_empty() {
        parts.push(format!("names={:?}", association.names));
    }

    parts.join(", ")
}

fn describe_optional_names(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!(", names={names:?}")
    }
}

fn describe_command_details(command: &CommandEntry) -> String {
    let mut details = Vec::new();

    if !command.description.is_empty() {
        details.push(command.description.clone());
    }
    details.push(format!("run: {}", command.run));
    if let Some(cwd) = &command.cwd {
        details.push(format!("cwd: {cwd}"));
    }

    format!(" ({})", details.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_config_parses() {
        let config: Config = toml::from_str(SAMPLE_CONFIG).expect("sample config should parse");

        assert_eq!(config.extension.len(), 7);
        assert_eq!(config.folder.len(), 1);
        assert_eq!(config.association.len(), 2);
        assert_eq!(config.shortcut.len(), 21);
    }

    #[test]
    fn menu_art_uses_default_when_no_file_is_configured() {
        let config = Config {
            menu: MenuConfig::default(),
            extension: Vec::new(),
            folder: Vec::new(),
            association: Vec::new(),
            shortcut: Vec::new(),
        };

        let art = load_menu_art(&config, Path::new("/tmp/opn/config.toml"))
            .expect("default art should load");

        assert_eq!(art, DEFAULT_MENU_ART);
    }

    #[test]
    fn menu_art_file_paths_are_relative_to_config_file() {
        let root = std::env::temp_dir().join(format!(
            "opn-art-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let art_dir = root.join("art");
        fs::create_dir_all(&art_dir).expect("test art dir should be created");
        fs::write(art_dir.join("banner.txt"), "CUSTOM\n").expect("test art should be written");

        let config = Config {
            menu: MenuConfig {
                art_file: Some("art/banner.txt".to_string()),
            },
            extension: Vec::new(),
            folder: Vec::new(),
            association: Vec::new(),
            shortcut: Vec::new(),
        };

        let art =
            load_menu_art(&config, &root.join("config.toml")).expect("relative art should load");

        assert_eq!(art, "CUSTOM\n");

        let _ = fs::remove_dir_all(root);
    }
}
