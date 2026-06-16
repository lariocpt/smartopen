use std::fmt;

use anyhow::Result;
use inquire::{Select, ui::RenderConfig};

use crate::config::CommandEntry;

#[derive(Clone)]
struct MenuItem {
    command: CommandEntry,
}

impl MenuItem {
    fn new(command: CommandEntry) -> Self {
        Self { command }
    }

    fn into_command(self) -> CommandEntry {
        self.command
    }
}

impl fmt::Display for MenuItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icon = self.command.icon.trim();
        let label = self.command.label.trim();
        let description = self.command.description.trim();

        match (icon.is_empty(), description.is_empty()) {
            (true, true) => write!(f, "{label}"),
            (false, true) => write!(f, "{icon}  {label}"),
            (true, false) => write!(f, "{label} - {description}"),
            (false, false) => write!(f, "{icon}  {label} - {description}"),
        }
    }
}

pub fn select_command(prompt: &str, commands: &[CommandEntry]) -> Result<Option<CommandEntry>> {
    match commands {
        [] => Ok(None),
        [command] => Ok(Some(command.clone())),
        _ => {
            let items = commands.iter().cloned().map(MenuItem::new).collect();
            let selection = Select::new(prompt, items)
                .with_page_size(12)
                .with_render_config(RenderConfig::default())
                .prompt()?;

            Ok(Some(selection.into_command()))
        }
    }
}
