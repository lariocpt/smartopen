//! `{{name}}` parameters: filled in before a command runs.
//!
//! Distinct from the `{path}` target placeholders — one brace is the target, two braces
//! is a question for the user. Each parameter can carry a prompt, a default (or `"last"`,
//! the value used last time), and a `choices` command whose stdout lines are offered in
//! the picker. Values are shell-quoted on the way in, the same guarantee target
//! placeholders have; `--param name=value` presets one from the command line.

use std::collections::BTreeMap;
use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::shell::Shell;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// What to ask; the parameter name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// A literal default, or `"last"` for whatever was entered last time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// A command whose stdout lines become the choices, run in the shortcut's `cwd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<String>,
}

/// What may sit between the braces of `{{name}}`. The renderer in `runner.rs` uses the
/// same rule, so a `{{ }}` or `{{a b}}` is literal text to both.
pub fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The `{{name}}` parameters a command line mentions, in first-appearance order.
pub fn names(run: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut rest = run;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = after[..end].trim();
        if is_name(name) && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        rest = &after[end + 2..];
    }
    names
}

/// How a value is asked for: injected so tests never touch a terminal.
pub trait Prompter {
    /// Pick one of `choices`; `None` means the user cancelled.
    fn choose(
        &mut self,
        prompt: &str,
        choices: &[String],
        preferred: Option<&str>,
    ) -> Result<Option<String>>;
    /// Free text, with a default shown; `None` means cancelled.
    fn ask(&mut self, prompt: &str, default: Option<&str>) -> Result<Option<String>>;
}

/// Resolve every parameter of `run`, in order. `presets` come from `--param`; `last`
/// answers `default = "last"`; `choices` commands run through `shell` in `cwd`.
/// Returns `None` if the user cancelled a prompt.
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    run: &str,
    params: &BTreeMap<String, Param>,
    presets: &BTreeMap<String, String>,
    last: &dyn Fn(&str) -> Option<String>,
    shell: Shell,
    cwd: Option<&std::path::Path>,
    prompter: &mut dyn Prompter,
) -> Result<Option<BTreeMap<String, String>>> {
    let mut values = BTreeMap::new();
    for name in names(run) {
        if let Some(preset) = presets.get(&name) {
            values.insert(name, preset.clone());
            continue;
        }

        let param = params.get(&name).cloned().unwrap_or_default();
        let prompt = param.prompt.clone().unwrap_or_else(|| name.clone());
        let default = match param.default.as_deref() {
            Some("last") => last(&name),
            Some(literal) => Some(literal.to_string()),
            None => None,
        };

        let value = match &param.choices {
            Some(command) => {
                let choices = run_choices(command, shell, cwd)
                    .with_context(|| format!("choices for {{{{{name}}}}}"))?;
                if choices.is_empty() {
                    bail!("choices command for {{{{{name}}}}} produced no lines: {command}");
                }
                prompter.choose(&prompt, &choices, default.as_deref())?
            }
            None => prompter.ask(&prompt, default.as_deref())?,
        };

        match value {
            Some(value) => {
                values.insert(name, value);
            }
            None => return Ok(None),
        }
    }
    Ok(Some(values))
}

