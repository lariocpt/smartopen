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
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::config::CommandEntry;
use crate::fuzzy;
use crate::history::History;
use crate::matcher::Target;
use crate::runner::{command_availability, plan_command};

/// Rows a PageUp/PageDown moves by.
const PAGE: usize = 10;

pub fn select_command(
    prompt: &str,
    commands: &[CommandEntry],
    art: &str,
    target: Option<&Target>,
    history: &History,
) -> Result<Option<CommandEntry>> {
    if commands.is_empty() {
        return Ok(None);
    }

    let mut terminal = TerminalSession::new()?;
    let mut picker = Picker::new(prompt, commands, art, target, history);
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

/// One visible row: which command, and which characters of its label the query matched.
struct Row {
    index: usize,
    highlight: Vec<usize>,
}

struct Picker<'a> {
    prompt: &'a str,
    commands: &'a [CommandEntry],
    art: &'a str,
    target: Option<&'a Target>,
    history: &'a History,
    query: String,
    rows: Vec<Row>,
    selected: usize,
}

enum PickerAction {
    Continue,
    Cancel,
    Select(CommandEntry),
}

impl<'a> Picker<'a> {
    fn new(
        prompt: &'a str,
        commands: &'a [CommandEntry],
        art: &'a str,
        target: Option<&'a Target>,
        history: &'a History,
    ) -> Self {
        let mut picker = Self {
            prompt,
            commands,
            art,
            target,
            history,
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
        };
        picker.refresh_rows();
        picker
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
                    PickerAction::Select(command) => return Ok(Some(command)),
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
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(columns[0]);

        let filter = Paragraph::new(self.query.as_str())
            .block(Block::default().borders(Borders::ALL).title(self.prompt));
        frame.render_widget(filter, left[0]);

        let quick_numbers = self.query.is_empty();
        let items = self
            .rows
            .iter()
            .enumerate()
            .map(|(position, row)| {
                command_list_item(
                    &self.commands[row.index],
                    &row.highlight,
                    position,
                    quick_numbers,
                )
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        if !self.rows.is_empty() {
            state.select(Some(self.selected));
        }

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Items ({})", self.rows.len())),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::White)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, left[1], &mut state);

        let help = Paragraph::new(Line::from(vec![Span::styled(
            " type to filter · ↑↓ move · 1-9 pick · Alt+key · Enter run · Esc cancel",
            Style::default().fg(Color::DarkGray),
        )]));
        frame.render_widget(help, left[2]);

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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let query_empty = self.query.is_empty();

        match key.code {
            KeyCode::Esc => PickerAction::Cancel,
            KeyCode::Char('c') if ctrl => PickerAction::Cancel,
            KeyCode::Enter => self.select_current(),

            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            // Ctrl-n/Ctrl-p, never bare j/k: this is a filter box, and a letter that
            // moves the cursor instead is a letter you cannot type — "json" and "kill"
            // were unreachable while j/k navigated.
            KeyCode::Char('p') if ctrl => self.step(-1),
            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::Home => self.jump_to(0),
            KeyCode::End => self.jump_to(self.rows.len().saturating_sub(1)),
            KeyCode::PageUp => self.step(-(PAGE as isize)),
            KeyCode::PageDown => self.step(PAGE as isize),

            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refresh_rows();
                PickerAction::Continue
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refresh_rows();
                PickerAction::Continue
            }

            // 1-9 pick a row directly while the list is unfiltered; with a query they are
            // just characters, so "3d" and "mp4" stay typeable.
            KeyCode::Char(digit @ '1'..='9') if query_empty && !ctrl && !alt => {
                let position = digit.to_digit(10).unwrap() as usize - 1;
                match self.rows.get(position) {
                    Some(row) => PickerAction::Select(self.commands[row.index].clone()),
                    None => PickerAction::Continue,
                }
            }

            // Alt+<key> picks the command carrying that `key`, filter or no filter.
            KeyCode::Char(value) if alt && !ctrl => match self.command_with_key(value) {
                Some(command) => PickerAction::Select(command.clone()),
                None => PickerAction::Continue,
            },

            KeyCode::Char(value) if !ctrl => {
                self.query.push(value);
                self.refresh_rows();
                PickerAction::Continue
            }
            _ => PickerAction::Continue,
        }
    }

    fn select_current(&self) -> PickerAction {
        match self.selected_command() {
            Some(command) => PickerAction::Select(command.clone()),
            None => PickerAction::Continue,
        }
    }

    fn step(&mut self, delta: isize) -> PickerAction {
        if self.rows.is_empty() {
            self.selected = 0;
            return PickerAction::Continue;
        }
        let len = self.rows.len() as isize;
        let current = self.selected as isize;
        // Single steps wrap around; page steps clamp, so PageDown at the end stays put.
        self.selected = if delta.abs() == 1 {
            (current + delta).rem_euclid(len) as usize
        } else {
            (current + delta).clamp(0, len - 1) as usize
        };
        PickerAction::Continue
    }

    fn jump_to(&mut self, position: usize) -> PickerAction {
        if !self.rows.is_empty() {
            self.selected = position.min(self.rows.len() - 1);
        }
        PickerAction::Continue
    }

    fn command_with_key(&self, pressed: char) -> Option<&CommandEntry> {
        let pressed = pressed.to_lowercase().next()?;
        self.commands.iter().find(|command| {
            command
                .key
                .and_then(|key| key.to_lowercase().next())
                .is_some_and(|key| key == pressed)
        })
    }

    /// Recompute which commands are shown, in what order, with what highlighted.
    ///
    /// Empty query: every command, most frecent first, config order as the tiebreak.
    /// Otherwise: fuzzy matches only, best first — label matches outrank description
    /// matches, which outrank matches on the command text.
    fn refresh_rows(&mut self) {
        self.rows = if self.query.is_empty() {
            let mut rows: Vec<(f64, Row)> = self
                .commands
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    (
                        self.history.frecency(&command.label),
                        Row {
                            index,
                            highlight: Vec::new(),
                        },
                    )
                })
                .collect();
            rows.sort_by(|a, b| b.0.total_cmp(&a.0));
            rows.into_iter().map(|(_, row)| row).collect()
        } else {
            let mut scored: Vec<(u8, f64, Row)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(index, command)| {
                    let (tier, score, highlight) = score_command(&self.query, command)?;
                    Some((tier, score, Row { index, highlight }))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.total_cmp(&a.1)));
            scored.into_iter().map(|(_, _, row)| row).collect()
        };
        self.selected = 0;
    }

    fn selected_command(&self) -> Option<&CommandEntry> {
        self.rows
            .get(self.selected)
            .map(|row| &self.commands[row.index])
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

        if let Some(key) = command.key {
            lines.push(String::new());
            lines.push(format!("Hotkey: Alt+{key}"));
        }

        lines.join("\n")
    }
}

