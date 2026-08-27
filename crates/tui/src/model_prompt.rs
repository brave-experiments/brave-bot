//! Choosing which model to think with.
//!
//! Shown by `/model`. The list comes from the endpoint rather than from a set compiled in, so it
//! is whatever the backend actually offers today, and the choice is written to `~/.bua`: it
//! outlives the session that made it and applies in every directory.
//!
//! Nothing labelled is involved and nothing is quarantined. The names never reach a model: they
//! are drawn for a person, who picks one, and what they picked becomes the `model` field of later
//! requests. That field is routing, and a person choosing it off a list they read is the
//! endorsement for it, the same way a person approving a write destination is.

use bua_aichat::models::Model;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// What the picker is showing and where the cursor is.
#[derive(Debug)]
pub struct Picker {
    /// Everything on offer, `automatic` first.
    models: Vec<Model>,
    /// Which one is under the cursor.
    selected: usize,
    /// The one in use when the picker opened, so it can be marked as current.
    current: Option<String>,
}

impl Picker {
    /// Open on the model in use, since that is the row a user is looking for.
    pub fn new(models: Vec<Model>, current: Option<&str>) -> Self {
        let selected = current
            .and_then(|name| models.iter().position(|model| model.key == name))
            .unwrap_or(0);
        Self {
            models,
            selected,
            current: current.map(str::to_string),
        }
    }

    /// The model under the cursor.
    pub fn chosen(&self) -> Option<&Model> {
        self.models.get(self.selected)
    }

    fn down(&mut self) {
        let last = self.models.len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Whether this row is the model already in use.
    ///
    /// Nothing chosen is `automatic` in use, since that is what a request with no choice sends.
    fn is_current(&self, model: &Model) -> bool {
        match &self.current {
            Some(name) => model.key == *name,
            None => model.is_automatic(),
        }
    }
}

/// What a key press did to the picker.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still choosing.
    Continue,
    /// Use the model under the cursor.
    Select,
    /// Leave the model as it was.
    Cancel,
}

/// Interpret one key press.
///
/// Separated from the loop so it can be tested without a terminal.
pub fn handle_key(picker: &mut Picker, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
    if modifiers.contains(KeyModifiers::CONTROL) {
        // Raw mode delivers the interrupt as a key. Someone pressing it wants out of the picker,
        // and leaving the model alone is the way out that changes nothing.
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

/// Show the list and return the model the user picked, or `None` if they kept the current one.
pub fn choose<B: Backend>(
    terminal: &mut Terminal<B>,
    models: Vec<Model>,
    current: Option<&str>,
) -> Option<Model> {
    let mut picker = Picker::new(models, current);

    loop {
        // A terminal that cannot be drawn to cannot carry the question, so nothing is changed.
        if terminal.draw(|frame| draw(frame, &picker)).is_err() {
            return None;
        }

        let Ok(event) = event::read() else {
            return None;
        };
        let TermEvent::Key(key) = event else {
            continue;
        };
        // A key event arrives twice on Windows, once pressed and once released, and the release
        // would otherwise choose whatever the press had just moved to.
        if key.kind != event::KeyEventKind::Press {
            continue;
        }

        match handle_key(&mut picker, key.code, key.modifiers) {
            Outcome::Continue => continue,
            Outcome::Cancel => return None,
            Outcome::Select => return picker.chosen().cloned(),
        }
    }
}

/// Draw the list.
fn draw(frame: &mut Frame, picker: &Picker) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // heading
            Constraint::Length(1), // blank
            Constraint::Min(1),    // list
            Constraint::Length(1), // keys
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Select model",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))),
        layout[0],
    );

    frame.render_widget(Paragraph::new(list_lines(picker, layout[2])), layout[2]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  ↑↓ to choose  ·  Enter to select  ·  Esc to keep the current one",
            Style::default().fg(Color::DarkGray),
        ))),
        layout[3],
    );
}

