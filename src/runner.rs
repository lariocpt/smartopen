use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::CommandEntry;
use crate::matcher::Target;

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

    pub fn is_problem(&self) -> bool {
        matches!(self, Self::Missing { .. } | Self::Empty)
    }
}

pub fn run_command(command: &CommandEntry, target: Option<&Target>) -> Result<()> {
    let plan = plan_command(command, target)?;

    if command.detach {
        return spawn_detached(&plan, command);
    }

    let mut process = shell_command(&plan.command);
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

    if !status.success() {
        bail!("command '{}' exited with {status}", command.label);
    }

    Ok(())
}

/// Launch a command fully in the background (GUI apps): no wait, no inherited stdio, so the
/// menu returns immediately and a non-zero exit is not surfaced — the opener's `orphan`.
fn spawn_detached(plan: &ExecutionPlan, command: &CommandEntry) -> Result<()> {
    let mut process = shell_command(&plan.command);
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(cwd) = &plan.cwd {
        process.current_dir(cwd);
    }

    process
        .spawn()
        .with_context(|| format!("failed to launch '{}'", command.label))?;
    Ok(())
}

pub fn plan_command(command: &CommandEntry, target: Option<&Target>) -> Result<ExecutionPlan> {
    let command_line = render_command(command, target)?;
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

pub fn render_command(command: &CommandEntry, target: Option<&Target>) -> Result<String> {
    let Some(target) = target else {
        if contains_path_placeholder(&command.run) {
            bail!(
                "shortcut '{}' uses a path placeholder, but no path was provided",
                command.label
            );
        }

        return Ok(command.run.clone());
    };

    Ok(command
        .run
        .replace("{path}", &shell_quote_path(&target.path))
        .replace("{dir}", &shell_quote_path(&target.dir))
        .replace("{name}", &shell_quote(&target.name))
        .replace("{stem}", &shell_quote(&target.stem))
        .replace("{ext}", &shell_quote(&target.ext)))
}

fn expand_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path)
        .with_context(|| format!("failed to expand path '{path}'"))?
        .into_owned();
    Ok(PathBuf::from(expanded))
}

fn contains_path_placeholder(command: &str) -> bool {
    ["{path}", "{dir}", "{name}", "{stem}", "{ext}"]
        .iter()
        .any(|placeholder| command.contains(placeholder))
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut shell = Command::new("cmd");
        shell.arg("/C").arg(command);
        shell
    }

    #[cfg(not(windows))]
    {
        let mut shell = Command::new("sh");
        shell.arg("-c").arg(command);
        shell
    }
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.display().to_string())
}

pub fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
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
    word.starts_with('$') || word.starts_with('`')
}

fn contains_shell_expansion(word: &str) -> bool {
    word.contains('$') || word.contains('`')
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

fn find_executable(executable: &str) -> Option<PathBuf> {
    let executable_path = Path::new(executable);
    if executable_path.components().count() > 1 {
        return executable_path
            .exists()
            .then(|| executable_path.to_path_buf());
    }

    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target_with_path(path: PathBuf) -> Target {
        Target {
            path,
            dir: PathBuf::from("/tmp/project dir"),
            name: "hello world.rs".to_string(),
            stem: "hello world".to_string(),
            ext: "rs".to_string(),
            is_dir: false,
            is_empty: false,
        }
    }

    #[test]
    fn render_command_quotes_path_placeholders() {
        let command = CommandEntry {
            label: "Open".to_string(),
            description: String::new(),
            icon: String::new(),
            run: "vim {path}".to_string(),
            cwd: None,
            detach: false,
        };
        let target = target_with_path(PathBuf::from("/tmp/project dir/hello world.rs"));

        let rendered = render_command(&command, Some(&target)).expect("command should render");

        assert_eq!(rendered, "vim '/tmp/project dir/hello world.rs'");
    }

    #[test]
    fn render_shortcut_rejects_path_placeholder() {
        let command = CommandEntry {
            label: "Bad shortcut".to_string(),
            description: String::new(),
            icon: String::new(),
            run: "vim {path}".to_string(),
            cwd: None,
            detach: false,
        };

        assert!(render_command(&command, None).is_err());
    }

    #[test]
    fn render_command_preserves_shell_default_expansion() {
        let command = CommandEntry {
            label: "Edit".to_string(),
            description: String::new(),
            icon: String::new(),
            run: "${EDITOR:-nano} {path}".to_string(),
            cwd: None,
            detach: false,
        };
        let target = target_with_path(PathBuf::from("/tmp/file.rs"));

        let rendered = render_command(&command, Some(&target)).expect("command should render");

        assert_eq!(rendered, "${EDITOR:-nano} /tmp/file.rs");
    }

    #[test]
    fn plan_command_expands_cwd() {
        let command = CommandEntry {
            label: "Build".to_string(),
            description: String::new(),
            icon: String::new(),
            run: "cargo build".to_string(),
            cwd: Some(".".to_string()),
            detach: false,
        };

        let plan = plan_command(&command, None).expect("command should plan");

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
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's here"), "'it'\\''s here'");
    }
}
