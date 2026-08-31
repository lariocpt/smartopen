use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::platform::Platform;

// Every table is `deny_unknown_fields`: a misspelt key (`extension = ` for
// `extensions = `) is an error at load time, not a rule that silently never matches.

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub menu: MenuConfig,
    #[serde(default)]
    pub extension: Vec<ExtensionAssociation>,
    #[serde(default)]
    pub folder: Vec<FolderAssociation>,
    #[serde(default)]
    pub url: Vec<UrlAssociation>,
    #[serde(default)]
    pub association: Vec<Association>,
    #[serde(default)]
    pub shortcut: Vec<CommandEntry>,
}

/// `[[url]]`: commands for URL targets, chosen by scheme and host.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UrlAssociation {
    /// `https`, `mailto`, … ; empty means any scheme.
    #[serde(default)]
    pub schemes: Vec<String>,
    /// Host globs such as `github.com` or `*.example.com`; empty means any host.
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MenuConfig {
    #[serde(default)]
    pub art_file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Association {
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionAssociation {
    pub extensions: Vec<String>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FolderAssociation {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
    /// MIME globs such as `text/*` or `image/png`, detected from the file's bytes and
    /// name in yazi's vocabulary: directories are `inode/directory`, empty files
    /// `inode/empty`, URLs `x-scheme-handler/<scheme>`.
    #[serde(default)]
    pub mime: Vec<String>,
    /// Interpreter names from a `#!` line, as globs: `python*`, `bash`, `node`.
    #[serde(default)]
    pub shebang: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
    /// Offer this command only on one platform (`unix`, `linux`, `macos`, `windows`), so a
    /// single config can serve every machine. Absent means everywhere.
    #[serde(default)]
    pub platform: Option<Platform>,
    /// A hotkey: `Alt+<key>` picks this command straight from the menu, even while a
    /// filter is being typed. One character.
    #[serde(default)]
    pub key: Option<char>,
    /// Higher sorts first in the menu; equal priorities keep config order. Default 0.
    #[serde(default)]
    pub priority: i32,
    /// When exactly one offered command is `default`, it runs without a menu
    /// (`--menu` still shows one).
    #[serde(default)]
    pub default: bool,
}

impl CommandEntry {
    /// Is this command for the OS the binary is running on?
    pub fn applies_here(&self) -> bool {
        self.platform.is_none_or(Platform::applies_here)
    }
}

pub const DEFAULT_MENU_ART: &str = include_str!("../examples/art/default.txt");

pub const SAMPLE_CONFIG_LINUX: &str = include_str!("../examples/config.toml");
pub const SAMPLE_CONFIG_MACOS: &str = include_str!("../examples/config-macos.toml");
pub const SAMPLE_CONFIG_WINDOWS: &str = include_str!("../examples/config-windows.toml");

/// The starter config for the OS this binary was built for. Each one names tools that
/// exist there, so a fresh `--init-config` followed by `--doctor` is not a wall of red.
pub const SAMPLE_CONFIG: &str = if cfg!(windows) {
    SAMPLE_CONFIG_WINDOWS
} else if cfg!(target_os = "macos") {
    SAMPLE_CONFIG_MACOS
} else {
    SAMPLE_CONFIG_LINUX
};

pub fn default_config_path() -> Result<PathBuf> {
    paths::config_path().ok_or_else(|| {
        anyhow!("could not determine a config directory: set XDG_CONFIG_HOME or HOME (APPDATA on Windows)")
    })
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

/// File names a project may carry at its root (or anywhere above the working directory).
pub const PROJECT_CONFIG_NAMES: &[&str] = &[".smartopen.toml", ".opn.toml"];

/// Walk up from each start directory looking for a project config. The search stops
/// after the directory that holds `.git`, at the home directory, or at the root, so a
/// config in `~` never leaks into every repo below it. `starts` are tried in order —
/// the working directory first, then the target's — and the first hit wins.
pub fn find_project_config(starts: &[PathBuf], home: Option<&Path>) -> Option<PathBuf> {
    for start in starts {
        let mut dir = Some(start.as_path());
        while let Some(current) = dir {
            for name in PROJECT_CONFIG_NAMES {
                let candidate = current.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            if current.join(".git").exists() || home.is_some_and(|home| home == current) {
                break;
            }
            dir = current.parent();
        }
    }
    None
}

/// The user config with a project config layered over it: the project's associations
/// and shortcuts come first, so they win the menu order and label deduplication.
/// `[menu]` stays the user's — banners are a taste, not a project matter. Relative
/// folder paths in the project file are anchored to that file, not the user config.
pub fn merge_project(mut base: Config, mut project: Config, project_path: &Path) -> Config {
    let project_dir = project_path.parent().unwrap_or_else(|| Path::new("."));
    for association in &mut project.folder {
        for path in &mut association.paths {
            if !path.starts_with('~') && !Path::new(path.as_str()).is_absolute() {
                *path = project_dir.join(path.as_str()).display().to_string();
            }
        }
    }

    project.extension.append(&mut base.extension);
    project.folder.append(&mut base.folder);
    project.url.append(&mut base.url);
    project.association.append(&mut base.association);
    project.shortcut.append(&mut base.shortcut);
    project.menu = base.menu;
    project
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

    output.push_str("\nURL Associations:\n");
    if config.url.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for (index, association) in config.url.iter().enumerate() {
            let what = if association.schemes.is_empty() && association.hosts.is_empty() {
                "any url".to_string()
            } else {
                format!(
                    "schemes={:?} hosts={:?}",
                    association.schemes, association.hosts
                )
            };
            output.push_str(&format!("  {}. {what}\n", index + 1));
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
    if !rule.mime.is_empty() {
        parts.push(format!("mime={:?}", rule.mime));
    }
    if !rule.shebang.is_empty() {
        parts.push(format!("shebang={:?}", rule.shebang));
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
    if let Some(platform) = command.platform {
        details.push(format!("platform: {platform:?}").to_lowercase());
    }
    if let Some(key) = command.key {
        details.push(format!("key: Alt+{key}"));
    }
    if command.priority != 0 {
        details.push(format!("priority: {}", command.priority));
    }
    if command.default {
        details.push("default".to_string());
    }

    format!(" ({})", details.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every platform's sample parses on every platform, and each has something in every
    /// section. Counts are deliberately not asserted: they differ per OS and drift with
    /// every edit, which is how this test used to need patching alongside the sample.
    #[test]
    fn every_sample_config_parses_with_all_sections() {
        for (name, text) in [
            ("linux", SAMPLE_CONFIG_LINUX),
            ("macos", SAMPLE_CONFIG_MACOS),
            ("windows", SAMPLE_CONFIG_WINDOWS),
        ] {
            let config: Config =
                toml::from_str(text).unwrap_or_else(|e| panic!("{name} sample: {e}"));
            assert!(!config.extension.is_empty(), "{name}: no [[extension]]");
            assert!(!config.folder.is_empty(), "{name}: no [[folder]]");
            assert!(!config.association.is_empty(), "{name}: no [[association]]");
            assert!(!config.shortcut.is_empty(), "{name}: no [[shortcut]]");
        }
    }

    #[test]
    fn a_misspelt_key_is_an_error_not_a_silent_no_op() {
        let typo = "[[extension]]\nextension = [\"rs\"]\n\n[[extension.command]]\nlabel = \"x\"\nrun = \"x\"\n";
        let error = toml::from_str::<Config>(typo).expect_err("must reject `extension =`");
        assert!(
            error.to_string().contains("unknown field `extension`"),
            "{error}"
        );

        let nested = "[[shortcut]]\nlabel = \"x\"\nrun = \"x\"\ndettach = true\n";
        let error = toml::from_str::<Config>(nested).expect_err("must reject `dettach`");
        assert!(
            error.to_string().contains("unknown field `dettach`"),
            "{error}"
        );
    }

    #[test]
    fn the_built_in_sample_is_the_one_for_this_os() {
        let want = if cfg!(windows) {
            SAMPLE_CONFIG_WINDOWS
        } else if cfg!(target_os = "macos") {
            SAMPLE_CONFIG_MACOS
        } else {
            SAMPLE_CONFIG_LINUX
        };
        assert_eq!(SAMPLE_CONFIG, want);
    }

    #[test]
    fn command_platform_gates_where_it_applies() {
        let everywhere = CommandEntry::default();
        assert!(everywhere.applies_here());

        let windows_only = CommandEntry {
            platform: Some(Platform::Windows),
            ..CommandEntry::default()
        };
        assert_eq!(windows_only.applies_here(), cfg!(windows));
    }

    #[test]
    fn menu_art_uses_default_when_no_file_is_configured() {
        let config = Config::default();

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
            ..Config::default()
        };

        let art =
            load_menu_art(&config, &root.join("config.toml")).expect("relative art should load");

        assert_eq!(art, "CUSTOM\n");

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("smartopen-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn project_config_is_found_above_cwd_but_not_past_the_git_boundary() {
        let root = temp_root("project");
        let repo = root.join("repo");
        let deep = repo.join("src").join("deep");
        fs::create_dir_all(&deep).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".smartopen.toml"), "").unwrap();
        // A config ABOVE the repo must not leak in.
        fs::write(root.join(".opn.toml"), "").unwrap();

        let found = find_project_config(std::slice::from_ref(&deep), None);
        assert_eq!(found, Some(repo.join(".smartopen.toml")));

        // Remove the repo's own file: the search stops at .git and finds nothing.
        fs::remove_file(repo.join(".smartopen.toml")).unwrap();
        assert_eq!(find_project_config(std::slice::from_ref(&deep), None), None);

        // Without a .git, the walk continues up and reaches the legacy name.
        fs::remove_dir_all(repo.join(".git")).unwrap();
        assert_eq!(
            find_project_config(&[deep], None),
            Some(root.join(".opn.toml"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_search_stops_at_home() {
        let root = temp_root("home");
        let home = root.join("home");
        let work = home.join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join(".smartopen.toml"), "").unwrap();

        assert_eq!(
            find_project_config(std::slice::from_ref(&work), Some(&home)),
            None
        );
        fs::write(home.join(".smartopen.toml"), "").unwrap();
        assert_eq!(
            find_project_config(&[work], Some(&home)),
            Some(home.join(".smartopen.toml"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_entries_come_first_and_relative_folder_paths_anchor_to_the_project() {
        let base: Config = toml::from_str(
            "[[shortcut]]\nlabel = \"User\"\nrun = \"u\"\n[[folder]]\npaths = [\"a\"]\n[[folder.command]]\nlabel = \"UserFolder\"\nrun = \"x\"\n",
        )
        .unwrap();
        let project: Config = toml::from_str(
            "[[shortcut]]\nlabel = \"Project\"\nrun = \"p\"\n[[folder]]\npaths = [\"sub\", \"/abs\", \"~/x\"]\n[[folder.command]]\nlabel = \"ProjFolder\"\nrun = \"y\"\n[menu]\nart_file = \"ignored.txt\"\n",
        )
        .unwrap();

        let merged = merge_project(base, project, Path::new("/repo/.smartopen.toml"));

        let labels: Vec<_> = merged.shortcut.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Project", "User"]);
        assert_eq!(merged.folder[0].paths, ["/repo/sub", "/abs", "~/x"]);
        assert_eq!(merged.folder[1].paths, ["a"], "user paths are untouched");
        assert_eq!(merged.menu.art_file, None, "[menu] is the user's");
    }

    #[test]
    fn url_sections_and_new_match_keys_parse() {
        let config: Config = toml::from_str(
            "[[url]]\nschemes = [\"https\"]\nhosts = [\"*.github.com\"]\n[[url.command]]\nlabel = \"gh\"\nrun = \"gh browse {url}\"\n\n[[association]]\n[association.match]\nmime = [\"text/*\"]\nshebang = [\"python*\"]\n[[association.command]]\nlabel = \"py\"\nrun = \"python {path}\"\npriority = 5\ndefault = true\n",
        )
        .unwrap();
        assert_eq!(config.url[0].hosts, ["*.github.com"]);
        assert_eq!(config.association[0].match_rule.mime, ["text/*"]);
        assert_eq!(config.association[0].commands[0].priority, 5);
        assert!(config.association[0].commands[0].default);
    }
}
