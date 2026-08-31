mod broot;
mod catalog;
mod config;
mod diff;
mod doctor;
mod engine;
mod fuzzy;
mod history;
mod import;
mod installer;
mod launcher;
mod matcher;
mod menu;
mod mime;
mod navigators;
mod params;
mod paths;
mod platform;
mod render;
mod runner;
mod shell;
mod shell_widget;
mod spec;
mod target;
mod terminal;
mod tomlio;
mod wizard;

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{CommandFactory, Parser, Subcommand};

use crate::config::{
    CommandEntry, Config, SAMPLE_CONFIG, default_config_path, describe_config, find_project_config,
    init_config, load_config, load_menu_art, merge_project,
};
use crate::doctor::{diagnose, render_text};
use crate::history::History;
use crate::import::Source;
use crate::launcher::Context as WhenContext;
use crate::matcher::{default_command, matching_commands, shortcuts_here};
use crate::menu::select_command;
use crate::navigators::{Action as NavAction, Navigator};
use crate::params::TerminalPrompter;
use crate::runner::{PLACEHOLDERS, plan_command_with, plan_cwd, run_command};
use crate::shell::Shell;
use crate::shell_widget::ShellKind;
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

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Use a specific config file"
    )]
    config_path: Option<PathBuf>,

    // The flags below predate the subcommands and stay as hidden aliases so nothing that
    // calls `--setup-yazi` or `--doctor` breaks. `smartopen config …` is the documented
    // form.
    #[arg(long, hide = true)]
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

    #[arg(
        long,
        help = "Print the chosen command to stdout instead of running it (the shell widget's mode)"
    )]
    print: bool,

    #[arg(
        long,
        help = "Also list commands hidden by their `when` conditions, greyed, with the reason"
    )]
    all: bool,

    #[arg(
        long,
        value_name = "NAME=VALUE",
        help = "Preset a {{parameter}} instead of being asked (repeatable)"
    )]
    param: Vec<String>,

    #[arg(long, help = "Skip `confirm = true` prompts")]
    yes: bool,

    #[arg(long, hide = true)]
    edit_config: bool,

    #[arg(long, hide = true)]
    list: bool,

    #[arg(long, hide = true)]
    doctor: bool,

    #[arg(long, hide = true)]
    strict: bool,

    #[arg(long, hide = true)]
    json: bool,

    #[arg(long, hide = true)]
    sample_config: bool,

    #[arg(long, hide = true)]
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

    #[arg(long, hide = true)]
    setup_yazi: bool,

    #[arg(long, hide = true)]
    setup_broot: bool,
}

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Set up navigators and file associations step by step, installing tools on request
    Wizard {
        /// Show what would be written and run, then stop
        #[arg(long)]
        dry_run: bool,
        /// Take every recommendation without asking (the review is still shown)
        #[arg(long)]
        yes: bool,
        /// Write the config but never run a package manager
        #[arg(long)]
        no_install: bool,
    },
    /// The wizard's tool catalogue: what exists, what is installed, how to get the rest
    Tools {
        #[command(subcommand)]
        action: ToolsAction,
    },
    /// Show, create, edit or check the config
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Make yazi open files through this menu ([opener]/[open] in yazi.toml)
    Yazi {
        #[command(subcommand)]
        action: NavigatorAction,
        #[command(flatten)]
        opts: NavigatorOpts,
    },
    /// Make broot open files through this menu (an Enter verb in its config)
    Broot {
        #[command(subcommand)]
        action: NavigatorAction,
        #[command(flatten)]
        opts: NavigatorOpts,
    },
    /// Print a shell completion script (source it, or install it where the shell looks)
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print the manual page in roff (`smartopen man > smartopen.1`)
    Man,
    /// Print a shell snippet binding Ctrl-G to the launcher (source it from your rc file)
    Shell {
        #[arg(value_enum)]
        shell: ShellKind,
    },
    /// Work with shortcuts
    Shortcuts {
        #[command(subcommand)]
        action: ShortcutsAction,
    },
}

