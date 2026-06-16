use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::config::CommandEntry;
use crate::matcher::Target;
use crate::runner::{command_availability, plan_command};

pub fn select_command(
    prompt: &str,
    commands: &[CommandEntry],
    art: &str,
    target: Option<&Target>,
) -> Result<Option<CommandEntry>> {
    if commands.is_empty() {
        return Ok(None);
    }

    let mut terminal = TerminalSession::new()?;
    let mut picker = Picker::new(prompt, commands, art, target);
    picker.run(&mut terminal.terminal)
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalSession {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

struct Picker<'a> {
    prompt: &'a str,
    commands: &'a [CommandEntry],
    art: &'a str,
    target: Option<&'a Target>,
    query: String,
    filtered: Vec<usize>,
    selected: usize,
}

impl<'a> Picker<'a> {
    fn new(
        prompt: &'a str,
        commands: &'a [CommandEntry],
        art: &'a str,
        target: Option<&'a Target>,
    ) -> Self {
        Self {
            prompt,
            commands,
            art,
            target,
            query: String::new(),
            filtered: (0..commands.len()).collect(),
            selected: 0,
        }
    }

    fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<Option<CommandEntry>> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match self.handle_key(key) {
                    PickerAction::Continue => {}
                    PickerAction::Cancel => return Ok(None),
                    PickerAction::Select => {
                        return Ok(self.selected_command().map(|command| command.to_owned()));
                    }
                }
            }
        }
    }

    fn draw(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let art_height = self.art_height(area.height);
        let rows = if art_height == 0 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(art_height), Constraint::Min(0)])
                .split(area)
        };

        let body = if art_height == 0 {
            rows[0]
        } else {
            let art = Paragraph::new(self.art)
                .block(Block::default())
                .wrap(Wrap { trim: false });
            frame.render_widget(art, rows[0]);
            rows[1]
        };

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(body);

        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(columns[0]);

        let filter = Paragraph::new(self.query.as_str())
            .block(Block::default().borders(Borders::ALL).title(self.prompt));
        frame.render_widget(filter, left[0]);

        let items = self
            .filtered
            .iter()
            .map(|index| command_list_item(&self.commands[*index]))
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        if !self.filtered.is_empty() {
            state.select(Some(self.selected));
        }

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Items ({})", self.filtered.len())),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::White)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, left[1], &mut state);

        let detail = Paragraph::new(self.detail_text())
            .block(Block::default().borders(Borders::ALL).title("Action"))
            .wrap(Wrap { trim: false });
        frame.render_widget(detail, columns[1]);
    }

    fn art_height(&self, terminal_height: u16) -> u16 {
        if self.art.trim().is_empty() || terminal_height < 10 {
            return 0;
        }

        let line_count = self.art.lines().count() as u16;
        line_count.min(terminal_height / 3).min(8)
    }

    fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Esc => PickerAction::Cancel,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                PickerAction::Cancel
            }
            KeyCode::Enter => {
                if self.filtered.is_empty() {
                    PickerAction::Continue
                } else {
                    PickerAction::Select
                }
            }
            KeyCode::Up => {
                self.previous();
                PickerAction::Continue
            }
            KeyCode::Down => {
                self.next();
                PickerAction::Continue
            }
            KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.previous();
                PickerAction::Continue
            }
            KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.next();
                PickerAction::Continue
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refresh_filter();
                PickerAction::Continue
            }
            KeyCode::Char(value) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(value);
                self.refresh_filter();
                PickerAction::Continue
            }
            _ => PickerAction::Continue,
        }
    }

    fn previous(&mut self) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }

        self.selected = if self.selected == 0 {
            self.filtered.len() - 1
        } else {
            self.selected - 1
        };
    }

    fn next(&mut self) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }

        self.selected = (self.selected + 1) % self.filtered.len();
    }

    fn refresh_filter(&mut self) {
        let query = self.query.to_lowercase();
        self.filtered = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let haystack = format!("{} {} {}", command.label, command.description, command.run)
                    .to_lowercase();

                haystack.contains(&query).then_some(index)
            })
            .collect();

        self.selected = 0;
    }

    fn selected_command(&self) -> Option<&CommandEntry> {
        self.filtered
            .get(self.selected)
            .map(|index| &self.commands[*index])
    }

    fn detail_text(&self) -> String {
        let Some(command) = self.selected_command() else {
            return "No matching items".to_string();
        };

        let mut lines = Vec::new();
        lines.push(command.label.clone());

        if !command.description.trim().is_empty() {
            lines.push(String::new());
            lines.push(command.description.clone());
        }

        lines.push(String::new());
        match plan_command(command, self.target) {
            Ok(plan) => {
                lines.push("Command".to_string());
                lines.push(plan.command.clone());
                lines.push(String::new());
                lines.push("Executable".to_string());
                lines.push(command_availability(&plan.command).summary());
                if let Some(cwd) = plan.cwd {
                    lines.push(String::new());
                    lines.push("Working directory".to_string());
                    lines.push(cwd.display().to_string());
                }
            }
            Err(error) => {
                lines.push("Command preview error".to_string());
                lines.push(error.to_string());
            }
        }

        if let Some(target) = self.target {
            lines.push(String::new());
            lines.push("Target".to_string());
            lines.push(target.path.display().to_string());
            lines.push(format!(
                "type: {}",
                if target.is_dir { "folder" } else { "file" }
            ));
            if !target.ext.is_empty() {
                lines.push(format!("extension: {}", target.ext));
            }
        } else {
            lines.push(String::new());
            lines.push("Target".to_string());
            lines.push("shortcut".to_string());
        }

        lines.join("\n")
    }
}

enum PickerAction {
    Continue,
    Cancel,
    Select,
}

fn command_list_item(command: &CommandEntry) -> ListItem<'static> {
    let icon = command.icon.trim();
    let label = command.label.trim();
    let text = if icon.is_empty() {
        label.to_string()
    } else {
        format!("{icon}  {label}")
    };

    ListItem::new(Line::from(text))
}
