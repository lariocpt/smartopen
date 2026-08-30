mod config;
mod doctor;
mod fuzzy;
mod history;
mod matcher;
mod menu;
mod paths;
mod platform;
mod render;
mod runner;
mod shell;
mod tomlio;

// The yazi/broot surface. Today only `--setup-yazi` reaches it, so the diff/check/print
// paths are dead until they become `smartopen yazi …` subcommands; the allow is scoped
// here, not crate-wide, so nothing else can hide behind it.
#[allow(dead_code)]
mod diff;
#[allow(dead_code)]
mod engine;
#[allow(dead_code)]
mod spec;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use crate::config::{
    CommandEntry, SAMPLE_CONFIG, default_config_path, describe_config, init_config, load_config,
    load_menu_art,
};
use crate::doctor::print_doctor;
use crate::history::History;
use crate::matcher::{Target, matching_commands, shortcuts_here};
use crate::menu::select_command;
use crate::runner::{plan_command, run_command};
use crate::shell::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "smartopen",
    version,
    about = "Open files and shortcuts from configurable command menus"
)]
struct Cli {
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    #[arg(long, value_name = "PATH", help = "Use a specific config file")]
    config_path: Option<PathBuf>,

    #[arg(long, help = "Print the config file path")]
    config: bool,

    #[arg(long, help = "Open the config in $EDITOR, creating it first if needed")]
    edit_config: bool,

    #[arg(long, help = "List configured associations and shortcuts")]
    list: bool,

    #[arg(long, help = "Check config, menu art, and command availability")]
    doctor: bool,

    #[arg(long, help = "Print a sample config")]
    sample_config: bool,

    #[arg(long, help = "Create a starter config if one does not exist")]
    init_config: bool,

    #[arg(
        long,
        value_name = "LABEL",
        help = "Run a command by label without showing a menu"
    )]
    command: Option<String>,

    #[arg(long, help = "Print the selected command instead of running it")]
    dry_run: bool,

    // FalseyValueParser so the env var works the way people set env vars: `=1`, `=yes`,
    // `=true` all mean on; only empty, `0`, `false`, `no`, `off` mean off.
    #[arg(
        long,
        env = "SMARTOPEN_NO_HISTORY",
        value_parser = clap::builder::FalseyValueParser::new(),
        help = "Neither read nor record which commands were picked"
    )]
    no_history: bool,

    #[arg(long, help = "Configure yazi to use smartopen for file associations")]
    setup_yazi: bool,
}

/// The process exit code for `main`: the launched command's own code on success, 1 after
/// printing an error. Kept out of `run` so the binaries stay one line each.
pub fn main_exit_code() -> i32 {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            1
        }
    }
}

pub fn run() -> Result<i32> {
    let cli = Cli::parse();
    let config_path = selected_config_path(cli.config_path.as_deref())?;

    if cli.config {
        println!("{}", config_path.display());
        return Ok(0);
    }

    if cli.sample_config {
        print!("{SAMPLE_CONFIG}");
        return Ok(0);
    }

    if cli.init_config {
        init_config(&config_path)?;
        println!("created {}", config_path.display());
        return Ok(0);
    }

    if cli.edit_config {
        return edit_config(&config_path);
    }

    if cli.setup_yazi {
        let effective = engine::effective(
            &spec::Spec::builtin(),
            engine::Engine::Smartopen,
            "smartopen",
        );
        let config_path =
            paths::yazi_config_path().context("could not determine yazi's config directory")?;
        match tomlio::apply(&config_path, &effective, false, true)? {
            tomlio::Outcome::Created => println!("created {}", config_path.display()),
            tomlio::Outcome::Updated => println!("updated {}", config_path.display()),
            tomlio::Outcome::InSync => println!("already in sync: {}", config_path.display()),
        }
        return Ok(0);
    }

    let config = load_config(&config_path)?;

    if cli.list {
        print!("{}", describe_config(&config, &config_path));
        return Ok(0);
    }

    if cli.doctor {
        print_doctor(&config, &config_path)?;
        return Ok(0);
    }

    let mut history = if cli.no_history {
        History::disabled()
    } else {
        paths::state_path()
            .map(History::load)
            .unwrap_or_else(History::disabled)
    };

    match cli.path {
        Some(path) => {
            let target = Target::from_path(&path)?;
            let commands = matching_commands(&config, &config_path, &target)?;

            if commands.is_empty() {
                bail!("no matching commands for {}", target.path.display());
            }

            if cli.command.is_none() && commands.len() == 1 {
                return execute_or_print(&commands[0], Some(&target), cli.dry_run, &mut history);
            }

            let menu_art = menu_art_for_selection(&config, &config_path, &cli.command)?;
            match resolve_command(
                "Choose a command",
                &commands,
                &cli.command,
                &menu_art,
                Some(&target),
                &history,
            )? {
                Some(command) => {
                    execute_or_print(&command, Some(&target), cli.dry_run, &mut history)
                }
                None => Ok(0),
            }
        }
        None => {
            let shortcuts = shortcuts_here(&config);
            if shortcuts.is_empty() {
                bail!(
                    "no shortcuts configured for this platform in {}",
                    config_path.display()
                );
            }

            let menu_art = menu_art_for_selection(&config, &config_path, &cli.command)?;
            match resolve_command(
                "Choose a shortcut",
                &shortcuts,
                &cli.command,
                &menu_art,
                None,
                &history,
            )? {
                Some(command) => execute_or_print(&command, None, cli.dry_run, &mut history),
                None => Ok(0),
            }
        }
    }
}

