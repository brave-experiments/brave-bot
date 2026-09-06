//! Choosing how hard the model thinks before it answers.
//!
//! Shown by `/effort`. The list is the levels the protocol defines plus the row that asks for
//! none, and the choice is written to `~/.bravebot`: it outlives the session that made it and
//! applies in every directory, the same as the model and the theme.
//!
//! Drawn as a centred panel over the session, so the transcript stays visible. Enter takes the row
//! under the cursor. Escape leaves the level that was in force alone.
//!
//! Nothing labelled is involved. The words never reach a model as a decision: they are drawn for a
//! person, who picks one, and their pick is what endorses the request field it lands in.

use bravebot_aichat::protocol::Effort;
use bravebot_i18n::t;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::theme;

/// One row of the list.
///
/// The row that asks for no level is offered because it is a choice somebody may want back: a
/// picker of five levels and no way out makes the first pick permanent, and the absent field is
/// what every service applies its own default to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row(pub Option<Effort>);

impl Row {
    /// Every row, the service's own default first and then the levels cheapest first.
    fn all() -> Vec<Row> {
        std::iter::once(Row(None))
            .chain(Effort::ALL.into_iter().map(|level| Row(Some(level))))
            .collect()
    }

    /// The word shown for this row.
    ///
    /// A level is drawn as the word it is sent and stored as, untranslated: it is an identifier
    /// that appears in a settings file and in the request, and a person comparing what they picked
    /// against what a service documents is comparing the same word.
    fn name(self) -> String {
        match self.0 {
            Some(level) => level.as_str().to_string(),
            None => t!(effort_unset).to_string(),
        }
    }

    /// What this row is for, in one line under the list.
    ///
    /// A match rather than a lookup, so every sentence is named in the source: nothing computed
    /// while running can choose which of them a person is shown.
    fn hint(self) -> &'static str {
        match self.0 {
            None => t!(effort_hint_unset),
            Some(Effort::Low) => t!(effort_hint_low),
            Some(Effort::Medium) => t!(effort_hint_medium),
            Some(Effort::High) => t!(effort_hint_high),
            Some(Effort::Xhigh) => t!(effort_hint_xhigh),
            Some(Effort::Max) => t!(effort_hint_max),
        }
    }
}

/// What the picker is showing and where the cursor is.
#[derive(Debug)]
pub struct Picker {
    rows: Vec<Row>,
    /// Which row is under the cursor.
    selected: usize,
    /// The level in force when the picker opened, marked in the list.
    previous: Row,
}

impl Picker {
    /// Open on the level in use, since that is the row a person is looking for.
    pub fn new(current: Option<Effort>) -> Self {
        let rows = Row::all();
        let previous = Row(current);
        let selected = rows.iter().position(|row| *row == previous).unwrap_or(0);
        Self {
            rows,
            selected,
            previous,
        }
    }

    /// The row under the cursor.
    pub fn chosen(&self) -> Option<Row> {
        self.rows.get(self.selected).copied()
    }

    fn down(&mut self) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn is_current(&self, row: Row) -> bool {
        row == self.previous
    }
}

/// What a key press did to the picker.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still choosing.
    Continue,
    /// Take the row under the cursor.
    Select,
    /// Leave the level that was in force alone.
    Cancel,
}

/// Interpret one key press.
pub fn handle_key(picker: &mut Picker, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
    if modifiers.contains(KeyModifiers::CONTROL) {
        return match code {
            KeyCode::Char('c') => Outcome::Cancel,
            _ => Outcome::Continue,
        };
    }

    match code {
        KeyCode::Esc => Outcome::Cancel,
        KeyCode::Enter => Outcome::Select,
        KeyCode::Up | KeyCode::Char('k') => {
            picker.up();
            Outcome::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            picker.down();
            Outcome::Continue
        }
        _ => Outcome::Continue,
    }
}

/// Show the list as a centred panel over whatever `behind` draws, and return the row the person
/// picked, or `None` if they kept what was in force.
///
/// `behind` is the session already on screen. Drawn first each frame so Escape leaves the person
/// looking at the same session they opened the picker from.
pub fn choose<B: Backend>(
    terminal: &mut Terminal<B>,
    current: Option<Effort>,
    mut behind: impl FnMut(&mut Frame),
) -> Option<Row> {
    let mut picker = Picker::new(current);

    loop {
        if terminal
            .draw(|frame| {
                behind(frame);
                draw(frame, &picker);
            })
            .is_err()
        {
            return None;
        }

        let Ok(event) = event::read() else {
            return None;
        };
        let TermEvent::Key(key) = event else {
            continue;
        };
        if key.kind != event::KeyEventKind::Press {
            continue;
        }

        match handle_key(&mut picker, key.code, key.modifiers) {
            Outcome::Continue => continue,
            Outcome::Cancel => return None,
            Outcome::Select => return picker.chosen(),
        }
    }
}