/// The list itself, one line per model.
fn list_lines(picker: &Picker, area: Rect) -> Vec<Line<'static>> {
    // The window is what fits rather than what exists, so a long list does not draw past the area.
    let visible = (area.height as usize).max(1);
    let first = picker.selected.saturating_sub(visible.saturating_sub(1));

    let mut lines = Vec::new();
    for (index, model) in picker.models.iter().enumerate().skip(first).take(visible) {
        let chosen = index == picker.selected;
        let marker = if chosen { "❯ " } else { "  " };
        let name = if chosen {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let mut spans = vec![
            Span::styled(marker, Style::default().fg(Color::Cyan)),
            Span::styled(model.display_name.clone(), name),
        ];

        // Said rather than left to be discovered by a request failing: without a subscription a
        // premium model returns 403 and the session looks broken for no stated reason.
        if model.premium {
            spans.push(Span::styled(
                "  premium",
                Style::default().fg(Color::Yellow),
            ));
        }
        if picker.is_current(model) {
            spans.push(Span::styled("  current", Style::default().fg(Color::Green)));
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

    fn model(key: &str, display: &str, premium: bool) -> Model {
        Model {
            key: key.to_string(),
            display_name: display.to_string(),
            premium,
        }
    }

    fn offered() -> Vec<Model> {
        vec![
            Model::automatic(),
            model("claude-3-sonnet", "Claude 4 Sonnet", true),
            model("llama-3-8b-instruct", "Llama 3 8B", false),
        ]
    }

    /// The row a user is looking for is the one already in use, so the cursor starts there rather
    /// than at the top of a list they then have to search.
    #[test]
    fn the_picker_opens_on_the_model_in_use() {
        let picker = Picker::new(offered(), Some("llama-3-8b-instruct"));
        assert_eq!(picker.chosen().expect("a model").key, "llama-3-8b-instruct");
    }

    /// Nothing chosen means the request sends "automatic", so that is what is in use.
    #[test]
    fn with_no_choice_the_picker_opens_on_automatic() {
        let picker = Picker::new(offered(), None);
        assert!(picker.chosen().expect("a model").is_automatic());
    }

    /// A model that has stopped being offered must not leave the cursor pointing past the end.
    #[test]
    fn a_current_model_no_longer_offered_opens_at_the_top() {
        let picker = Picker::new(offered(), Some("withdrawn-model"));
        assert!(picker.chosen().expect("a model").is_automatic());
    }

    #[test]
    fn the_arrows_walk_the_list_and_stop_at_its_ends() {
        let mut picker = Picker::new(offered(), None);
        handle_key(&mut picker, KeyCode::Up, KeyModifiers::NONE);
        assert!(
            picker.chosen().expect("a model").is_automatic(),
            "walked past the top"
        );

        for _ in 0..5 {
            handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        }
        assert_eq!(
            picker.chosen().expect("a model").key,
            "llama-3-8b-instruct",
            "walked past the bottom"
        );
    }

    #[test]
    fn enter_selects_and_escape_keeps_the_current_model() {
        let mut picker = Picker::new(offered(), None);
        assert_eq!(
            handle_key(&mut picker, KeyCode::Enter, KeyModifiers::NONE),
            Outcome::Select
        );
        assert_eq!(
            handle_key(&mut picker, KeyCode::Esc, KeyModifiers::NONE),
            Outcome::Cancel
        );
    }

    /// Ctrl-C is what a user reaches for to get out of anything, and leaving the model alone is the
    /// way out that changes nothing.
    #[test]
    fn ctrl_c_leaves_the_model_alone() {
        let mut picker = Picker::new(offered(), None);
        assert_eq!(
            handle_key(&mut picker, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Outcome::Cancel
        );
    }

    fn rendered(picker: &Picker) -> String {
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("terminal");
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

    /// A premium model returns 403 without a subscription, so which rows those are has to be on the
    /// screen rather than discovered by a request failing.
    #[test]
    fn a_premium_model_says_so() {
        let output = rendered(&Picker::new(offered(), None));
        assert!(output.contains("Claude 4 Sonnet  premium"), "{output}");
        assert!(!output.contains("Llama 3 8B  premium"), "{output}");
    }

    /// Which one is in use answers the question a user opened the picker to ask.
    #[test]
    fn the_model_in_use_is_marked() {
        let output = rendered(&Picker::new(offered(), Some("claude-3-sonnet")));
        assert!(
            output.contains("Claude 4 Sonnet  premium  current"),
            "{output}"
        );
    }

    /// The list is drawn from the display names, never the keys: a key is for a request field and
    /// says nothing to a person choosing.
    #[test]
    fn the_list_shows_names_a_person_reads() {
        let output = rendered(&Picker::new(offered(), None));
        assert!(output.contains("Automatic"), "{output}");
        assert!(output.contains("Llama 3 8B"), "{output}");
    }
}
