//! `smartopen shortcuts import <navi|pet|tldr> <source>`: turn the competitors' formats
//! into shortcuts, so switching costs one command and a tldr page becomes a shortcut group.
//!
//! navi's `.cheat`: `% tags`, `# description`, the command with `<arg>` placeholders, and
//! `$ arg: command` lines that supply choices. pet's `snippet.toml`: `[[snippets]]` with
//! `description`, `command` (`<param=default>`), `tag`. tldr's page: `# name`, `> about`,
//! then `- what it does:` above a fenced command with `{{arg}}` — already this tool's
//! parameter syntax. Output is TOML on stdout for review; `--write` appends it to the
//! config.

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Deserialize;

use crate::config::{CommandEntry, Config};
use crate::params::Param;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Source {
    Navi,
    Pet,
    Tldr,
}

pub fn import(source: Source, text: &str) -> Result<Vec<CommandEntry>> {
    let shortcuts = match source {
        Source::Navi => navi(text),
        Source::Pet => pet(text)?,
        Source::Tldr => tldr(text),
    };
    if shortcuts.is_empty() {
        bail!("nothing to import: no commands recognised in the input");
    }
    Ok(shortcuts)
}

/// The shortcuts as a config fragment: `[[shortcut]]` tables, ready to paste or append.
pub fn to_toml(shortcuts: Vec<CommandEntry>) -> Result<String> {
    let fragment = Config {
        shortcut: shortcuts,
        ..Config::default()
    };
    toml::to_string_pretty(&fragment).context("serialising shortcuts")
}

fn navi(text: &str) -> Vec<CommandEntry> {
    let mut out = Vec::new();
    let mut group: Option<String> = None;
    let mut description: Option<String> = None;
    let mut pending: Vec<CommandEntry> = Vec::new();

    let flush_choices = |pending: &mut Vec<CommandEntry>, out: &mut Vec<CommandEntry>| {
        out.append(pending);
    };

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(tags) = line.strip_prefix('%') {
            flush_choices(&mut pending, &mut out);
            group = tags
                .split(',')
                .map(str::trim)
                .find(|t| !t.is_empty())
                .map(str::to_string);
            continue;
        }
        if let Some(desc) = line.strip_prefix('#') {
            description = Some(desc.trim().to_string());
            continue;
        }
        if let Some(choice) = line.strip_prefix('$') {
            // `$ arg: command --- --column 1` supplies choices for the LAST command.
            if let Some((name, command)) = choice.split_once(':') {
                let name = name.trim();
                let command = command.split("---").next().unwrap_or("").trim().to_string();
                for entry in pending.iter_mut() {
                    if entry.param.contains_key(name) {
                        entry.param.insert(
                            name.to_string(),
                            Param {
                                choices: Some(command.clone()),
                                ..Param::default()
                            },
                        );
                    }
                }
            }
            continue;
        }

        // Anything else is a command; `<arg>` becomes `{{arg}}`.
        let (run, names) = angle_to_braces(line);
        let mut entry = CommandEntry {
            label: description.take().unwrap_or_else(|| line.to_string()),
            run,
            group: group.clone(),
            ..CommandEntry::default()
        };
        for name in names {
            let (name, param) = split_default(name);
            entry.param.insert(name, param);
        }
        pending.push(entry);
    }
    flush_choices(&mut pending, &mut out);
    out
}

#[derive(Deserialize)]
struct PetFile {
    #[serde(default)]
    snippets: Vec<PetSnippet>,
}

#[derive(Deserialize)]
struct PetSnippet {
    #[serde(default)]
    description: String,
    command: String,
    #[serde(default)]
    tag: Vec<String>,
}

fn pet(text: &str) -> Result<Vec<CommandEntry>> {
    let file: PetFile = toml::from_str(text).context("not a pet snippet.toml")?;
    Ok(file
        .snippets
        .into_iter()
        .map(|snippet| {
            let (run, names) = angle_to_braces(&snippet.command);
            let mut entry = CommandEntry {
                label: if snippet.description.is_empty() {
                    snippet.command.clone()
                } else {
                    snippet.description
                },
                run,
                group: snippet.tag.first().cloned(),
                ..CommandEntry::default()
            };
            for name in names {
                let (name, param) = split_default(name);
                entry.param.insert(name, param);
            }
            entry
        })
        .collect())
}