fn draw(frame: &mut Frame, picker: &Picker) {
    let area = centred(frame.area(), picker.rows.len());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::brand_primary()))
        .title(format!(" {} ", t!(effort_picker_title)))
        .style(Style::default().bg(theme::background()).fg(theme::text()));
    let inside = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // list
            Constraint::Length(1), // what the row under the cursor is for
            Constraint::Length(1), // keys
        ])
        .split(inside);

    frame.render_widget(Paragraph::new(list_lines(picker, layout[0])), layout[0]);

    // Every row has a hint, so this line never empties and the list below it does not shift as the
    // cursor moves. Indented like the key line rather than like a name, so a sentence under the
    // list does not read as another row to pick.
    let hint = picker.chosen().map(Row::hint).unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {hint}"),
            Style::default()
                .fg(theme::muted())
                .add_modifier(Modifier::ITALIC),
        ))),
        layout[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", t!(effort_picker_keys)),
            Style::default().fg(theme::muted()),
        ))),
        layout[2],
    );
}

/// A centred panel sized to the list, never larger than the terminal.
fn centred(area: Rect, rows: usize) -> Rect {
    let available = area.width.saturating_sub(2);
    let width = available.min(52).max(24.min(available));
    // Borders, hint line, key line, and one row per level that fits.
    let list = (rows as u16)
        .saturating_add(4)
        .min(area.height.saturating_sub(2));
    let height = list.max(6).min(area.height.saturating_sub(2));

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn list_lines(picker: &Picker, area: Rect) -> Vec<Line<'static>> {
    let visible = (area.height as usize).max(1);
    let first = picker.selected.saturating_sub(visible.saturating_sub(1));

    let mut lines = Vec::new();
    for (index, row) in picker.rows.iter().enumerate().skip(first).take(visible) {
        let chosen = index == picker.selected;
        let marker = if chosen { "❯ " } else { "  " };
        let name = if chosen {
            Style::default()
                .fg(theme::brand_primary())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::text())
        };

        let mut spans = vec![
            Span::styled(marker, Style::default().fg(theme::brand_primary())),
            Span::styled(row.name(), name),
        ];

        if picker.is_current(*row) {
            spans.push(Span::styled(
                format!("  {}", t!(picker_current)),
                Style::default().fg(theme::ok()),
            ));
        }

        lines.push(Line::from(spans));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn the_picker_opens_on_the_level_in_use() {
        let picker = Picker::new(Some(Effort::Xhigh));
        assert_eq!(picker.chosen(), Some(Row(Some(Effort::Xhigh))));
    }

    /// Somebody who has never chosen opens on the row that asks for no level, which is what their
    /// requests are already doing.
    #[test]
    fn having_chosen_nothing_opens_on_the_row_that_asks_for_none() {
        let picker = Picker::new(None);
        assert_eq!(picker.chosen(), Some(Row(None)));
    }

    /// A first pick must not be permanent: the row that asks for no level is how somebody gets
    /// back to whatever the service does on its own.
    #[test]
    fn asking_for_no_level_is_one_of_the_rows() {
        let picker = Picker::new(Some(Effort::Max));
        assert!(picker.rows.contains(&Row(None)));
    }

    #[test]
    fn the_arrows_walk_the_list_and_stop_at_its_ends() {
        let mut picker = Picker::new(None);
        handle_key(&mut picker, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(picker.chosen(), Some(Row(None)));

        for _ in 0..100 {
            handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        }
        assert_eq!(picker.chosen(), Some(Row(Some(Effort::Max))));
    }

    #[test]
    fn enter_selects_and_escape_keeps_the_current_level() {
        let mut picker = Picker::new(None);
        assert_eq!(
            handle_key(&mut picker, KeyCode::Enter, KeyModifiers::NONE),
            Outcome::Select
        );
        assert_eq!(
            handle_key(&mut picker, KeyCode::Esc, KeyModifiers::NONE),
            Outcome::Cancel
        );
    }

    #[test]
    fn ctrl_c_leaves_the_level_alone() {
        let mut picker = Picker::new(None);
        assert_eq!(
            handle_key(&mut picker, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Outcome::Cancel
        );
    }

    fn rendered(picker: &Picker) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, picker))
            .expect("draw succeeds");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The words drawn are the words sent and stored, so somebody comparing what they picked
    /// against what a service documents is reading the same word.
    #[test]
    fn the_list_shows_the_words_a_request_carries() {
        let output = rendered(&Picker::new(None));
        for level in Effort::ALL {
            assert!(output.contains(level.as_str()), "{output}");
        }
    }

    #[test]
    fn the_level_in_use_is_marked() {
        let output = rendered(&Picker::new(Some(Effort::High)));
        assert!(output.contains("high  current"), "{output}");
    }

    /// Every row says what it is for. A list of five ranked words says nothing about which one to
    /// pick, and the whole point of the panel is that somebody can decide.
    #[test]
    fn the_row_under_the_cursor_says_what_it_is_for() {
        let output = rendered(&Picker::new(Some(Effort::Max)));
        assert!(output.contains("the most thinking, cost aside"), "{output}");
    }

    #[test]
    fn the_picker_is_drawn_as_a_centred_panel() {
        let output = rendered(&Picker::new(None));
        assert!(output.contains("effort"), "{output}");
        assert!(
            output.contains('╭') && output.contains('╮'),
            "no rounded border: {output}"
        );
    }

    #[test]
    fn the_panel_stays_inside_a_tiny_terminal() {
        let picker = Picker::new(None);
        let mut terminal = Terminal::new(TestBackend::new(30, 8)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &picker))
            .expect("draw succeeds");
        let area = centred(Rect::new(0, 0, 30, 8), picker.rows.len());
        assert!(area.width <= 30);
        assert!(area.height <= 8);
        assert!(area.x + area.width <= 30);
        assert!(area.y + area.height <= 8);
    }
}