#[derive(Debug, Subcommand)]
enum ToolsAction {
    /// List every catalogue tool for this OS, marking the installed ones
    List {
        #[arg(long, help = "Print JSON instead of text")]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Print the config file path (and the project config, if one applies)
    Path,
    /// Open the config in $EDITOR, creating it first if needed
    Edit,
    /// Create a starter config if one does not exist
    Init,
    /// Print the starter config for this OS
    Sample,
    /// List configured associations and shortcuts
    List {
        #[arg(long, help = "Print JSON instead of text")]
        json: bool,
    },
    /// Check the config, menu art, and whether each command's program is installed
    Doctor {
        #[arg(long, help = "Print JSON instead of text")]
        json: bool,
        #[arg(
            long,
            help = "Exit 1 when a command is missing (the default reports and exits 0)"
        )]
        strict: bool,
    },
}

#[derive(Debug, Subcommand)]
enum NavigatorAction {
    /// Write the configuration (idempotent; the previous file is backed up first)
    Apply {
        /// Replace [opener]/[open] sections that were edited by hand (yazi)
        #[arg(long)]
        force: bool,
        /// Do not back up the existing file
        #[arg(long)]
        no_backup: bool,
    },
    /// Show a unified diff of what `apply` would change
    Diff,
    /// Exit 0 if already in sync, 1 if `apply` would change something
    Check,
    /// Print the rendered configuration fragment
    Print,
    /// Print the built-in spec in editable form, for use with --spec
    PrintSpec,
}

#[derive(Debug, clap::Args)]
struct NavigatorOpts {
    /// The binary the navigator delegates to (default: the one running now)
    #[arg(long, global = true, value_name = "NAME")]
    bin: Option<String>,
    /// yazi.toml, or broot's config directory, instead of the platform default
    #[arg(long, global = true, value_name = "PATH")]
    target: Option<PathBuf>,
    /// Explicit per-type viewers (the built-in spec) instead of delegating to the menu
    #[arg(long, global = true)]
    rules: bool,
    /// An external spec file instead of the built-in one (see `print-spec`)
    #[arg(long, global = true, value_name = "PATH")]
    spec: Option<PathBuf>,
}

impl From<NavigatorAction> for NavAction {
    fn from(action: NavigatorAction) -> Self {
        match action {
            NavigatorAction::Apply { force, no_backup } => NavAction::Apply { force, no_backup },
            NavigatorAction::Diff => NavAction::Diff,
            NavigatorAction::Check => NavAction::Check,
            NavigatorAction::Print => NavAction::Print,
            NavigatorAction::PrintSpec => NavAction::PrintSpec,
        }
    }
}

impl From<NavigatorOpts> for navigators::Options {
    fn from(opts: NavigatorOpts) -> Self {
        navigators::Options {
            bin: opts.bin,
            target: opts.target,
            rules: opts.rules,
            spec: opts.spec,
        }
    }
}

#[derive(Debug, Subcommand)]
enum ShortcutsAction {
    /// Convert navi cheats, a pet snippet file or a tldr page into [[shortcut]] TOML
    Import {
        #[arg(value_enum)]
        source: Source,
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Append to the config file instead of printing (a backup is written first)
        #[arg(long)]
        write: bool,
    },
}