fn tldr(text: &str) -> Vec<CommandEntry> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut pending_label: Option<String> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if let Some(title) = line.strip_prefix("# ") {
            name = Some(title.trim().to_string());
            continue;
        }
        if let Some(label) = line.strip_prefix("- ") {
            pending_label = Some(label.trim_end_matches(':').trim().to_string());
            continue;
        }
        if line.starts_with('`') && line.ends_with('`') && line.len() >= 2 {
            let run = legalise_params(&line[1..line.len() - 1]);
            let Some(label) = pending_label.take() else {
                continue;
            };
            let mut entry = CommandEntry {
                label,
                run: run.clone(),
                group: name.clone(),
                ..CommandEntry::default()
            };
            for param in crate::params::names(&run) {
                entry.param.insert(param, Param::default());
            }
            if let Some(program) = name.as_deref() {
                entry.when = Some(crate::launcher::When {
                    has: vec![program.to_string()],
                    ..Default::default()
                });
            }
            out.push(entry);
        }
    }
    out
}

/// `<param=default>` carries its default through [`angle_to_braces`] as `name=default`;
/// split it back into the name and the `Param`. navi and pet both write that syntax —
/// navi's importer used to insert the whole `ref=HEAD` string as the parameter's NAME,
/// so `{{ref}}` had no table, the default was lost, and `--write` appended a dead one.
fn split_default(raw: String) -> (String, Param) {
    match raw.split_once('=') {
        Some((name, default)) => (
            name.to_string(),
            Param {
                default: Some(default.to_string()),
                ..Param::default()
            },
        ),
        None => (raw, Param::default()),
    }
}

/// A name the parameter parser will read back. tldr writes `{{path/to/file}}`,
/// `{{source.tar}}` and `{{file1 file2}}`, none of which [`crate::params::is_name`]
/// accepts — so the shortcut ran `tar xf {{source.tar}}` literally, without a word. Every
/// other character becomes `_`, runs collapse, the ends are trimmed; nothing left is `arg`.
fn legal_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "arg".to_string()
    } else {
        out.to_string()
    }
}

