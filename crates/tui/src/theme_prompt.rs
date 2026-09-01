//! Choosing which theme paints the interface.
//!
//! Shown by `/theme`. The list is the built-in set plus JSON files from `~/.bravebot/themes`, and
//! the choice is written to `~/.bravebot`: it outlives the session that made it and applies in
//! every directory.
//!
//! Drawn as a centred panel over the session, so the transcript stays visible and a live preview
//! repaints what is behind the box. Moving the cursor live-previews. Enter keeps the preview and
//! persists the name. Escape puts back the theme that was in force when the picker opened.
//!
//! Nothing labelled is involved. The names never reach a model: they are drawn for a person, who
//! picks one.

use bravebot_i18n::t;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::theme::{self, Theme};

/// What the picker is showing and where the cursor is.
#[derive(Debug)]
pub struct Picker {
    /// Everything on offer, `brave` first among the built-ins.
    themes: Vec<Theme>,
    /// Which one is under the cursor.
    selected: usize,
    /// The theme in force when the picker opened, restored on cancel.
    previous: Theme,
}

impl Picker {
    /// Open on the theme in use, since that is the row a user is looking for.
    pub fn new(themes: Vec<Theme>, current: &str) -> Self {
        let previous = themes
            .iter()
            .find(|theme| theme.name == current)
            .cloned()
            .unwrap_or_else(|| Theme {
                name: theme::BRAVE.to_string(),
                palette: theme::brave_palette(false),
            });
        let selected = themes
            .iter()
            .position(|theme| theme.name == current)
            .unwrap_or(0);
        Self {
            themes,
            selected,
            previous,
        }
    }

    /// The theme under the cursor.
    pub fn chosen(&self) -> Option<&Theme> {
        self.themes.get(self.selected)
    }

    fn down(&mut self) {
        let last = self.themes.len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn is_current(&self, theme: &Theme) -> bool {
        theme.name == self.previous.name
    }

    /// Put the theme under the cursor in force so the list itself shows what it would look like.
    fn preview(&self) {
        if let Some(theme) = self.chosen() {
            theme::apply(theme);
        }
    }

    fn restore(&self) {
        theme::apply(&self.previous);
    }
}

/// What a key press did to the picker.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still choosing; the preview may have changed.
    Continue,
    /// Keep the theme under the cursor.
    Select,
    /// Put back the theme that was in force when the picker opened.
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

/// Show the list as a centred panel over whatever `behind` draws, and return the theme the user
/// picked, or `None` if they kept the current one.
///
/// `behind` is the session (or anything else already on screen). Drawn first each frame so a live
/// preview repaints the transcript in the theme under the cursor, and so Escape leaves the person
/// looking at the same session they opened the picker from.
pub fn choose<B: Backend>(
    terminal: &mut Terminal<B>,
    themes: Vec<Theme>,
    current: &str,
    mut behind: impl FnMut(&mut Frame),
) -> Option<Theme> {
    let mut picker = Picker::new(themes, current);
    picker.preview();

    loop {
        if terminal
            .draw(|frame| {
                behind(frame);
                draw(frame, &picker);
            })
            .is_err()
        {
            picker.restore();
            return None;
        }

        let Ok(event) = event::read() else {
            picker.restore();
            return None;
        };
        let TermEvent::Key(key) = event else {
            continue;
        };
        if key.kind != event::KeyEventKind::Press {
            continue;
        }

        match handle_key(&mut picker, key.code, key.modifiers) {
            Outcome::Continue => {
                picker.preview();
                continue;
            }
            Outcome::Cancel => {
                picker.restore();
                return None;
            }
            Outcome::Select => {
                let chosen = picker.chosen().cloned();
                if let Some(ref theme) = chosen {
                    theme::apply(theme);
                } else {
                    picker.restore();
                }
                return chosen;
            }
        }
    }
}

fn draw(frame: &mut Frame, picker: &Picker) {
    let area = centred(frame.area(), picker.themes.len());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::brand_primary()))
        .title(format!(" {} ", t!(theme_picker_title)))
        .style(Style::default().bg(theme::background()).fg(theme::text()));
    let inside = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // list
            Constraint::Length(1), // hint for the theme under the cursor
            Constraint::Length(1), // keys
        ])
        .split(inside);

    frame.render_widget(Paragraph::new(list_lines(picker, layout[0])), layout[0]);

    // Drawn empty rather than skipped when the cursor is on a theme with nothing to add, so the
    // list does not shift under the cursor as it moves. Indented like the key line rather than like
    // a name, so a sentence under the list does not read as another theme to pick.
    let hint = picker.chosen().and_then(|theme| theme::hint(&theme.name));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint.map(|hint| format!(" {hint}")).unwrap_or_default(),
            Style::default()
                .fg(theme::muted())
                .add_modifier(Modifier::ITALIC),
        ))),
        layout[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", t!(theme_picker_keys)),
            Style::default().fg(theme::muted()),
        ))),
        layout[2],
    );
}