fn selected_config_path(path: Option<&Path>) -> Result<PathBuf> {
    match path {
        Some(path) => expand_path(path),
        None => default_config_path(),
    }
}

fn expand_path(path: &Path) -> Result<PathBuf> {
    let path = path
        .to_str()
        .ok_or_else(|| anyhow!("path contains invalid UTF-8: {}", path.display()))?;
    let expanded = shellexpand::full(path)
        .with_context(|| format!("failed to expand path '{path}'"))?
        .into_owned();

    Ok(PathBuf::from(expanded))
}

fn edit_config(path: &Path) -> Result<i32> {
    if !path.exists() {
        init_config(path)?;
        println!("created {}", path.display());
    }

    let shell = Shell::current();
    let quoted = shell.quote(&path.display().to_string())?;
    // `${EDITOR:-nano}` is sh; cmd has no default-expansion syntax, so resolve the
    // editor here and fall back to the one every Windows install has.
    let run = match shell {
        Shell::Posix => format!("${{EDITOR:-nano}} {quoted}"),
        Shell::Cmd => {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "notepad".to_string());
            format!("{editor} {quoted}")
        }
    };
    let command = CommandEntry {
        label: "Edit config".to_string(),
        run,
        ..CommandEntry::default()
    };

    run_command(&command, None)
}

fn menu_art_for_selection(
    config: &crate::config::Config,
    config_path: &Path,
    requested_label: &Option<String>,
) -> Result<String> {
    if requested_label.is_some() {
        return Ok(String::new());
    }

    load_menu_art(config, config_path)
}

fn resolve_command(
    prompt: &str,
    commands: &[CommandEntry],
    requested_label: &Option<String>,
    menu_art: &str,
    target: Option<&Target>,
    history: &History,
) -> Result<Option<CommandEntry>> {
    let Some(label) = requested_label else {
        return select_command(prompt, commands, menu_art, target, history);
    };

    let label_lower = label.to_lowercase();
    let command = commands
        .iter()
        .find(|command| command.label.to_lowercase() == label_lower)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "no command labeled '{}'\navailable labels: {}",
                label,
                available_labels(commands)
            )
        })?;

    Ok(Some(command))
}

fn available_labels(commands: &[CommandEntry]) -> String {
    if commands.is_empty() {
        return "(none)".to_string();
    }

    commands
        .iter()
        .map(|command| command.label.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn execute_or_print(
    command: &CommandEntry,
    target: Option<&Target>,
    dry_run: bool,
    history: &mut History,
) -> Result<i32> {
    if !dry_run {
        // Recorded before running, so a long-lived command still counts as picked.
        history.record(&command.label);
        return run_command(command, target);
    }

    let plan = plan_command(command, target)?;
    if let Some(cwd) = plan.cwd {
        println!("cwd: {}", cwd.display());
    }
    println!("command: {}", plan.command);

    Ok(0)
}
