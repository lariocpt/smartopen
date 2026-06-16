use anyhow::{Result, bail};

use crate::config::{CommandEntry, Config, load_menu_art};
use crate::runner::{CommandAvailability, command_availability};
use std::path::Path;

pub fn print_doctor(config: &Config, config_path: &Path) -> Result<()> {
    let mut problems = 0;

    println!("Config: {}", config_path.display());

    match load_menu_art(config, config_path) {
        Ok(art) if art.trim().is_empty() => println!("Menu art: empty"),
        Ok(_) => println!("Menu art: ok"),
        Err(error) => {
            problems += 1;
            println!("Menu art: problem - {error}");
        }
    }

    println!();
    println!("Commands:");
    for report in command_reports(config) {
        let status = command_availability(&report.command.run);
        if status.is_problem() {
            problems += 1;
        }

        println!("  {} - {}", report.context, availability_summary(&status));
    }

    println!();
    if problems == 0 {
        println!("Doctor: ok");
        return Ok(());
    }

    println!("Doctor: {problems} problem(s)");
    bail!("doctor found {problems} problem(s)")
}

struct CommandReport<'a> {
    context: String,
    command: &'a CommandEntry,
}

fn command_reports(config: &Config) -> Vec<CommandReport<'_>> {
    let mut reports = Vec::new();

    for association in &config.extension {
        let target = format!("extension {:?}", association.extensions);
        push_command_reports(&mut reports, target, &association.commands);
    }

    for association in &config.folder {
        let target = if association.names.is_empty() && association.paths.is_empty() {
            "folder any".to_string()
        } else {
            format!(
                "folder names={:?} paths={:?}",
                association.names, association.paths
            )
        };
        push_command_reports(&mut reports, target, &association.commands);
    }

    for association in &config.association {
        push_command_reports(
            &mut reports,
            "generic association".to_string(),
            &association.commands,
        );
    }

    push_command_reports(&mut reports, "shortcut".to_string(), &config.shortcut);

    reports
}

fn push_command_reports<'a>(
    reports: &mut Vec<CommandReport<'a>>,
    target: String,
    commands: &'a [CommandEntry],
) {
    for command in commands {
        reports.push(CommandReport {
            context: format!("{target} / {}", command.label),
            command,
        });
    }
}

fn availability_summary(status: &CommandAvailability) -> String {
    match status {
        CommandAvailability::Found { .. } => format!("ok ({})", status.summary()),
        CommandAvailability::Missing { .. } => format!("missing ({})", status.summary()),
        CommandAvailability::Dynamic { .. } => format!("dynamic ({})", status.summary()),
        CommandAvailability::Empty => status.summary(),
    }
}