/// A centred panel sized to the theme list, never larger than the terminal.
///
/// Narrower than the confirm box: a name list does not need the width a diff does, and OpenCode's
/// theme dialog is a compact select rather than a full-bleed page.
fn centred(area: Rect, theme_count: usize) -> Rect {
    let available = area.width.saturating_sub(2);
    let width = available.min(42).max(24.min(available));
    // Borders, hint line, key line, and one row per theme that fits.
    let list = (theme_count as u16)
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
    for (index, theme) in picker.themes.iter().enumerate().skip(first).take(visible) {
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
            Span::styled(theme.name.clone(), name),
        ];

        if picker.is_current(theme) {
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

    fn offered() -> Vec<Theme> {
        theme::builtins()
    }

    #[test]
    fn the_picker_opens_on_the_theme_in_use() {
        let picker = Picker::new(offered(), "nord");
        assert_eq!(picker.chosen().expect("a theme").name, "nord");
    }

    #[test]
    fn a_current_theme_no_longer_offered_opens_at_the_top() {
        let picker = Picker::new(offered(), "withdrawn-theme");
        assert_eq!(picker.chosen().expect("a theme").name, theme::BRAVE);
    }

    #[test]
    fn the_arrows_walk_the_list_and_stop_at_its_ends() {
        let mut picker = Picker::new(offered(), theme::BRAVE);
        handle_key(&mut picker, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(picker.chosen().expect("a theme").name, theme::BRAVE);

        for _ in 0..100 {
            handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        }
        assert_eq!(
            picker.chosen().expect("a theme").name,
            offered().last().expect("non-empty").name
        );
    }

    #[test]
    fn enter_selects_and_escape_keeps_the_current_theme() {
        let mut picker = Picker::new(offered(), theme::BRAVE);
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
    fn ctrl_c_leaves_the_theme_alone() {
        let mut picker = Picker::new(offered(), theme::BRAVE);
        assert_eq!(
            handle_key(&mut picker, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Outcome::Cancel
        );
    }

    #[test]
    fn preview_puts_the_cursor_theme_in_force_and_cancel_restores() {
        theme::apply_brave();
        let mut picker = Picker::new(offered(), theme::BRAVE);
        handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        picker.preview();
        assert_ne!(theme::name(), theme::BRAVE);
        picker.restore();
        assert_eq!(theme::name(), theme::BRAVE);
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

    #[test]
    fn the_theme_in_use_is_marked() {
        let output = rendered(&Picker::new(offered(), "nord"));
        assert!(output.contains("nord  current"), "{output}");
    }

    #[test]
    fn the_list_shows_names_a_person_reads() {
        let themes = offered();
        let output = rendered(&Picker::new(themes.clone(), theme::BRAVE));
        assert!(output.contains(&themes[0].name), "{output}");
        // The panel on 80x24 shows the start of the list; names near the end sit below the fold.
        assert!(output.contains(&themes[1].name), "{output}");
    }

    /// `brave` is the only theme whose inks depend on the terminal, and its name does not say so.
    #[test]
    fn the_terminal_following_theme_says_so_under_the_list() {
        let output = rendered(&Picker::new(offered(), theme::BRAVE));
        assert!(output.contains("follows your terminal"), "{output}");
    }

    #[test]
    fn a_theme_that_paints_every_ink_itself_has_nothing_to_add() {
        let output = rendered(&Picker::new(offered(), "nord"));
        assert!(!output.contains("follows your terminal"), "{output}");
    }

    /// The picker is a panel over the session, not a full-screen takeover: the title sits in a
    /// rounded border the confirm prompts already use.
    #[test]
    fn the_picker_is_drawn_as_a_centred_panel() {
        let output = rendered(&Picker::new(offered(), theme::BRAVE));
        assert!(output.contains("themes"), "{output}");
        // Corners of a rounded border, so a full-bleed list would not pass.
        assert!(
            output.contains('╭') && output.contains('╮'),
            "no rounded border: {output}"
        );
    }

    #[test]
    fn the_panel_stays_inside_a_tiny_terminal() {
        let picker = Picker::new(offered(), theme::BRAVE);
        let mut terminal = Terminal::new(TestBackend::new(30, 8)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &picker))
            .expect("draw succeeds");
        let area = centred(Rect::new(0, 0, 30, 8), picker.themes.len());
        assert!(area.width <= 30);
        assert!(area.height <= 8);
        assert!(area.x + area.width <= 30);
        assert!(area.y + area.height <= 8);
    }
}
