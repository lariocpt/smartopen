use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::CommandEntry;
use crate::shell::Shell;
use crate::target::Target;
use crate::terminal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    pub command: String,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAvailability {
    Found { executable: String, path: PathBuf },
    Missing { executable: String },
    Dynamic { reason: String },
    Empty,
}

impl CommandAvailability {
    pub fn summary(&self) -> String {
        match self {
            Self::Found { executable, path } => {
                format!("{executable}: found at {}", path.display())
            }
            Self::Missing { executable } => format!("{executable}: missing from PATH"),
            Self::Dynamic { reason } => format!("dynamic shell command: {reason}"),
            Self::Empty => "empty command".to_string(),
        }
    }
}

/// Every placeholder a command may use. `{path}`… come from the first target; `{paths}`
/// is every target; the URL three render a file as `file://…` / `file` / empty.
pub const PLACEHOLDERS: &[&str] = &[
    "{path}", "{paths}", "{dir}", "{name}", "{stem}", "{ext}", "{url}", "{scheme}", "{host}",
];

/// Run the command and return the exit code the child reported, so the launcher exits the
/// way the launched program did. A detached launch reports 0: there is nothing to wait
/// for. Only a failure to start at all is an error.
pub fn run_command(command: &CommandEntry, targets: &[Target]) -> Result<i32> {
    let mut plan = plan_command(command, targets)?;

    // A new terminal window is a GUI launch: wrap the line for the terminal program and
    // let go of it, the same as `detach`.
    if command.terminal {
        plan.command = terminal::wrap(&plan.command, plan.cwd.as_deref())?;
    }

    if command.detach || command.terminal {
        spawn_detached(&plan, command)?;
        return Ok(0);
    }

    let mut process = Shell::current().command(&plan.command);
    process
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some(cwd) = &plan.cwd {
        process.current_dir(cwd);
    }

    let status = process
        .status()
        .with_context(|| format!("failed to run command '{}'", command.label))?;

    // No code means a signal killed it (Unix); 128+n is what shells report for that.
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + signal_number(&status)))
}

#[cfg(unix)]
fn signal_number(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.signal().unwrap_or(1)
}

#[cfg(not(unix))]
fn signal_number(_status: &std::process::ExitStatus) -> i32 {
    1
}

/// Launch a command fully in the background (GUI apps): no wait, no inherited stdio, so the
/// menu returns immediately and a non-zero exit is not surfaced — the opener's `orphan`.
///
/// "Fully" means its own process group (Unix) or a detached console (Windows). Without
/// that, closing the terminal that ran the menu takes the launched app down with it.
fn spawn_detached(plan: &ExecutionPlan, command: &CommandEntry) -> Result<()> {
    let mut process = Shell::current().command(&plan.command);
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(cwd) = &plan.cwd {
        process.current_dir(cwd);
    }

    detach_from_terminal(&mut process);

    process
        .spawn()
        .with_context(|| format!("failed to launch '{}'", command.label))?;
    Ok(())
}

#[cfg(unix)]
fn detach_from_terminal(process: &mut Command) {
    use std::os::unix::process::CommandExt;
    // A new process group: SIGHUP for the terminal's group no longer reaches the child.
    process.process_group(0);
}

#[cfg(windows)]
fn detach_from_terminal(process: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    process.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach_from_terminal(_process: &mut Command) {}

pub fn plan_command(command: &CommandEntry, targets: &[Target]) -> Result<ExecutionPlan> {
    let command_line = render_command(command, targets)?;
    let cwd = command
        .cwd
        .as_deref()
        .map(expand_path)
        .transpose()
        .with_context(|| format!("failed to prepare cwd for command '{}'", command.label))?;

    Ok(ExecutionPlan {
        command: command_line,
        cwd,
    })
}

pub fn command_availability(command_line: &str) -> CommandAvailability {
    match first_executable(command_line) {
        ExecutableHint::Name(executable) => match find_executable(&executable) {
            Some(path) => CommandAvailability::Found { executable, path },
            None => CommandAvailability::Missing { executable },
        },
        ExecutableHint::Dynamic(reason) => CommandAvailability::Dynamic { reason },
        ExecutableHint::Empty => CommandAvailability::Empty,
    }
}

/// Substitute the target placeholders, quoted for the shell this OS runs commands through.
pub fn render_command(command: &CommandEntry, targets: &[Target]) -> Result<String> {
    render_command_for(Shell::current(), command, targets)
}

/// [`render_command`] for an explicit shell, so both quoting rules are testable anywhere.
pub fn render_command_for(
    shell: Shell,
    command: &CommandEntry,
    targets: &[Target],
) -> Result<String> {
    let Some(first) = targets.first() else {
        if contains_placeholder(&command.run) {
            bail!(
                "shortcut '{}' uses a target placeholder, but no path or URL was provided",
                command.label
            );
        }

        return Ok(command.run.clone());
    };

    let quote = |value: &str| {
        shell
            .quote(value)
            .with_context(|| format!("cannot render command '{}'", command.label))
    };

    let mut all_paths = Vec::with_capacity(targets.len());
    for target in targets {
        all_paths.push(quote(&target.path.display().to_string())?);
    }
    let (scheme, host) = match &first.url {
        Some(url) => (url.scheme.as_str(), url.host.as_str()),
        None => ("file", ""),
    };

    Ok(command
        .run
        .replace("{paths}", &all_paths.join(" "))
        .replace("{path}", &quote(&first.path.display().to_string())?)
        .replace("{dir}", &quote(&first.dir.display().to_string())?)
        .replace("{name}", &quote(&first.name)?)
        .replace("{stem}", &quote(&first.stem)?)
        .replace("{ext}", &quote(&first.ext)?)
        .replace("{url}", &quote(&first.as_url_string())?)
        .replace("{scheme}", &quote(scheme)?)
        .replace("{host}", &quote(host)?))
}

fn expand_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path)
        .with_context(|| format!("failed to expand path '{path}'"))?
        .into_owned();
    Ok(PathBuf::from(expanded))
}