fn run_choices(command: &str, shell: Shell, cwd: Option<&std::path::Path>) -> Result<Vec<String>> {
    let mut process = shell.command(command);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }
    let output = process
        .output()
        .with_context(|| format!("failed to run `{command}`"))?;
    if !output.status.success() {
        bail!(
            "`{command}` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// The real prompter: the picker for choices, a line read for free text.
pub struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn choose(
        &mut self,
        prompt: &str,
        choices: &[String],
        preferred: Option<&str>,
    ) -> Result<Option<String>> {
        use crate::config::CommandEntry;
        use crate::history::History;
        use crate::menu::select_command;

        // The preferred (last-used) choice goes first, so Enter takes it.
        let mut ordered: Vec<&String> = choices.iter().collect();
        if let Some(preferred) = preferred
            && let Some(position) = ordered.iter().position(|c| *c == preferred)
        {
            let hit = ordered.remove(position);
            ordered.insert(0, hit);
        }
        let entries: Vec<CommandEntry> = ordered
            .into_iter()
            .map(|choice| CommandEntry {
                label: choice.clone(),
                run: choice.clone(),
                ..CommandEntry::default()
            })
            .collect();
        Ok(select_command(prompt, &entries, "", &[], &History::disabled())?.map(|e| e.label))
    }

    fn ask(&mut self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        if !io::stdin().is_terminal() {
            bail!(
                "parameter '{prompt}' needs a terminal to ask for a value; use --param {prompt}=…"
            );
        }
        let mut stderr = io::stderr();
        match default {
            Some(default) => write!(stderr, "{prompt} [{default}]: ")?,
            None => write!(stderr, "{prompt}: ")?,
        }
        stderr.flush()?;

        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let typed = line.trim_end_matches(['\n', '\r']);
        Ok(Some(match (typed.is_empty(), default) {
            (true, Some(default)) => default.to_string(),
            _ => typed.to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_ordered_unique_and_ignore_target_placeholders() {
        assert_eq!(
            names("git checkout {{branch}} -- {path} {{file}} {{branch}}"),
            ["branch", "file"]
        );
        assert!(names("no params {path}").is_empty());
        assert!(names("{{ spaced }}").contains(&"spaced".to_string()));
        assert!(
            names("{{bad name}}").is_empty(),
            "spaces inside a name are not a name"
        );
    }

    struct Scripted(Vec<Option<String>>);
    impl Prompter for Scripted {
        fn choose(
            &mut self,
            _: &str,
            choices: &[String],
            preferred: Option<&str>,
        ) -> Result<Option<String>> {
            // "Enter": the preferred choice if any, else the first.
            let _ = self.0.remove(0);
            Ok(preferred
                .map(str::to_string)
                .or_else(|| choices.first().cloned()))
        }
        fn ask(&mut self, _: &str, default: Option<&str>) -> Result<Option<String>> {
            Ok(self.0.remove(0).map(|typed| {
                if typed.is_empty() {
                    default.unwrap_or("").to_string()
                } else {
                    typed
                }
            }))
        }
    }

    #[test]
    fn resolve_uses_presets_defaults_last_values_and_choices() {
        let mut params = BTreeMap::new();
        params.insert(
            "branch".to_string(),
            Param {
                choices: Some("printf 'main\\nfeature\\n'".to_string()),
                default: Some("last".to_string()),
                ..Param::default()
            },
        );
        params.insert(
            "msg".to_string(),
            Param {
                default: Some("wip".to_string()),
                ..Param::default()
            },
        );
        let presets: BTreeMap<String, String> = [("host".to_string(), "box".to_string())]
            .into_iter()
            .collect();
        let last = |name: &str| (name == "branch").then(|| "feature".to_string());
        let mut prompter = Scripted(vec![Some(String::new()), Some(String::new())]);

        let values = resolve(
            "deploy {{host}} {{branch}} {{msg}}",
            &params,
            &presets,
            &last,
            Shell::Posix,
            None,
            &mut prompter,
        )
        .unwrap()
        .expect("not cancelled");

        assert_eq!(values["host"], "box", "preset wins, no prompt");
        assert_eq!(
            values["branch"], "feature",
            "last value is the preferred choice"
        );
        assert_eq!(values["msg"], "wip", "empty answer takes the default");
    }

    #[test]
    fn a_failing_choices_command_is_an_error_not_an_empty_list() {
        let mut params = BTreeMap::new();
        params.insert(
            "x".to_string(),
            Param {
                choices: Some("exit 3".to_string()),
                ..Param::default()
            },
        );
        let mut prompter = Scripted(vec![Some(String::new())]);
        let error = resolve(
            "echo {{x}}",
            &params,
            &BTreeMap::new(),
            &|_| None,
            Shell::Posix,
            None,
            &mut prompter,
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("choices for {{x}}"), "{error:#}");
    }

    #[test]
    fn cancelling_a_prompt_cancels_the_run() {
        let mut prompter = Scripted(vec![None]);
        let outcome = resolve(
            "echo {{x}}",
            &BTreeMap::new(),
            &BTreeMap::new(),
            &|_| None,
            Shell::Posix,
            None,
            &mut prompter,
        )
        .unwrap();
        assert!(outcome.is_none());
    }
}
