//! Choosing how the input box edits text.
//!
//! Shown by `/config`. One question so far, which is whether the box edits the way vi does, and the
//! choice is written to `~/.bravebot`: it outlives the session that made it and applies in every
//! directory, the same as the model, the theme and the effort level.
//!
//! Drawn as a centred panel over the session, so the transcript stays visible. Enter takes the row
//! under the cursor. Escape leaves the style that was in force alone.
//!
//! Nothing labelled is involved. The words are drawn for a person, who picks one, and what their pick
//! changes is which keys move a caret in a box they are typing into.
//!
//! # Why the panel exists for one question
//!
//! Because the alternative is a command per preference, and this is the second of them: `/theme` and
//! `/effort` each took a word of the command space for one choice. A panel is where the third and the
//! fourth go without another verb being invented, and it is the surface somebody coming from Claude
//! Code already looks for.

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
use crate::vim::Editing;

/// One row of the list: a style of editing to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row(pub Editing);

impl Row {
    /// Every row, the ordinary box first, which is what most people are already using.
    fn all() -> Vec<Row> {
        Editing::ALL.into_iter().map(Row).collect()
    }

    /// The word shown for this row.
    ///
    /// The word the setting is spelled with, untranslated: it appears in a settings file and in the
    /// documentation of the tools this borrows the name from, so somebody comparing what they picked
    /// against what they wrote in a file is comparing the same word.
    fn name(self) -> &'static str {
        self.0.as_str()
    }

    /// What this row means, in one line under the list.
    ///
    /// A match rather than a lookup, so every sentence is named in the source: nothing computed while
    /// running can choose which of them a person is shown.
    fn hint(self) -> &'static str {
        match self.0 {
            Editing::Ordinary => t!(config_editing_hint_ordinary),
            Editing::Vi => t!(config_editing_hint_vi),
        }
    }
}

/// What the picker is showing and where the cursor is.
#[derive(Debug)]
pub struct Picker {
    rows: Vec<Row>,
    /// Which row is under the cursor.
    selected: usize,
    /// The style in force when the picker opened, marked in the list.
    previous: Row,
}

impl Picker {
    /// Open on the style in use, since that is the row a person is looking for.
    pub fn new(current: Editing) -> Self {
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
    /// Leave the style that was in force alone.
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
    current: Editing,
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
        .title(format!(" {} ", t!(config_picker_title)))
        .style(Style::default().bg(theme::background()).fg(theme::text()));
    let inside = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // list
            Constraint::Length(1), // what the row under the cursor means
            Constraint::Length(1), // keys
        ])
        .split(inside);

    frame.render_widget(Paragraph::new(list_lines(picker, layout[0])), layout[0]);

    // Every row has a hint, so this line never empties and the list above it does not shift as the
    // cursor moves.
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
            format!(" {}", t!(config_picker_keys)),
            Style::default().fg(theme::muted()),
        ))),
        layout[2],
    );
}

