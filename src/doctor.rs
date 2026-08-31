//! `--doctor`: is this config going to work on this machine?
//!
//! Produces a [`DoctorReport`] — a plain data structure — and renders it as text or JSON.
//! The report never decides the exit code; that is the caller's, because "a tool is
//! missing" is a finding, not a failure, unless the caller asked for `--strict`.

use std::path::Path;

use serde::Serialize;

use crate::config::{CommandEntry, Config, load_menu_art};
use crate::platform::{Host, Platform};
use crate::runner::{CommandAvailability, command_availability};

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub config_path: String,
    pub platform: &'static str,
    pub menu_art: MenuArtStatus,
    pub commands: Vec<CommandStatus>,
    /// Missing executables plus an unreadable menu art — what `--strict` fails on.
    pub problems: usize,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum MenuArtStatus {
    Ok,
    Empty,
    Problem { error: String },
}

#[derive(Debug, Serialize)]
pub struct CommandStatus {
    /// Where in the config the command sits, e.g. `extension ["csv"]` or `shortcut`.
    pub context: String,
    pub label: String,
    #[serde(flatten)]
    pub availability: Availability,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Availability {
    Ok {
        executable: String,
        path: String,
    },
    Missing {
        executable: String,
    },
    Dynamic {
        reason: String,
    },
    Empty,
    /// For another OS; not looked up here and not a problem.
    Skipped {
        platform: Platform,
    },
}

impl Availability {
    fn is_problem(&self) -> bool {
        matches!(self, Availability::Missing { .. } | Availability::Empty)
    }
}

pub fn diagnose(config: &Config, config_path: &Path) -> DoctorReport {
    let mut problems = 0;

    let menu_art = match load_menu_art(config, config_path) {
        Ok(art) if art.trim().is_empty() => MenuArtStatus::Empty,
        Ok(_) => MenuArtStatus::Ok,
        Err(error) => {
            problems += 1;
            MenuArtStatus::Problem {
                error: format!("{error:#}"),
            }
        }
    };

    let mut commands = Vec::new();
    for (context, command) in command_sites(config) {
        let availability = match command.platform {
            Some(platform) if !command.applies_here() => Availability::Skipped { platform },
            _ => match command_availability(&command.run) {
                CommandAvailability::Found { executable, path } => Availability::Ok {
                    executable,
                    path: path.display().to_string(),
                },
                CommandAvailability::Missing { executable } => Availability::Missing { executable },
                CommandAvailability::Dynamic { reason } => Availability::Dynamic { reason },
                CommandAvailability::Empty => Availability::Empty,
            },
        };
        if availability.is_problem() {
            problems += 1;
        }
        commands.push(CommandStatus {
            context,
            label: command.label.clone(),
            availability,
        });
    }

    DoctorReport {
        config_path: config_path.display().to_string(),
        platform: Host::current().name(),
        menu_art,
        commands,
        problems,
    }
}

pub fn render_text(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("Config: {}\n", report.config_path));
    out.push_str(&format!("Platform: {}\n", report.platform));
    out.push_str(&match &report.menu_art {
        MenuArtStatus::Ok => "Menu art: ok\n".to_string(),
        MenuArtStatus::Empty => "Menu art: empty\n".to_string(),
        MenuArtStatus::Problem { error } => format!("Menu art: problem - {error}\n"),
    });

    out.push_str("\nCommands:\n");
    for command in &report.commands {
        let summary = match &command.availability {
            Availability::Ok { executable, path } => format!("ok ({executable}: found at {path})"),
            Availability::Missing { executable } => {
                format!("missing ({executable}: missing from PATH)")
            }
            Availability::Dynamic { reason } => {
                format!("dynamic (dynamic shell command: {reason})")
            }
            Availability::Empty => "empty command".to_string(),
            Availability::Skipped { platform } => {
                format!("skipped (platform: {platform:?})").to_lowercase()
            }
        };
        out.push_str(&format!(
            "  {} / {} - {summary}\n",
            command.context, command.label
        ));
    }

