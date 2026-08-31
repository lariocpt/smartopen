//! `smartopen wizard`: a guided first run.
//!
//! Navigators first — yazi and broot are how a file manager's Enter reaches this menu —
//! then one checklist per file category from the catalogue, with what is installed
//! ticked and what would be installed marked. Then a review of two things: the TOML
//! about to be written and the exact package-manager commands about to run. Nothing is
//! written or run before that review, installs default to no, and `--dry-run` stops at
//! the review. `--yes` takes every recommendation, for scripted setups; it still shows
//! the review.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::catalog::{ASSUMED, Catalog, Category, Choice, Tool};
use crate::config::{
    Association, CommandEntry, Config, ExtensionAssociation, FolderAssociation, MatchRule,
};
use crate::installer::{self, Manager, Step};
use crate::menu;
use crate::navigators::{self, Action, Navigator};
use crate::platform::Host;
use crate::tomlio;

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Show the review and exit: write nothing, run nothing.
    pub dry_run: bool,
    /// Take the recommendation at every step and confirm the review.
    pub yes: bool,
    /// Write the config but never run an installer.
    pub no_install: bool,
}

/// What the wizard decided, before anything touches the disk.
#[derive(Debug, Default)]
pub struct Plan {
    pub config: Config,
    pub installs: Vec<Step>,
    /// Tools wanted but not installable by any manager here, with how to get them.
    pub manual: Vec<(String, String)>,
    pub apply_yazi: bool,
    pub apply_broot: bool,
}

