mod config;
mod doctor;
mod fuzzy;
mod history;
mod matcher;
mod menu;
mod mime;
mod paths;
mod platform;
mod render;
mod runner;
mod shell;
mod target;
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
use clap::{CommandFactory, Parser, Subcommand};

use crate::config::{
    CommandEntry, Config, SAMPLE_CONFIG, default_config_path, describe_config, find_project_config,
    init_config, load_config, load_menu_art, merge_project,
};
use crate::doctor::{diagnose, render_text};
use crate::history::History;
use crate::matcher::{default_command, matching_commands, shortcuts_here};
use crate::menu::select_command;
use crate::runner::{plan_command, run_command};
use crate::shell::Shell;
use crate::target::{Target, targets_from_args};

#[derive(Debug, Parser)]
#[command(
    name = "smartopen",
    version,
    about = "Open files and shortcuts from configurable command menus"
)]
struct Cli {
    #[command(subcommand)]
    subcommand: Option<Subcommands>,

    #[arg(
        value_name = "PATH|URL",
        help = "Files, folders or URLs to open; several get only the commands they all share"
    )]
    targets: Vec<String>,

    #[arg(long, value_name = "PATH", help = "Use a specific config file")]
    config_path: Option<PathBuf>,

    #[arg(
        long,
        help = "Print the config file path (and the project config, if one applies)"
    )]
    config: bool,

    #[arg(
        long,
        help = "Ignore .smartopen.toml / .opn.toml files above the working directory or target"
    )]
    no_project: bool,

    #[arg(
        long,
        help = "Always show the menu, even for a single match or a `default` command"
    )]
    menu: bool,

    #[arg(long, help = "Open the config in $EDITOR, creating it first if needed")]
    edit_config: bool,

    #[arg(long, help = "List configured associations and shortcuts")]
    list: bool,

    #[arg(long, help = "Check config, menu art, and command availability")]
    doctor: bool,

    #[arg(
        long,
        help = "With --doctor: exit 1 when a command is missing (the default is to report and exit 0)"
    )]
    strict: bool,

    #[arg(long, help = "With --list or --doctor: print JSON instead of text")]
    json: bool,

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

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Print a shell completion script (source it, or install it where the shell looks)
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print the manual page in roff (`smartopen man > smartopen.1`)
    Man,
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

    if let Some(subcommand) = cli.subcommand {
        return run_subcommand(subcommand);
    }

    let config_path = selected_config_path(cli.config_path.as_deref())?;

    // Targets are resolved before the config loads: a target's directory is one of the
    // places a project config is searched for.
    let targets = if cli.targets.is_empty() {
        Vec::new()
    } else {
        targets_from_args(&cli.targets)?
    };
    let target_dirs: Vec<PathBuf> = targets
        .iter()
        .filter(|target| !target.is_url())
        .map(|target| target.dir.clone())
        .collect();
    let project_path = discover_project(cli.no_project, &target_dirs);

    if cli.config {
        println!("{}", config_path.display());
        if let Some(project) = &project_path {
            println!("project: {}", project.display());
        }
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

    let config = load_effective_config(&config_path, project_path.as_deref())?;

    if cli.list {
        if cli.json {
            let listing = serde_json::json!({
                "config_path": config_path.display().to_string(),
                "config": config,
            });
            println!("{}", serde_json::to_string_pretty(&listing)?);
        } else {
            print!("{}", describe_config(&config, &config_path));
        }
        return Ok(0);
    }

    if cli.doctor {
        let report = diagnose(&config, &config_path);
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{}", render_text(&report));
        }
        // A missing tool is a finding, not a failure: the config may name optional
        // viewers on purpose. --strict is for scripts that want the exit code to say.
        return Ok(if cli.strict && report.problems > 0 {
            1
        } else {
            0
        });
    }

    let mut history = if cli.no_history {
        History::disabled()
    } else {
        paths::state_path()
            .map(History::load)
            .unwrap_or_else(History::disabled)
    };

    if !targets.is_empty() {
        let commands = matching_commands(&config, &config_path, &targets)?;

        if commands.is_empty() {
            let named: Vec<String> = targets
                .iter()
                .map(|t| t.path.display().to_string())
                .collect();
            bail!("no matching commands for {}", named.join(", "));
        }

        // Skip the menu when there is nothing to choose: one match, or one command the
        // config marked `default`. --menu and --command both mean "I want to pick".
        if cli.command.is_none() && !cli.menu {
            let sole = (commands.len() == 1)
                .then(|| &commands[0])
                .or_else(|| default_command(&commands));
            if let Some(command) = sole {
                return execute_or_print(command, &targets, cli.dry_run, &mut history);
            }
        }

        let menu_art = menu_art_for_selection(&config, &config_path, &cli.command)?;
        return match resolve_command(
            "Choose a command",
            &commands,
            &cli.command,
            &menu_art,
            &targets,
            &history,
        )? {
            Some(command) => execute_or_print(&command, &targets, cli.dry_run, &mut history),
            None => Ok(0),
        };
    }

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
        &[],
        &history,
    )? {
        Some(command) => execute_or_print(&command, &[], cli.dry_run, &mut history),
        None => Ok(0),
    }
}

