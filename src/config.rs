use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use directories::BaseDirs;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub association: Vec<Association>,
    #[serde(default)]
    pub shortcut: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Association {
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
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
    pub dirs: Option<bool>,
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
}

pub const SAMPLE_CONFIG: &str = include_str!("../examples/config.toml");

pub fn default_config_path() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().ok_or_else(|| anyhow!("could not determine home directory"))?;
    Ok(base_dirs.config_dir().join("smartopen").join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        bail!(
            "no config found at {}\ncreate one with: smartopen --init-config",
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

pub fn describe_config(config: &Config, path: &Path) -> String {
    let mut output = String::new();

    output.push_str(&format!("Config: {}\n", path.display()));
    output.push_str("\nAssociations:\n");
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
    if let Some(dirs) = rule.dirs {
        parts.push(format!("dirs={dirs}"));
    }

    if parts.is_empty() {
        "(empty)".to_string()
    } else {
        parts.join(", ")
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

        assert_eq!(config.association.len(), 2);
        assert_eq!(config.shortcut.len(), 2);
    }
}