/// The process exit code for `main`: the launched command's own code on success, 1 after
/// printing an error. Kept out of `run` so the binaries stay one line each.
pub fn main_exit_code() -> i32 {
    // `smartopen tools list | head` must end quietly, the way every Unix tool does, not
    // with a "failed printing to stdout" panic: let SIGPIPE terminate the process again.
    #[cfg(unix)]
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
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

    if let Some(subcommand) = cli.subcommand {
        return run_subcommand(subcommand, &config_path, cli.no_project);
    }

    // `--json` belongs to two of the legacy flags. On its own it used to be ignored, and
    // `smartopen --json file` opened the file.
    if cli.json && !(cli.list || cli.doctor) {
        bail!("--json goes with --list or --doctor: `config list --json`, `config doctor --json`");
    }

    // The hidden legacy flags map onto the subcommands they became.
    let legacy = if cli.config {
        Some(ConfigAction::Path)
    } else if cli.sample_config {
        Some(ConfigAction::Sample)
    } else if cli.init_config {
        Some(ConfigAction::Init)
    } else if cli.edit_config {
        Some(ConfigAction::Edit)
    } else if cli.list {
        Some(ConfigAction::List { json: cli.json })
    } else if cli.doctor {
        Some(ConfigAction::Doctor {
            json: cli.json,
            strict: cli.strict,
        })
    } else {
        None
    };
    if let Some(action) = legacy {
        return run_config_action(action, &config_path, cli.no_project);
    }
    if cli.setup_yazi || cli.setup_broot {
        let navigator = if cli.setup_yazi {
            Navigator::Yazi
        } else {
            Navigator::Broot
        };
        return navigators::run(
            navigator,
            NavAction::Apply {
                force: true,
                no_backup: false,
            },
            &navigators::Options::default(),
        );
    }

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

    // First run: no config anywhere and a person at the terminal. Offer the wizard
    // instead of an error that tells them to go write TOML.
    if !config_path.exists()
        && project_path.is_none()
        && cli.command.is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
    {
        eprint!(
            "No config found at {}.\nRun the setup wizard? [Y/n] ",
            config_path.display()
        );
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if matches!(answer.trim(), "" | "y" | "Y" | "yes") {
            let code = wizard::run(&config_path, wizard::Options::default())?;
            if code != 0 || !config_path.exists() {
                return Ok(code);
            }
        } else {
            bail!(
                "no config at {}\ncreate one with: smartopen wizard   (or: smartopen config init)",
                config_path.display()
            );
        }
    }

    let config = load_effective_config(&config_path, project_path.as_deref())?;

    let mut history = if cli.no_history {
        History::disabled()
    } else {
        paths::state_path()
            .map(History::load)
            .unwrap_or_else(History::disabled)
    };

    let mode = if cli.print {
        Mode::Print
    } else if cli.dry_run {
        Mode::DryRun
    } else {
        Mode::Run
    };
    let presets = parse_presets(&cli.param)?;
    let run_opts = RunOptions {
        mode,
        presets,
        yes: cli.yes,
    };
    // A cancelled pick prints nothing and exits 130 in --print mode, so the shell widget
    // can tell "chose nothing" from "chose an empty command"; otherwise it is just 0.
    let cancelled = if cli.print { 130 } else { 0 };
    let when_context = WhenContext::from_process();

    if !targets.is_empty() {
        let commands = matching_commands(&config, &config_path, &targets)?;
        let commands = apply_when(commands, &when_context, cli.all);

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
                return execute(command, &targets, &run_opts, &mut history, cancelled);
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
            Some(command) => execute(&command, &targets, &run_opts, &mut history, cancelled),
            None => Ok(cancelled),
        };
    }

    let shortcuts = apply_when(shortcuts_here(&config), &when_context, cli.all);
    if shortcuts.is_empty() {
        bail!(
            "no shortcuts apply here (see --all) in {}",
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
        Some(command) => execute(&command, &[], &run_opts, &mut history, cancelled),
        None => Ok(cancelled),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Run,
    DryRun,
    Print,
}

struct RunOptions {
    mode: Mode,
    presets: BTreeMap<String, String>,
    yes: bool,
}

/// `--param name=value` into a map; a bare name or an empty name is an error.
fn parse_presets(args: &[String]) -> Result<BTreeMap<String, String>> {
    args.iter()
        .map(|arg| match arg.split_once('=') {
            Some((name, value)) if !name.trim().is_empty() => {
                Ok((name.trim().to_string(), value.to_string()))
            }
            _ => bail!("--param wants NAME=VALUE, got '{arg}'"),
        })
        .collect()
}

/// Drop the commands whose `when` conditions fail — or, with `--all`, keep them marked
/// with the reason so the picker can show them greyed.
fn apply_when(
    commands: Vec<CommandEntry>,
    context: &WhenContext<'_>,
    keep_hidden: bool,
) -> Vec<CommandEntry> {
    commands
        .into_iter()
        .filter_map(|mut command| {
            let verdict = command
                .when
                .as_ref()
                .map_or(Ok(()), |when| when.check(context));
            match verdict {
                Ok(()) => Some(command),
                Err(reason) if keep_hidden => {
                    command.hidden_reason = Some(reason);
                    Some(command)
                }
                Err(_) => None,
            }
        })
        .collect()
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

/// `smartopen config …`, and the hidden legacy flags that map onto it.
fn run_config_action(action: ConfigAction, config_path: &Path, no_project: bool) -> Result<i32> {
    match action {
        ConfigAction::Path => {
            println!("{}", config_path.display());
            if let Some(project) = discover_project(no_project, &[]) {
                println!("project: {}", project.display());
            }
            Ok(0)
        }
        ConfigAction::Sample => {
            print!("{SAMPLE_CONFIG}");
            Ok(0)
        }
        ConfigAction::Init => {
            init_config(config_path)?;
            println!("created {}", config_path.display());
            Ok(0)
        }
        ConfigAction::Edit => edit_config(config_path),
        ConfigAction::List { json } => {
            let project_path = discover_project(no_project, &[]);
            let config = load_effective_config(config_path, project_path.as_deref())?;
            if json {
                let listing = serde_json::json!({
                    "config_path": config_path.display().to_string(),
                    "project_path": project_path.as_ref().map(|p| p.display().to_string()),
                    "config": config,
                });
                println!("{}", serde_json::to_string_pretty(&listing)?);
            } else {
                print!("{}", describe_config(&config, config_path));
            }
            Ok(0)
        }
        ConfigAction::Doctor { json, strict } => {
            let project_path = discover_project(no_project, &[]);
            let config = load_effective_config(config_path, project_path.as_deref())?;
            let report = diagnose(&config, config_path);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render_text(&report));
            }
            // A missing tool is a finding, not a failure: the config may name optional
            // viewers on purpose. --strict is for scripts that want the exit code to say.
            Ok(if strict && report.problems > 0 { 1 } else { 0 })
        }
    }
}