pub fn run(config_path: &Path, options: Options) -> Result<i32> {
    let catalog = Catalog::builtin()?;
    catalog.validate()?;
    let host = Host::current();
    let managers = Manager::detect();

    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !interactive && !options.yes && !options.dry_run {
        bail!("the wizard needs a terminal; pass --yes to take every recommendation");
    }
    let ask = interactive && !options.yes;

    // Step 0: navigators.
    // broot's Enter verb needs `sh`, which broot on Windows does not run through; the
    // wizard offers only what `broot apply` would accept there.
    let navigator_tools: Vec<&Tool> = ["yazi", "broot"]
        .iter()
        .filter(|name| host != Host::Windows || **name != "broot")
        .filter_map(|name| catalog.tool(name))
        .collect();
    let navigator_choices: Vec<menu::Choice> = navigator_tools
        .iter()
        .map(|tool| {
            let installed = tool.installed();
            let step = installer::plan(tool, &managers, host);
            let (marker, detail) = describe_tool(tool, installed, step.as_ref());
            menu::Choice {
                label: format!(
                    "{} — open files through this menu{}",
                    tool.name,
                    if installed { "" } else { " (install it first)" }
                ),
                detail,
                marker,
                checked: installed || step.is_some(),
            }
        })
        .collect();
    let navigator_picks = if ask {
        match menu::select_many(
            "Navigators",
            "Enter on a file in yazi or broot opens this menu. Tick the ones to set up; a missing one is installed first.",
            &navigator_choices,
        )? {
            Some(picks) => picks,
            None => return Ok(cancelled()),
        }
    } else {
        navigator_choices.iter().map(|c| c.checked).collect()
    };

    // Steps 1..n: one checklist per category.
    let mut selections: Vec<(&Category, Vec<&Choice>)> = Vec::new();
    for category in &catalog.categories {
        let candidates: Vec<&Choice> = category
            .choices
            .iter()
            .filter(|choice| choice.platform.is_none_or(|p| p.applies_on(host)))
            .filter(|choice| {
                ASSUMED.contains(&choice.tool.as_str())
                    || catalog.tool(&choice.tool).is_some_and(|t| t.runs_on(host))
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let rows: Vec<menu::Choice> = candidates
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                choice_row(
                    &catalog,
                    choice,
                    index == 0,
                    &managers,
                    host,
                    &Tool::installed,
                )
            })
            .collect();
        let picks = if ask {
            match menu::select_many(&category.title, category_intro(category), &rows)? {
                Some(picks) => picks,
                None => return Ok(cancelled()),
            }
        } else {
            rows.iter().map(|r| r.checked).collect()
        };
        let chosen: Vec<&Choice> = candidates
            .iter()
            .zip(&picks)
            .filter(|(_, picked)| **picked)
            .map(|(choice, _)| *choice)
            .collect();
        if !chosen.is_empty() {
            selections.push((category, chosen));
        }
    }

    let mut plan = build_plan(&catalog, &selections, &managers, host, &Tool::installed);
    plan.apply_yazi = navigator_picks.first().copied().unwrap_or(false);
    plan.apply_broot = navigator_picks.get(1).copied().unwrap_or(false);
    for (tool, wanted) in navigator_tools
        .iter()
        .zip([plan.apply_yazi, plan.apply_broot])
    {
        if wanted && !tool.installed() {
            match installer::plan(tool, &managers, host) {
                Some(step) if !plan.installs.iter().any(|s| s.tool == step.tool) => {
                    plan.installs.insert(0, step);
                }
                Some(_) => {}
                None => plan.manual.push((tool.name.clone(), manual_hint(tool))),
            }
        }
    }
    if options.no_install {
        plan.installs.clear();
    }

    // The review.
    let toml_text = toml::to_string_pretty(&plan.config).context("rendering the config")?;
    let mut out = io::stderr();
    writeln!(
        out,
        "\n──── config to write: {} ────\n{toml_text}",
        config_path.display()
    )?;
    if !plan.installs.is_empty() {
        writeln!(out, "──── installs (nothing has run yet) ────")?;
        for step in &plan.installs {
            writeln!(out, "  {}    # {}", step.command, step.tool)?;
        }
    }
    if !plan.manual.is_empty() {
        writeln!(out, "──── not installable from here ────")?;
        for (tool, hint) in &plan.manual {
            writeln!(out, "  {tool}: {hint}")?;
        }
    }
    let navigators_line = [("yazi", plan.apply_yazi), ("broot", plan.apply_broot)]
        .iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    if !navigators_line.is_empty() {
        writeln!(out, "──── navigators to configure: {navigators_line} ────")?;
    }
    if options.dry_run {
        writeln!(out, "\n--dry-run: nothing written, nothing run.")?;
        return Ok(0);
    }

    // Write the config.
    if config_path.exists() {
        let replace = if ask {
            confirm(
                &format!(
                    "{} exists. Back it up and replace its associations (shortcuts and [menu] are kept)?",
                    config_path.display()
                ),
                false,
            )?
        } else {
            true
        };
        if !replace {
            writeln!(
                out,
                "Config left untouched; the TOML above is yours to merge."
            )?;
            return Ok(0);
        }
        let existing: Config = toml::from_str(&fs::read_to_string(config_path)?)
            .with_context(|| format!("parsing {}", config_path.display()))?;
        plan.config.shortcut = existing.shortcut;
        plan.config.menu = existing.menu;
        let backup = tomlio::backup(config_path)?;
        writeln!(
            out,
            "backed up {} -> {}",
            config_path.display(),
            backup.display()
        )?;
    } else if ask && !confirm(&format!("Write {}?", config_path.display()), true)? {
        return Ok(cancelled());
    }
    let final_text = format!(
        "# Written by `smartopen wizard`. Edit freely; run the wizard again to start over.\n\n{}",
        toml::to_string_pretty(&plan.config)?
    );
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    tomlio::atomic_write(config_path, &final_text)?;
    writeln!(out, "wrote {}", config_path.display())?;

    // Installs: shown above, default no.
    if !plan.installs.is_empty() {
        let go = if ask {
            confirm(
                &format!("Run the {} install command(s) above?", plan.installs.len()),
                false,
            )?
        } else {
            true
        };
        if go {
            if let Err(error) = installer::run(&plan.installs) {
                writeln!(out, "install stopped: {error:#}")?;
                writeln!(
                    out,
                    "the config is written; `smartopen config doctor` shows what is still missing"
                )?;
            }
        } else {
            writeln!(
                out,
                "Skipped installs; the commands above can be run by hand."
            )?;
        }
    }

    // Navigators last, so their binaries exist by now if they were just installed.
    for (navigator, wanted) in [
        (Navigator::Yazi, plan.apply_yazi),
        (Navigator::Broot, plan.apply_broot),
    ] {
        if wanted {
            navigators::run(
                navigator,
                Action::Apply {
                    force: true,
                    no_backup: false,
                },
                &navigators::Options::default(),
            )?;
        }
    }

    writeln!(
        out,
        "\nDone. `smartopen config doctor` checks the result; `smartopen` opens the launcher."
    )?;
    Ok(0)
}

