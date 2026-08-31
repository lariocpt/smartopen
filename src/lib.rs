mod config;
mod doctor;
mod matcher;
mod menu;
mod render;
mod runner;
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
use crate::matcher::{Target, matching_commands};
use crate::menu::select_command;
use crate::runner::{plan_command, run_command, shell_quote};

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

    #[arg(long, help = "Configure yazi to use smartopen for file associations")]
    setup_yazi: bool,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = selected_config_path(cli.config_path.as_deref())?;

    if cli.config {
        println!("{}", config_path.display());
        return Ok(());
    }

    if cli.sample_config {
        print!("{SAMPLE_CONFIG}");
        return Ok(());
    }

    if cli.init_config {
        init_config(&config_path)?;
        println!("created {}", config_path.display());
        return Ok(());
    }

    if cli.edit_config {
        edit_config(&config_path)?;
        return Ok(());
    }

    if cli.setup_yazi {
        let effective = engine::effective(
            &spec::Spec::builtin(),
            engine::Engine::Smartopen,
            "smartopen",
        );
        let config_path = default_yazi_config_path()?;
        match tomlio::apply(&config_path, &effective, false, true)? {
            tomlio::Outcome::Created => println!("created {}", config_path.display()),
            tomlio::Outcome::Updated => println!("updated {}", config_path.display()),
            tomlio::Outcome::InSync => println!("already in sync: {}", config_path.display()),
        }
        return Ok(());
    }

    let config = load_config(&config_path)?;

    if cli.list {
        print!("{}", describe_config(&config, &config_path));
        return Ok(());
    }

    if cli.doctor {
        print_doctor(&config, &config_path)?;
        return Ok(());
    }

    match cli.path {
        Some(path) => {
            let target = Target::from_path(&path)?;
            let commands = matching_commands(&config, &config_path, &target)?;

            if commands.is_empty() {
                bail!("no matching commands for {}", target.path.display());
            }

            if cli.command.is_none() && commands.len() == 1 {
                execute_or_print(&commands[0], Some(&target), cli.dry_run)?;
                return Ok(());
            }

            let menu_art = menu_art_for_selection(&config, &config_path, &cli.command)?;
            if let Some(command) = resolve_command(
                "Choose a command",
                &commands,
                &cli.command,
                &menu_art,
                Some(&target),
            )? {
                execute_or_print(&command, Some(&target), cli.dry_run)?;
            }
        }
        None => {
            if config.shortcut.is_empty() {
                bail!("no shortcuts configured in {}", config_path.display());
            }

            let menu_art = menu_art_for_selection(&config, &config_path, &cli.command)?;
            if let Some(command) = resolve_command(
                "Choose a shortcut",
                &config.shortcut,
                &cli.command,
                &menu_art,
                None,
            )? {
                execute_or_print(&command, None, cli.dry_run)?;
            }
        }
    }

    Ok(())
}

fn selected_config_path(path: Option<&Path>) -> Result<PathBuf> {
    match path {
        Some(path) => expand_path(path),
        None => default_config_path(),
    }
}

fn default_yazi_config_path() -> Result<PathBuf> {
    let bd = directories::BaseDirs::new().context("cannot determine home/config directory")?;
    Ok(bd.config_dir().join("yazi").join("yazi.toml"))
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

fn edit_config(path: &Path) -> Result<()> {
    if !path.exists() {
        init_config(path)?;
        println!("created {}", path.display());
    }

    let run = format!(
        "${{EDITOR:-nano}} {}",
        shell_quote(&path.display().to_string())
    );
    let command = CommandEntry {
        label: "Edit config".to_string(),
        description: String::new(),
        icon: String::new(),
        run,
        cwd: None,
        detach: false,
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
) -> Result<Option<CommandEntry>> {
    let Some(label) = requested_label else {
        return select_command(prompt, commands, menu_art, target);
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

fn execute_or_print(command: &CommandEntry, target: Option<&Target>, dry_run: bool) -> Result<()> {
    if !dry_run {
        return run_command(command, target);
    }

    let plan = plan_command(command, target)?;
    if let Some(cwd) = plan.cwd {
        println!("cwd: {}", cwd.display());
    }
    println!("command: {}", plan.command);

    Ok(())
}