/// Every `{{…}}` in `run`, made a legal name.
fn legalise_params(run: &str) -> String {
    let mut out = String::with_capacity(run.len());
    let mut rest = run;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                out.push_str(&format!("{{{{{}}}}}", legal_name(&after[..end])));
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `<arg>` → `{{arg}}`, returning the names found (with any `=default` still attached,
/// which pet's caller splits off).
fn angle_to_braces(command: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(command.len());
    let mut names = Vec::new();
    let mut rest = command;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        match after.find('>') {
            Some(end) if !after[..end].is_empty() && !after[..end].contains(' ') => {
                let inner = &after[..end];
                let (name, default) = match inner.split_once('=') {
                    Some((name, default)) => (legal_name(name), Some(default)),
                    None => (legal_name(inner), None),
                };
                out.push_str(&rest[..start]);
                out.push_str(&format!("{{{{{name}}}}}"));
                names.push(match default {
                    Some(default) => format!("{name}={default}"),
                    None => name,
                });
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..=start]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    (out, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navi_cheats_become_grouped_parameterised_shortcuts_with_choices() {
        let cheat = "% git, vcs\n\n# Switch to a branch\ngit checkout <branch>\n\n$ branch: git branch --format='%(refname:short)' --- --column 1\n\n# Show the log\ngit log --oneline\n";
        let shortcuts = navi(cheat);
        assert_eq!(shortcuts.len(), 2);
        assert_eq!(shortcuts[0].label, "Switch to a branch");
        assert_eq!(shortcuts[0].run, "git checkout {{branch}}");
        assert_eq!(shortcuts[0].group.as_deref(), Some("git"));
        assert_eq!(
            shortcuts[0].param["branch"].choices.as_deref(),
            Some("git branch --format='%(refname:short)'")
        );
        assert_eq!(shortcuts[1].run, "git log --oneline");
        assert!(shortcuts[1].param.is_empty());
    }

    #[test]
    fn pet_snippets_carry_defaults_and_tags() {
        let toml = "[[snippets]]\ndescription = \"Ping a host\"\ncommand = \"ping -c <count=4> <host>\"\ntag = [\"net\"]\n";
        let shortcuts = pet(toml).unwrap();
        assert_eq!(shortcuts[0].label, "Ping a host");
        assert_eq!(shortcuts[0].run, "ping -c {{count}} {{host}}");
        assert_eq!(shortcuts[0].param["count"].default.as_deref(), Some("4"));
        assert_eq!(shortcuts[0].param["host"].default, None);
        assert_eq!(shortcuts[0].group.as_deref(), Some("net"));
    }

    #[test]
    fn navi_keeps_a_defaulted_arg_as_a_default_not_as_the_name() {
        // `<ref=HEAD>` used to become `[shortcut.param."ref=HEAD"]` — a table matching no
        // placeholder, with the default lost, which `--write` then appended to the config.
        let cheat = "% git\n# Reset hard to a ref\ngit reset --hard <ref=HEAD>\n";
        let shortcuts = navi(cheat);
        assert_eq!(shortcuts.len(), 1);
        assert_eq!(shortcuts[0].run, "git reset --hard {{ref}}");
        let param = shortcuts[0]
            .param
            .get("ref")
            .expect("the parameter is named for the placeholder");
        assert_eq!(param.default.as_deref(), Some("HEAD"));
        for name in crate::params::names(&shortcuts[0].run) {
            assert!(
                shortcuts[0].param.contains_key(&name),
                "{name} has no table"
            );
        }
    }

    #[test]
    fn tldr_pages_become_a_group_gated_on_the_program() {
        let page = "# tar\n\n> Archiving utility.\n\n- Create an archive:\n\n`tar cf {{target.tar}} {{file1 file2}}`\n\n- List the contents:\n\n`tar tvf {{archive.tar}}`\n";
        let shortcuts = tldr(page);
        // tldr's placeholder spellings are not parameter names; the review ran the
        // imported `tar xf {{source.tar}}` and it ran literally.
        assert_eq!(shortcuts[0].run, "tar cf {{target_tar}} {{file1_file2}}");
        assert_eq!(shortcuts[1].run, "tar tvf {{archive_tar}}");
        for shortcut in &shortcuts {
            for name in crate::params::names(&shortcut.run) {
                assert!(crate::params::is_name(&name), "{name}");
                assert!(shortcut.param.contains_key(&name), "{name} not a param");
            }
        }
        assert_eq!(shortcuts.len(), 2);
        assert_eq!(shortcuts[0].label, "Create an archive");
        assert_eq!(shortcuts[0].group.as_deref(), Some("tar"));
        assert_eq!(shortcuts[0].when.as_ref().unwrap().has, ["tar"]);
        assert_eq!(legal_name("path/to/file"), "path_to_file");
        assert_eq!(legal_name("my-arg"), "my-arg");
        assert_eq!(legal_name("  "), "arg");
        assert_eq!(
            angle_to_braces("scp <src.file=a.txt> <host>").0,
            "scp {{src_file}} {{host}}"
        );
        assert_eq!(
            angle_to_braces("scp <src.file=a.txt> <host>").1,
            vec!["src_file=a.txt".to_string(), "host".to_string()]
        );
    }

    #[test]
    fn output_is_a_config_fragment_that_parses_back() {
        let shortcuts = navi("# Hello\necho hi <name>\n");
        let text = to_toml(shortcuts).unwrap();
        assert!(text.contains("[[shortcut]]"), "{text}");
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.shortcut[0].run, "echo hi {{name}}");
    }

    #[test]
    fn unrecognised_input_is_an_error() {
        assert!(import(Source::Tldr, "nothing here").is_err());
    }
}