/// The project config that applies: searched up from the working directory, then from
/// each target's directory, unless `--no-project`.
fn discover_project(no_project: bool, target_dirs: &[PathBuf]) -> Option<PathBuf> {
    if no_project {
        return None;
    }
    let mut starts = Vec::with_capacity(target_dirs.len() + 1);
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    starts.extend(target_dirs.iter().cloned());
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    find_project_config(&starts, home.as_deref())
}

/// The user config with any project config layered over it. A project config alone is
/// enough to run — a repo that ships `.smartopen.toml` should work on a machine that
/// never ran `--init-config`.
fn load_effective_config(config_path: &Path, project_path: Option<&Path>) -> Result<Config> {
    let base = match project_path {
        Some(_) if !config_path.exists() => Config::default(),
        _ => load_config(config_path)?,
    };
    let Some(project_path) = project_path else {
        return Ok(base);
    };
    let project = load_config(project_path)
        .with_context(|| format!("in project config {}", project_path.display()))?;
    Ok(merge_project(base, project, project_path))
}

fn run_subcommand(subcommand: Subcommands) -> Result<i32> {
    // Completions and the man page name whichever binary was invoked, so `opn
    // completions zsh` completes `opn`.
    let bin_name = std::env::args()
        .next()
        .and_then(|arg0| {
            Path::new(&arg0)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "smartopen".to_string());
    // clap wants a 'static name; one small leak at process start is the idiom.
    let bin_name: &'static str = Box::leak(bin_name.into_boxed_str());
    let mut command = Cli::command().name(bin_name);

    match subcommand {
        Subcommands::Completions { shell } => {
            clap_complete::generate(shell, &mut command, bin_name, &mut std::io::stdout());
        }
        Subcommands::Man => {
            let mut out = Vec::new();
            clap_mangen::Man::new(command).render(&mut out)?;
            std::io::Write::write_all(&mut std::io::stdout(), &out)?;
        }
    }
    Ok(0)
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

    run_command(&command, &[])
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
    targets: &[Target],
    history: &History,
) -> Result<Option<CommandEntry>> {
    let Some(label) = requested_label else {
        return select_command(prompt, commands, menu_art, targets, history);
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
    targets: &[Target],
    dry_run: bool,
    history: &mut History,
) -> Result<i32> {
    if !dry_run {
        // Recorded before running, so a long-lived command still counts as picked.
        history.record(&command.label);
        return run_command(command, targets);
    }

    let plan = plan_command(command, targets)?;
    if let Some(cwd) = plan.cwd {
        println!("cwd: {}", cwd.display());
    }
    println!("command: {}", plan.command);

    Ok(0)
}