/// The config and install steps for a set of selections. Pure, so it is testable.
pub fn build_plan(
    catalog: &Catalog,
    selections: &[(&Category, Vec<&Choice>)],
    managers: &[Manager],
    host: Host,
    installed: &dyn Fn(&Tool) -> bool,
) -> Plan {
    let mut plan = Plan::default();
    let mut wanted_tools: BTreeSet<String> = BTreeSet::new();

    for (category, choices) in selections {
        let commands: Vec<CommandEntry> = choices
            .iter()
            .map(|choice| {
                if !ASSUMED.contains(&choice.tool.as_str()) {
                    wanted_tools.insert(choice.tool.clone());
                }
                command_for(choice, host)
            })
            .collect();

        match (category.kind.as_deref(), category.extensions.is_empty()) {
            (Some("folder"), _) => plan.config.folder.push(FolderAssociation {
                names: Vec::new(),
                paths: Vec::new(),
                commands,
            }),
            (_, false) => plan.config.extension.push(ExtensionAssociation {
                extensions: category.extensions.clone(),
                names: Vec::new(),
                commands,
            }),
            (_, true) if !category.name_patterns.is_empty() => {
                plan.config.association.push(Association {
                    match_rule: MatchRule {
                        name_patterns: category.name_patterns.clone(),
                        dirs: Some(false),
                        ..MatchRule::default()
                    },
                    commands,
                })
            }
            _ => {}
        }
    }

    for name in wanted_tools {
        let Some(tool) = catalog.tool(&name) else {
            continue;
        };
        if installed(tool) {
            continue;
        }
        match installer::plan(tool, managers, host) {
            Some(step) => plan.installs.push(step),
            None => plan.manual.push((tool.name.clone(), manual_hint(tool))),
        }
    }
    plan
}

fn command_for(choice: &Choice, host: Host) -> CommandEntry {
    // `$TERMINAL` choices open a new terminal window in the folder (`cwd = "{path}"` is
    // rendered by the runner) running an interactive shell: `${SHELL:-sh}` is sh syntax,
    // so Windows gets `cmd`.
    let terminal = choice.tool == "$TERMINAL";
    CommandEntry {
        label: choice.label.clone(),
        description: choice.description.clone(),
        icon: choice.icon.clone(),
        run: match (terminal, host) {
            (true, Host::Windows) => "cmd".to_string(),
            (true, _) => "${SHELL:-sh}".to_string(),
            (false, _) => choice.run.clone(),
        },
        cwd: terminal.then(|| "{path}".to_string()),
        detach: choice.detach,
        platform: choice.platform,
        terminal,
        ..CommandEntry::default()
    }
}

