//! The wizard's catalogue: which terminal tools open which kinds of file, and how each
//! one is installed. Data, not code — `catalog/tools.toml` and `catalog/categories.toml`
//! are embedded at build time, so adding a tool is a TOML edit.
//!
//! EVERY INSTALL SOURCE IS A VERIFIED CLAIM. A `cargo = "…"` key means the crate of that
//! name IS this tool, checked on crates.io; a `pacman = "…"` key means the package is in
//! the Arch repositories, checked with `pacman -Si`. A missing key means "not there", not
//! "didn't look". The lesson comes from the estate's installer: `qo`, `surge` and
//! `redthread` have unrelated crates squatting their names, and this catalogue's own
//! check found five more — `micro`, `glow`, `chafa`, `mpv` and `helix` on crates.io are
//! a macro playground, GL bindings, a wrapper, bindings and a Ruby embedder. The test
//! below refuses a `cargo` key for any of them.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::platform::{Host, Platform};
use crate::runner::find_executable;

const TOOLS_TOML: &str = include_str!("../catalog/tools.toml");
const CATEGORIES_TOML: &str = include_str!("../catalog/categories.toml");

/// Names whose crates.io crate is NOT the tool. A `cargo` source for any of these is a
/// catalogue bug, and the test says so.
pub const NEVER_CARGO: &[&str] = &[
    "qo",
    "surge",
    "redthread",
    "micro",
    "glow",
    "chafa",
    "mpv",
    "helix",
    "hl",
    "lazygit",
    "fx",
    "lnav",
    "neovim",
    "lynx",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// The executable to look for, when it differs from `name` (tailspin → `tspin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    /// Where it runs; empty means everywhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<Platform>,
    #[serde(default)]
    pub install: Install,
}

/// One key per package manager, each naming the package AS THAT MANAGER KNOWS IT.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Install {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacman: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dnf: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brew: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipx: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winget: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoop: Option<String>,
    /// `owner/repo` on GitHub, for a release-asset fallback via `eget`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// Shown when nothing above applies: how a person installs it by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    /// Extensions this category covers (a file category).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// `folder` for directories, `git` for directories holding `.git`, `url` for links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Name globs for a generic association (`.env`, `Dockerfile*`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_patterns: Vec<String>,
    /// The candidate commands, recommended first.
    #[serde(default, rename = "choice")]
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// A catalogue tool name, or `$EDITOR` / `$TERMINAL` / `open` for things that are not
    /// installed but assumed.
    pub tool: String,
    pub label: String,
    pub run: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub detach: bool,
    /// Working directory, with placeholders (`{path}`, `{dir}`); the way to run a tool
    /// "in this folder". Not `sh -c 'cd … && tool'`: `config doctor` would then check
    /// `sh` and report the missing tool as fine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolsFile {
    #[serde(rename = "tool")]
    tools: Vec<Tool>,
}

#[derive(Debug, Clone, Deserialize)]
struct CategoriesFile {
    #[serde(rename = "category")]
    categories: Vec<Category>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub tools: Vec<Tool>,
    pub categories: Vec<Category>,
}

/// Pseudo-tools a choice may name that are not installed programs.
pub const ASSUMED: &[&str] = &["$EDITOR", "$TERMINAL", "open"];

impl Catalog {
    pub fn builtin() -> Result<Catalog> {
        let tools: ToolsFile = toml::from_str(TOOLS_TOML).context("catalog/tools.toml")?;
        let categories: CategoriesFile =
            toml::from_str(CATEGORIES_TOML).context("catalog/categories.toml")?;
        Ok(Catalog {
            tools: tools.tools,
            categories: categories.categories,
        })
    }

    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// Every choice must name a known tool or an assumed one, and every category must
    /// have at least one choice — the checks the test runs and the wizard relies on.
    pub fn validate(&self) -> Result<()> {
        for category in &self.categories {
            anyhow::ensure!(
                !category.choices.is_empty(),
                "category {} has no choices",
                category.id
            );
            for choice in &category.choices {
                anyhow::ensure!(
                    ASSUMED.contains(&choice.tool.as_str()) || self.tool(&choice.tool).is_some(),
                    "category {} names unknown tool {}",
                    category.id,
                    choice.tool
                );
            }
        }
        for tool in &self.tools {
            if let Some(krate) = &tool.install.cargo {
                anyhow::ensure!(
                    !NEVER_CARGO.contains(&krate.as_str()),
                    "tool {} claims cargo = {krate}, but that crate is not this tool",
                    tool.name
                );
            }
        }
        Ok(())
    }
}