fn run_subcommand(subcommand: Subcommands, config_path: &Path, no_project: bool) -> Result<i32> {
    // Completions and the man page name whichever binary was invoked, so `opn
    // completions zsh` completes `opn`.
    let bin_name = navigators::current_bin_name();
    // clap wants a 'static name; one small leak at process start is the idiom.
    let bin_name: &'static str = Box::leak(bin_name.into_boxed_str());
    let mut command = Cli::command().name(bin_name);

    match subcommand {
        Subcommands::Wizard {
            dry_run,
            yes,
            no_install,
        } => {
            return wizard::run(
                config_path,
                wizard::Options {
                    dry_run,
                    yes,
                    no_install,
                },
            );
        }
        Subcommands::Tools {
            action: ToolsAction::List { json },
        } => return wizard::list_tools(json),
        Subcommands::Config { action } => {
            return run_config_action(action, config_path, no_project);
        }
        Subcommands::Yazi { action, opts } => {
            return navigators::run(Navigator::Yazi, action.into(), &opts.into());
        }
        Subcommands::Broot { action, opts } => {
            return navigators::run(Navigator::Broot, action.into(), &opts.into());
        }
        Subcommands::Completions { shell } => {
            clap_complete::generate(shell, &mut command, bin_name, &mut std::io::stdout());
        }
        Subcommands::Man => {
            let mut out = Vec::new();
            clap_mangen::Man::new(command).render(&mut out)?;
            std::io::stdout().write_all(&out)?;
        }
        Subcommands::Shell { shell } => {
            print!("{}", shell_widget::snippet(shell, bin_name));
        }
        Subcommands::Shortcuts {
            action:
                ShortcutsAction::Import {
                    source,
                    file,
                    write,
                },
        } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let shortcuts = import::import(source, &text)?;
            let count = shortcuts.len();
            let fragment = import::to_toml(shortcuts)?;
            if !write {
                print!("{fragment}");
                return Ok(0);
            }
            let config_path = default_config_path()?;
            if config_path.exists() {
                let backup = tomlio::backup(&config_path)?;
                eprintln!(
                    "backed up {} -> {}",
                    config_path.display(),
                    backup.display()
                );
            } else if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&config_path)
                .with_context(|| format!("opening {}", config_path.display()))?;
            writeln!(
                file,
                "\n# imported with `smartopen shortcuts import`\n{fragment}"
            )?;
            println!("appended {count} shortcut(s) to {}", config_path.display());
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

    run_command(&command, &[], None)
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

/// Fill in parameters, then run, print, or describe the command.
fn execute(
    command: &CommandEntry,
    targets: &[Target],
    opts: &RunOptions,
    history: &mut History,
    cancelled: i32,
) -> Result<i32> {
    // Parameters are ASKED for here; the renderer substitutes them and the target
    // placeholders together in one pass. The choices command runs in the shortcut's cwd,
    // like the shortcut itself will.
    let values = if params::names(&command.run).is_empty() {
        None
    } else {
        let cwd = plan_cwd(command, targets, None)?;
        let label = command.label.clone();
        let last = |name: &str| history.last_param(&label, name);
        let Some(values) = params::resolve(
            &command.run,
            &command.param,
            &opts.presets,
            &last,
            Shell::current(),
            cwd.as_deref(),
            &mut TerminalPrompter,
        )?
        else {
            return Ok(cancelled);
        };
        Some(values)
    };

    let plan = plan_command_with(command, targets, values.as_ref())?;

    // `{path}`, `{dir}`, `{name}`… are the first target by design; `{paths}` is all of
    // them. A selection from yazi arrives as several, and a command that names only the
    // first used to drop the rest without a word.
    if targets.len() > 1 && !command.run.contains("{paths}") {
        let first_only = PLACEHOLDERS
            .iter()
            .any(|p| *p != "{paths}" && command.run.contains(p));
        if first_only {
            eprintln!(
                "warning: {} of {} targets ignored: '{}' names only the first ({{path}}); use {{paths}} for all of them",
                targets.len() - 1,
                targets.len(),
                command.label
            );
        }
    }

    // Answers are remembered only once the command is really going to run — after the
    // confirm gate, never on --dry-run — or a declined `confirm` would still become the
    // "last" value next time.
    let remember = |history: &mut History| {
        history.record(&command.label);
        if let Some(values) = &values {
            history.record_params(&command.label, values);
        }
    };

    match opts.mode {
        Mode::Print => {
            // What the shell widget pastes: the command, with a `cd` when it wanted one.
            match plan.cwd {
                Some(cwd) if std::env::current_dir().ok().as_deref() != Some(cwd.as_path()) => {
                    println!(
                        "cd {} && {}",
                        Shell::current().quote(&cwd.display().to_string())?,
                        plan.command
                    );
                }
                _ => println!("{}", plan.command),
            }
            remember(history);
            Ok(0)
        }
        Mode::DryRun => {
            if let Some(cwd) = plan.cwd {
                println!("cwd: {}", cwd.display());
            }
            println!("command: {}", plan.command);
            if command.terminal {
                println!("terminal: yes");
            }
            Ok(0)
        }
        Mode::Run => {
            if command.confirm && !opts.yes && !confirmed(&plan.command)? {
                return Ok(cancelled);
            }
            // Recorded before running, so a long-lived command still counts as picked.
            remember(history);
            run_command(command, targets, values.as_ref())
        }
    }
}

/// `confirm = true`: show the rendered command, ask, default no.
fn confirmed(command_line: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("this command asks for confirmation and there is no terminal; pass --yes");
    }
    let mut stderr = std::io::stderr();
    write!(stderr, "run: {command_line}\nproceed? [y/N] ")?;
    stderr.flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
}