fn choice_row(
    catalog: &Catalog,
    choice: &Choice,
    recommended: bool,
    managers: &[Manager],
    host: Host,
    is_installed: &dyn Fn(&Tool) -> bool,
) -> menu::Choice {
    let (marker, detail, available) = if ASSUMED.contains(&choice.tool.as_str()) {
        (
            "✓".to_string(),
            format!(
                "{}\n\nUses {} — nothing to install.",
                choice.run, choice.tool
            ),
            true,
        )
    } else {
        let tool = catalog.tool(&choice.tool).expect("validated");
        let installed = is_installed(tool);
        let step = installer::plan(tool, managers, host);
        let (marker, detail) = describe_tool(tool, installed, step.as_ref());
        (
            marker,
            format!("{}\n\n{detail}", choice.run),
            installed || step.is_some(),
        )
    };
    let installed_now = marker == "✓";
    menu::Choice {
        label: format!(
            "{}{}",
            choice.label,
            if recommended { "  (recommended)" } else { "" }
        ),
        detail,
        marker,
        // Ticked by default: anything already installed, plus the recommendation when it
        // can be had. Never a tool with no way to install it.
        checked: installed_now || (recommended && available),
    }
}

fn describe_tool(tool: &Tool, installed: bool, step: Option<&Step>) -> (String, String) {
    let homepage = tool.homepage.as_deref().unwrap_or("");
    if installed {
        return (
            "✓".to_string(),
            format!("{}\n\n{}\ninstalled", tool.summary, homepage),
        );
    }
    match step {
        Some(step) => (
            "↓".to_string(),
            format!(
                "{}\n\n{}\nwould install with:\n  {}",
                tool.summary, homepage, step.command
            ),
        ),
        None => (
            "✗".to_string(),
            format!(
                "{}\n\n{}\nnot installable from here: {}",
                tool.summary,
                homepage,
                manual_hint(tool)
            ),
        ),
    }
}

fn manual_hint(tool: &Tool) -> String {
    if let Some(note) = &tool.install.note {
        return note.clone();
    }
    if let Some(repo) = &tool.install.github {
        return format!("releases at https://github.com/{repo}/releases");
    }
    tool.homepage
        .clone()
        .unwrap_or_else(|| "see the tool's own instructions".to_string())
}

fn category_intro(category: &Category) -> &str {
    if category.summary.is_empty() {
        "Tick the commands to offer for these files. ✓ installed · ↓ will be installed · ✗ not installable here."
    } else {
        category.summary.as_str()
    }
}

fn confirm(question: &str, default_yes: bool) -> Result<bool> {
    let mut stderr = io::stderr();
    write!(
        stderr,
        "{question} [{}] ",
        if default_yes { "Y/n" } else { "y/N" }
    )?;
    stderr.flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(match answer.trim() {
        "" => default_yes,
        a => matches!(a, "y" | "Y" | "yes" | "YES"),
    })
}

fn cancelled() -> i32 {
    eprintln!("cancelled; nothing written.");
    130
}