/// Match the query against a command: the label first, then the description, then the
/// command text. The tier says which one matched, so a label hit always outranks a
/// description hit regardless of raw score; highlights exist only for label hits.
fn score_command(query: &str, command: &CommandEntry) -> Option<(u8, f64, Vec<usize>)> {
    if let Some(m) = fuzzy::score(query, &command.label) {
        return Some((2, m.score, m.indices));
    }
    if let Some(m) = fuzzy::score(query, &command.description) {
        return Some((1, m.score, Vec::new()));
    }
    fuzzy::score(query, &command.run).map(|m| (0, m.score, Vec::new()))
}

fn command_list_item(
    command: &CommandEntry,
    highlight: &[usize],
    position: usize,
    quick_numbers: bool,
) -> ListItem<'static> {
    let mut spans = Vec::new();

    // Gutter: the 1-9 shortcut while unfiltered, or the command's own Alt+key.
    let gutter = match (command.key, quick_numbers && position < 9) {
        (Some(key), _) => format!("[{key}]"),
        (None, true) => format!(" {} ", position + 1),
        (None, false) => "   ".to_string(),
    };
    spans.push(Span::styled(gutter, Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw(" "));

    let icon = command.icon.trim();
    if !icon.is_empty() {
        spans.push(Span::raw(format!("{icon}  ")));
    }

    let matched = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let mut plain = String::new();
    for (index, ch) in command.label.trim().chars().enumerate() {
        if highlight.contains(&index) {
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            spans.push(Span::styled(ch.to_string(), matched));
        } else {
            plain.push(ch);
        }
    }
    if !plain.is_empty() {
        spans.push(Span::raw(plain));
    }

    ListItem::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<CommandEntry> {
        [
            "Edit",
            "Render Markdown",
            "Reveal",
            "Kill server",
            "JSON view",
        ]
        .into_iter()
        .map(|label| CommandEntry {
            label: label.to_string(),
            run: format!("echo {label}"),
            ..CommandEntry::default()
        })
        .collect()
    }

    fn picker<'a>(commands: &'a [CommandEntry], history: &'a History) -> Picker<'a> {
        Picker::new("Pick", commands, "", None, history)
    }

    fn labels(picker: &Picker<'_>) -> Vec<String> {
        picker
            .rows
            .iter()
            .map(|row| picker.commands[row.index].label.clone())
            .collect()
    }

    fn press(picker: &mut Picker<'_>, code: KeyCode, modifiers: KeyModifiers) -> PickerAction {
        picker.handle_key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn typing_j_and_k_filters_instead_of_moving() {
        let commands = commands();
        let history = History::disabled();
        let mut picker = picker(&commands, &history);

        press(&mut picker, KeyCode::Char('j'), KeyModifiers::NONE);
        press(&mut picker, KeyCode::Char('s'), KeyModifiers::NONE);
        assert_eq!(picker.query, "js");
        assert_eq!(labels(&picker), ["JSON view"]);

        press(&mut picker, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert!(picker.query.is_empty());
        press(&mut picker, KeyCode::Char('k'), KeyModifiers::NONE);
        press(&mut picker, KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(labels(&picker), ["Kill server"]);
    }

    #[test]
    fn ctrl_n_and_ctrl_p_move_and_wrap() {
        let commands = commands();
        let history = History::disabled();
        let mut picker = picker(&commands, &history);

        press(&mut picker, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected, 1);
        press(&mut picker, KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected, 0);
        // Wraps around, and moving never touched the query.
        press(&mut picker, KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected, commands.len() - 1);
        assert!(picker.query.is_empty());
    }

    #[test]
    fn digits_pick_directly_while_unfiltered() {
        let commands = commands();
        let history = History::disabled();
        let mut picker = picker(&commands, &history);

        match press(&mut picker, KeyCode::Char('3'), KeyModifiers::NONE) {
            PickerAction::Select(command) => assert_eq!(command.label, "Reveal"),
            _ => panic!("digit should select"),
        }
        assert!(matches!(
            press(&mut picker, KeyCode::Char('9'), KeyModifiers::NONE),
            PickerAction::Continue
        ));

        // With a query, digits are text.
        press(&mut picker, KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(matches!(
            press(&mut picker, KeyCode::Char('3'), KeyModifiers::NONE),
            PickerAction::Continue
        ));
        assert_eq!(picker.query, "m3");
    }

    #[test]
    fn alt_key_picks_the_command_with_that_key_even_while_filtering() {
        let mut commands = commands();
        commands[3].key = Some('K');
        let history = History::disabled();
        let mut picker = picker(&commands, &history);

        press(&mut picker, KeyCode::Char('e'), KeyModifiers::NONE);
        match press(&mut picker, KeyCode::Char('k'), KeyModifiers::ALT) {
            PickerAction::Select(command) => assert_eq!(command.label, "Kill server"),
            _ => panic!("Alt+k should select"),
        }
    }

    #[test]
    fn fuzzy_order_puts_label_matches_before_description_matches() {
        let mut commands = commands();
        commands[0].description = "reveal everything".to_string();
        let history = History::disabled();
        let mut picker = picker(&commands, &history);

        press(&mut picker, KeyCode::Char('r'), KeyModifiers::NONE);
        press(&mut picker, KeyCode::Char('e'), KeyModifiers::NONE);
        press(&mut picker, KeyCode::Char('v'), KeyModifiers::NONE);
        let shown = labels(&picker);
        assert_eq!(shown.last().unwrap(), "Edit", "{shown:?}");
        assert!(shown.contains(&"Reveal".to_string()));
        assert!(!picker.rows[0].highlight.is_empty());
    }

    #[test]
    fn history_floats_recent_picks_up_when_unfiltered() {
        let commands = commands();
        let mut history = History::disabled();
        history.record("Kill server");
        let picker = picker(&commands, &history);

        assert_eq!(labels(&picker)[0], "Kill server");
        assert_eq!(labels(&picker)[1], "Edit", "config order is the tiebreak");
    }

    #[test]
    fn home_end_and_pages_clamp() {
        let commands = commands();
        let history = History::disabled();
        let mut picker = picker(&commands, &history);

        press(&mut picker, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(picker.selected, commands.len() - 1);
        press(&mut picker, KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(picker.selected, commands.len() - 1);
        press(&mut picker, KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(picker.selected, 0);
        press(&mut picker, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(picker.selected, 0);
    }
}