    out.push('\n');
    if report.problems == 0 {
        out.push_str("Doctor: ok\n");
    } else {
        out.push_str(&format!("Doctor: {} problem(s)\n", report.problems));
    }
    out
}

/// Every command in the config with a description of where it sits.
fn command_sites(config: &Config) -> Vec<(String, &CommandEntry)> {
    let mut sites = Vec::new();

    for association in &config.extension {
        let context = format!("extension {:?}", association.extensions);
        sites.extend(association.commands.iter().map(|c| (context.clone(), c)));
    }

    for association in &config.folder {
        let context = if association.names.is_empty() && association.paths.is_empty() {
            "folder any".to_string()
        } else {
            format!(
                "folder names={:?} paths={:?}",
                association.names, association.paths
            )
        };
        sites.extend(association.commands.iter().map(|c| (context.clone(), c)));
    }

    for association in &config.url {
        let context = if association.schemes.is_empty() && association.hosts.is_empty() {
            "url any".to_string()
        } else {
            format!(
                "url schemes={:?} hosts={:?}",
                association.schemes, association.hosts
            )
        };
        sites.extend(association.commands.iter().map(|c| (context.clone(), c)));
    }

    for association in &config.association {
        sites.extend(
            association
                .commands
                .iter()
                .map(|c| ("generic association".to_string(), c)),
        );
    }

    sites.extend(config.shortcut.iter().map(|c| ("shortcut".to_string(), c)));

    sites
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MenuConfig;

    fn config_with_shortcuts(runs: &[(&str, &str, Option<Platform>)]) -> Config {
        Config {
            menu: MenuConfig::default(),
            extension: Vec::new(),
            folder: Vec::new(),
            url: Vec::new(),
            association: Vec::new(),
            shortcut: runs
                .iter()
                .map(|(label, run, platform)| CommandEntry {
                    label: label.to_string(),
                    run: run.to_string(),
                    platform: *platform,
                    ..CommandEntry::default()
                })
                .collect(),
        }
    }

    #[test]
    fn report_classifies_each_command_and_counts_only_real_problems() {
        let other_os = if cfg!(windows) {
            Platform::Linux
        } else {
            Platform::Windows
        };
        let config = config_with_shortcuts(&[
            (
                "Missing",
                "definitely-not-installed-smartopen-doctor-test",
                None,
            ),
            ("Dynamic", "${EDITOR:-nano}", None),
            ("Empty", "", None),
            (
                "Elsewhere",
                "definitely-not-installed-either",
                Some(other_os),
            ),
        ]);

        let report = diagnose(&config, Path::new("/tmp/config.toml"));

        let statuses: Vec<&str> = report
            .commands
            .iter()
            .map(|c| match &c.availability {
                Availability::Ok { .. } => "ok",
                Availability::Missing { .. } => "missing",
                Availability::Dynamic { .. } => "dynamic",
                Availability::Empty => "empty",
                Availability::Skipped { .. } => "skipped",
            })
            .collect();
        assert_eq!(statuses, ["missing", "dynamic", "empty", "skipped"]);
        assert_eq!(
            report.problems, 2,
            "missing + empty; dynamic and skipped are not problems"
        );
    }

    #[test]
    fn json_shape_is_flat_and_tagged() {
        let config = config_with_shortcuts(&[("Dynamic", "${EDITOR:-nano}", None)]);
        let report = diagnose(&config, Path::new("/tmp/config.toml"));

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["problems"], 0);
        assert_eq!(json["menu_art"]["status"], "ok");
        assert_eq!(json["commands"][0]["status"], "dynamic");
        assert_eq!(json["commands"][0]["label"], "Dynamic");
        assert!(json["commands"][0]["reason"].is_string());
    }

    #[test]
    fn text_rendering_ends_with_the_verdict() {
        let config = config_with_shortcuts(&[("Empty", "", None)]);
        let report = diagnose(&config, Path::new("/tmp/config.toml"));
        let text = render_text(&report);
        assert!(text.contains("shortcut / Empty - empty command"));
        assert!(text.ends_with("Doctor: 1 problem(s)\n"));
    }
}