/// `smartopen tools list`: the catalogue, with what is installed and how to get the rest.
pub fn list_tools(json: bool) -> Result<i32> {
    let catalog = Catalog::builtin()?;
    let host = Host::current();
    let managers = Manager::detect();
    let rows: Vec<serde_json::Value> = catalog
        .tools
        .iter()
        .filter(|tool| tool.runs_on(host))
        .map(|tool| {
            let installed = tool.installed();
            let how = if installed {
                None
            } else {
                Some(
                    installer::plan(tool, &managers, host)
                        .map(|s| s.command)
                        .unwrap_or_else(|| manual_hint(tool)),
                )
            };
            serde_json::json!({
                "name": tool.name,
                "summary": tool.summary,
                "installed": installed,
                "install": how,
                "homepage": tool.homepage,
            })
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(0);
    }
    for row in &rows {
        let mark = if row["installed"].as_bool().unwrap_or(false) {
            "✓"
        } else {
            " "
        };
        println!(
            "{mark} {:<11} {}",
            row["name"].as_str().unwrap_or(""),
            row["summary"].as_str().unwrap_or("")
        );
        if let Some(how) = row["install"].as_str() {
            println!("              install: {how}");
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing_installed(_: &Tool) -> bool {
        false
    }

    #[test]
    fn a_plan_builds_the_right_sections_and_installs_what_is_missing() {
        let catalog = Catalog::builtin().unwrap();
        let folders = catalog
            .categories
            .iter()
            .find(|c| c.id == "directories")
            .unwrap();
        let csv = catalog.categories.iter().find(|c| c.id == "csv").unwrap();
        let env = catalog.categories.iter().find(|c| c.id == "env").unwrap();

        let selections = vec![
            (folders, vec![&folders.choices[0], &folders.choices[4]]), // yazi, $TERMINAL
            (csv, vec![&csv.choices[0]]),                              // xan
            (env, vec![&env.choices[0]]),                              // lazyenv
        ];
        let plan = build_plan(
            &catalog,
            &selections,
            &[Manager::Cargo],
            Host::Linux,
            &nothing_installed,
        );

        assert_eq!(plan.config.folder.len(), 1);
        assert_eq!(plan.config.folder[0].commands[0].run, "yazi {path}");
        let terminal = &plan.config.folder[0].commands[1];
        assert!(
            terminal.terminal,
            "$TERMINAL choices become terminal = true"
        );
        assert_eq!(terminal.cwd.as_deref(), Some("{path}"));
        assert_eq!(terminal.run, "${SHELL:-sh}");
        assert_eq!(
            command_for(&folders.choices[4], Host::Windows).run,
            "cmd",
            "sh syntax has no place on Windows"
        );
        assert_eq!(plan.config.extension[0].extensions, ["csv", "tsv"]);
        assert_eq!(
            plan.config.association[0].match_rule.name_patterns,
            [".env", ".env.*", "*.env"]
        );

        // With only cargo available: yazi and xan have crates, lazyenv does not.
        let commands: Vec<&str> = plan.installs.iter().map(|s| s.command.as_str()).collect();
        assert_eq!(
            commands,
            [
                "cargo install --locked xan",
                "cargo install --locked yazi-fm yazi-cli"
            ]
        );
        assert_eq!(plan.manual.len(), 1);
        assert_eq!(plan.manual[0].0, "lazyenv");
        assert!(plan.manual[0].1.contains("github.com/lazynop/lazyenv"));

        let text = toml::to_string_pretty(&plan.config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.extension.len(), 1, "the written config parses back");
    }

    #[test]
    fn nothing_to_install_when_everything_is_already_there() {
        let catalog = Catalog::builtin().unwrap();
        let csv = catalog.categories.iter().find(|c| c.id == "csv").unwrap();
        let plan = build_plan(
            &catalog,
            &[(csv, vec![&csv.choices[0]])],
            &[],
            Host::Linux,
            &|_| true,
        );
        assert!(plan.installs.is_empty());
        assert!(plan.manual.is_empty());
    }

    #[test]
    fn rows_tick_installed_tools_and_the_recommendation_but_never_the_uninstallable() {
        let catalog = Catalog::builtin().unwrap();
        let env = catalog.categories.iter().find(|c| c.id == "env").unwrap();

        // No managers: lazyenv cannot be installed, so its row is not ticked even though
        // it is the recommendation, and the marker says why.
        let row = choice_row(
            &catalog,
            &env.choices[0],
            true,
            &[],
            Host::Linux,
            &nothing_installed,
        );
        assert!(!row.checked);
        assert_eq!(row.marker, "✗");
        assert!(row.label.contains("(recommended)"));
        assert!(row.detail.contains("not installable from here"));

        // With eget it becomes installable, and the recommendation is ticked.
        let row = choice_row(
            &catalog,
            &env.choices[0],
            true,
            &[Manager::Eget],
            Host::Linux,
            &nothing_installed,
        );
        assert!(row.checked);
        assert_eq!(row.marker, "↓");

        // Installed tools are ticked whatever their position.
        let row = choice_row(&catalog, &env.choices[0], false, &[], Host::Linux, &|_| {
            true
        });
        assert!(row.checked);
        assert_eq!(row.marker, "✓");

        let editor = choice_row(
            &catalog,
            &env.choices[1],
            false,
            &[],
            Host::Linux,
            &nothing_installed,
        );
        assert!(editor.checked, "$EDITOR is assumed present");
        assert_eq!(editor.marker, "✓");
    }
}