impl Tool {
    pub fn binary(&self) -> &str {
        self.binary.as_deref().unwrap_or(&self.name)
    }

    pub fn runs_on(&self, host: Host) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|p| p.applies_on(host))
    }

    pub fn installed(&self) -> bool {
        find_executable(self.binary()).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_catalogue_parses_and_validates() {
        let catalog = Catalog::builtin().expect("catalogue must parse");
        catalog.validate().expect("catalogue must validate");
        assert!(catalog.tools.len() >= 20, "{} tools", catalog.tools.len());
        assert!(
            catalog.categories.len() >= 12,
            "{} categories",
            catalog.categories.len()
        );
    }

    #[test]
    fn navigators_come_first_and_every_category_has_a_recommendation() {
        let catalog = Catalog::builtin().unwrap();
        assert_eq!(catalog.categories[0].id, "directories");
        for category in &catalog.categories {
            assert!(!category.choices[0].label.is_empty(), "{}", category.id);
        }
    }

    #[test]
    fn no_tool_claims_a_squatted_crate() {
        let catalog = Catalog::builtin().unwrap();
        for tool in &catalog.tools {
            if let Some(krate) = &tool.install.cargo {
                assert!(!NEVER_CARGO.contains(&krate.as_str()), "{}", tool.name);
            }
        }
        // And the known-good ones are recorded, so the check is not vacuous.
        assert_eq!(
            catalog.tool("xan").unwrap().install.cargo.as_deref(),
            Some("xan")
        );
        assert_eq!(catalog.tool("micro").unwrap().install.cargo, None);
        assert_eq!(catalog.tool("glow").unwrap().install.cargo, None);
    }

    #[test]
    fn every_choice_placeholder_is_a_known_one() {
        let catalog = Catalog::builtin().unwrap();
        let known = crate::runner::PLACEHOLDERS;
        for category in &catalog.categories {
            for choice in &category.choices {
                for text in std::iter::once(choice.run.as_str()).chain(choice.cwd.as_deref()) {
                    // `${EDITOR:-micro}` is a shell expansion, not a placeholder: skip a
                    // brace that follows `$`.
                    let bytes = text.as_bytes();
                    let mut from = 0;
                    while let Some(offset) = text[from..].find('{') {
                        let start = from + offset;
                        if start > 0 && bytes[start - 1] == b'$' {
                            from = start + 1;
                            continue;
                        }
                        let Some(len) = text[start..].find('}') else {
                            break;
                        };
                        let token = &text[start..=start + len];
                        assert!(
                            known.contains(&token),
                            "{}: unknown placeholder {token} in {text}",
                            category.id
                        );
                        from = start + len + 1;
                    }
                }
            }
        }
    }

    #[test]
    fn no_choice_hides_its_tool_behind_sh_c() {
        // `sh -c 'cd "$1" && gitui' _ {path}` made doctor check `sh`; the review saw
        // `Open lazydocker` reported ok on a machine without lazydocker. `cwd` is the
        // spelling for "in this folder".
        let catalog = Catalog::builtin().unwrap();
        for category in &catalog.categories {
            for choice in &category.choices {
                assert!(
                    !choice.run.starts_with("sh -c"),
                    "{}: {} runs through sh; use cwd",
                    category.id,
                    choice.label
                );
            }
        }
        let folders = catalog
            .categories
            .iter()
            .find(|c| c.id == "directories")
            .unwrap();
        let gitui = folders.choices.iter().find(|c| c.tool == "gitui").unwrap();
        assert_eq!(gitui.run, "gitui");
        assert_eq!(gitui.cwd.as_deref(), Some("{path}"));
    }
}
