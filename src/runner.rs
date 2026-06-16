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

pub fn run_command(command: &CommandEntry, target: Option<&Target>) -> Result<()> {
    let plan = plan_command(command, target)?;

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
        };

        let plan = plan_command(&command, None).expect("command should plan");

        assert_eq!(plan.command, "cargo build");
        assert!(plan.cwd.is_some());
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's here"), "'it'\\''s here'");
    }
}