fn contains_placeholder(command: &str) -> bool {
    PLACEHOLDERS
        .iter()
        .any(|placeholder| command.contains(placeholder))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutableHint {
    Name(String),
    Dynamic(String),
    Empty,
}

fn first_executable(command_line: &str) -> ExecutableHint {
    let mut offset = 0;
    let mut allow_assignment = true;

    while let Some((word, next_offset)) = next_shell_word(command_line, offset) {
        offset = next_offset;

        if word.is_empty() {
            continue;
        }

        if starts_with_shell_expansion(&word) {
            return ExecutableHint::Dynamic(format!("starts with {word}"));
        }

        if allow_assignment && is_env_assignment(&word) {
            continue;
        }

        allow_assignment = false;

        if matches!(word.as_str(), "sudo" | "doas" | "command" | "exec") {
            continue;
        }

        if contains_shell_expansion(&word) {
            return ExecutableHint::Dynamic(format!("executable contains expansion: {word}"));
        }

        return ExecutableHint::Name(word);
    }

    ExecutableHint::Empty
}

fn next_shell_word(command_line: &str, start: usize) -> Option<(String, usize)> {
    let mut index = start;
    let mut word = String::new();
    let mut chars = command_line[start..].char_indices().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some((relative_index, ch)) = chars.peek().copied() {
        index = start + relative_index;
        if !in_single_quote && !in_double_quote && ch.is_whitespace() {
            chars.next();
            continue;
        }
        break;
    }

    while let Some((relative_index, ch)) = chars.next() {
        index = start + relative_index + ch.len_utf8();

        if !in_single_quote && !in_double_quote && ch.is_whitespace() {
            break;
        }

        if !in_single_quote && !in_double_quote && matches!(ch, '|' | ';' | '&' | '(') {
            break;
        }

        match ch {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '\\' if !in_single_quote => {
                if let Some((_, escaped)) = chars.next() {
                    word.push(escaped);
                    index += escaped.len_utf8();
                }
            }
            _ => word.push(ch),
        }
    }

    if word.is_empty() && index >= command_line.len() {
        None
    } else if word.is_empty() {
        next_shell_word(command_line, index)
    } else {
        Some((word, index))
    }
}

fn starts_with_shell_expansion(word: &str) -> bool {
    word.starts_with('$') || word.starts_with('`') || is_cmd_variable(word)
}

fn contains_shell_expansion(word: &str) -> bool {
    word.contains('$') || word.contains('`') || is_cmd_variable(word)
}

/// `%EDITOR%`-style cmd.exe expansion — only meaningful where cmd is the shell.
fn is_cmd_variable(word: &str) -> bool {
    cfg!(windows) && word.starts_with('%') && word.len() > 2 && word.ends_with('%')
}

fn is_env_assignment(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };

    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub fn find_executable(executable: &str) -> Option<PathBuf> {
    let executable_path = Path::new(executable);
    if executable_path.components().count() > 1 {
        return executable_path
            .exists()
            .then(|| executable_path.to_path_buf());
    }

    let path = env::var_os("PATH")?;
    let names = executable_names(executable);
    env::split_paths(&path)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

/// The file names a bare command may resolve to. On Windows `xan` is `xan.exe` (or any
/// other `PATHEXT` extension); everywhere else the name is the file name.
fn executable_names(executable: &str) -> Vec<String> {
    if !cfg!(windows) || Path::new(executable).extension().is_some() {
        return vec![executable.to_string()];
    }

    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut names = vec![executable.to_string()];
    names.extend(
        pathext
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(|ext| format!("{executable}{}", ext.to_lowercase())),
    );
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(run: &str) -> CommandEntry {
        CommandEntry {
            label: "Open".to_string(),
            run: run.to_string(),
            ..CommandEntry::default()
        }
    }

    fn render(shell: Shell, run: &str, targets: &[Target]) -> Result<String> {
        render_command_for(shell, &command(run), targets)
    }

    #[test]
    fn posix_render_single_quotes_path_placeholders() {
        let target = Target::fake_file("/tmp/project dir/hello world.rs");

        let rendered = render(Shell::Posix, "vim {path}", &[target]).unwrap();

        assert_eq!(rendered, "vim '/tmp/project dir/hello world.rs'");
    }

    #[test]
    fn cmd_render_double_quotes_path_placeholders() {
        // Built by hand: a Unix `Path` does not split on backslashes, and this test runs
        // on every OS.
        let mut target = Target::fake_file("/placeholder.rs");
        target.path = PathBuf::from(r"C:\project dir\hello world.rs");
        target.dir = PathBuf::from(r"C:\project dir");
        target.name = "hello world.rs".to_string();

        let rendered = render(Shell::Cmd, "micro {path} {dir} {name}", &[target]).unwrap();

        assert_eq!(
            rendered,
            r#"micro "C:\project dir\hello world.rs" "C:\project dir" "hello world.rs""#
        );
    }

    #[test]
    fn cmd_render_refuses_a_percent_in_the_path() {
        let target = Target::fake_file(r"C:\100%\a.rs");

        let error = render(Shell::Cmd, "micro {path}", &[target]).expect_err("% cannot be quoted");

        assert!(error.to_string().contains("cannot render command 'Open'"));
    }

    #[test]
    fn render_shortcut_rejects_target_placeholders() {
        assert!(render(Shell::Posix, "vim {path}", &[]).is_err());
        assert!(render(Shell::Posix, "open {url}", &[]).is_err());
        assert_eq!(
            render(Shell::Posix, "cargo test", &[]).unwrap(),
            "cargo test"
        );
    }

    #[test]
    fn render_command_preserves_shell_default_expansion() {
        let target = Target::fake_file("/tmp/file.rs");

        let rendered = render(Shell::Posix, "${EDITOR:-nano} {path}", &[target]).unwrap();

        assert_eq!(rendered, "${EDITOR:-nano} /tmp/file.rs");
    }

    #[test]
    fn paths_placeholder_renders_every_target_and_path_the_first() {
        let targets = [
            Target::fake_file("/tmp/a b.csv"),
            Target::fake_file("/tmp/c.csv"),
        ];

        let rendered = render(Shell::Posix, "xan cat {paths} --first {path}", &targets).unwrap();

        assert_eq!(
            rendered,
            "xan cat '/tmp/a b.csv' /tmp/c.csv --first '/tmp/a b.csv'"
        );
    }

    #[test]
    fn url_placeholders_come_from_the_url_and_degrade_for_files() {
        let url = Target::from_arg("https://Example.com/x/report.pdf").unwrap();
        let rendered = render(Shell::Posix, "open {url} {scheme} {host} {name}", &[url]).unwrap();
        assert_eq!(
            rendered,
            "open https://Example.com/x/report.pdf https example.com report.pdf"
        );

        let file = Target::fake_file("/tmp/a.txt");
        let rendered = render(Shell::Posix, "open {url} {scheme} {host}", &[file]).unwrap();
        assert_eq!(rendered, "open file:///tmp/a.txt file ''");
    }

    #[test]
    fn plan_command_expands_cwd() {
        let command = CommandEntry {
            label: "Build".to_string(),
            run: "cargo build".to_string(),
            cwd: Some(".".to_string()),
            ..CommandEntry::default()
        };

        let plan = plan_command(&command, &[]).expect("command should plan");

        assert_eq!(plan.command, "cargo build");
        assert!(plan.cwd.is_some());
    }

    #[test]
    fn first_executable_finds_simple_binary() {
        assert_eq!(
            first_executable("csvi {path}"),
            ExecutableHint::Name("csvi".to_string())
        );
    }

    #[test]
    fn first_executable_skips_env_assignments_and_wrappers() {
        assert_eq!(
            first_executable("FOO=bar sudo xan view {path}"),
            ExecutableHint::Name("xan".to_string())
        );
    }

    #[test]
    fn first_executable_marks_shell_expansion_as_dynamic() {
        assert_eq!(
            first_executable("${EDITOR:-nano} {path}"),
            ExecutableHint::Dynamic("starts with ${EDITOR:-nano}".to_string())
        );
    }

    #[test]
    fn command_availability_reports_missing_binary() {
        let availability = command_availability("definitely-not-installed-opn-test-binary {path}");

        assert_eq!(
            availability,
            CommandAvailability::Missing {
                executable: "definitely-not-installed-opn-test-binary".to_string()
            }
        );
    }

    #[test]
    fn executable_names_add_pathext_only_on_windows() {
        let names = executable_names("xan");
        if cfg!(windows) {
            assert!(names.iter().any(|name| name == "xan.exe"), "{names:?}");
        } else {
            assert_eq!(names, vec!["xan".to_string()]);
        }
        assert_eq!(executable_names("xan.exe"), vec!["xan.exe".to_string()]);
    }
}