/// A centred panel sized to the list, never larger than the terminal.
fn centred(area: Rect, rows: usize) -> Rect {
    let available = area.width.saturating_sub(2);
    let width = available.min(52).max(24.min(available));
    // Borders, hint line, key line, and one row per style that fits.
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

    /// The picker opens on the style in force, since that is the row somebody is looking for: opening
    /// on the first row would make the list say the ordinary box was chosen when it was not.
    #[test]
    fn the_picker_opens_on_the_style_in_force() {
        let picker = Picker::new(Editing::Vi);
        assert_eq!(picker.chosen(), Some(Row(Editing::Vi)));
    }

    /// The row in force is marked, so the list says which one it is rather than only where the cursor
    /// happens to be. Those are two different facts and the cursor moves.
    #[test]
    fn the_style_in_force_is_marked_wherever_the_cursor_is() {
        let mut picker = Picker::new(Editing::Ordinary);
        picker.down();

        assert_eq!(picker.chosen(), Some(Row(Editing::Vi)));
        assert!(picker.is_current(Row(Editing::Ordinary)));
        assert!(!picker.is_current(Row(Editing::Vi)));
    }

    /// The cursor stops at both ends rather than wrapping, so a person holding a key does not find
    /// themselves back where they started with no way to tell.
    #[test]
    fn the_cursor_stops_at_the_ends_of_the_list() {
        let mut picker = Picker::new(Editing::Ordinary);
        picker.up();
        assert_eq!(picker.chosen(), Some(Row(Editing::Ordinary)));

        for _ in 0..5 {
            picker.down();
        }
        assert_eq!(picker.chosen(), Some(Row(Editing::Vi)));
    }

    /// Escape and Ctrl-C leave the style alone, which is what makes the panel safe to open: somebody
    /// looking to see what is in force has a way out that changes nothing.
    #[test]
    fn escape_and_ctrl_c_leave_the_style_alone() {
        let mut picker = Picker::new(Editing::Ordinary);
        assert_eq!(
            handle_key(&mut picker, KeyCode::Esc, KeyModifiers::NONE),
            Outcome::Cancel
        );
        assert_eq!(
            handle_key(&mut picker, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Outcome::Cancel
        );
    }

    /// The keys that walk a list here are the arrows and vi's own, since the list is a list whichever
    /// style the box is in and somebody who chose vi editing will reach for `j` and `k`.
    #[test]
    fn the_arrows_and_vis_own_keys_walk_the_list() {
        for down in [KeyCode::Down, KeyCode::Char('j')] {
            let mut picker = Picker::new(Editing::Ordinary);
            assert_eq!(
                handle_key(&mut picker, down, KeyModifiers::NONE),
                Outcome::Continue
            );
            assert_eq!(picker.chosen(), Some(Row(Editing::Vi)));
        }
        for up in [KeyCode::Up, KeyCode::Char('k')] {
            let mut picker = Picker::new(Editing::Vi);
            handle_key(&mut picker, up, KeyModifiers::NONE);
            assert_eq!(picker.chosen(), Some(Row(Editing::Ordinary)));
        }
    }

    /// Enter takes the row under the cursor, which is the whole of what the panel is for.
    #[test]
    fn enter_takes_the_row_under_the_cursor() {
        let mut picker = Picker::new(Editing::Ordinary);
        picker.down();
        assert_eq!(
            handle_key(&mut picker, KeyCode::Enter, KeyModifiers::NONE),
            Outcome::Select
        );
        assert_eq!(picker.chosen(), Some(Row(Editing::Vi)));
    }

    /// Every row says what it means, so the panel is legible to somebody who does not already know
    /// what `vim` names here. A row with nothing under it would be a word and a guess.
    #[test]
    fn every_row_says_what_it_means() {
        for row in Row::all() {
            assert!(!row.hint().is_empty(), "{} has no hint", row.name());
        }
    }

    fn rendered(picker: &Picker) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).expect("terminal");
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

    /// The words drawn are the words the setting is spelled with, so somebody comparing what they
    /// picked against what they wrote in a file is reading the same word.
    #[test]
    fn the_list_shows_the_words_the_setting_is_spelled_with() {
        let output = rendered(&Picker::new(Editing::Ordinary));
        for style in Editing::ALL {
            assert!(output.contains(style.as_str()), "{output}");
        }
    }

    /// The style in force is marked on the screen and not only in the state, since the panel is where
    /// somebody goes to find out which one it is.
    #[test]
    fn the_style_in_force_is_drawn_as_current() {
        let output = rendered(&Picker::new(Editing::Vi));
        assert!(output.contains("vim  current"), "{output}");
    }

    /// The panel says what its keys are, so a person who opened it has a way out they can read rather
    /// than one they have to guess at.
    #[test]
    fn the_panel_says_which_keys_it_answers() {
        let output = rendered(&Picker::new(Editing::Ordinary));
        assert!(output.contains(t!(config_picker_keys)), "{output}");
        assert!(output.contains(t!(config_picker_title)), "{output}");
    }
}
