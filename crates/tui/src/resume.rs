//! Choosing a session to pick up again.
//!
//! Shown by `bravebot --resume`, before anything else happens. The list is this working directory's
//! sessions, newest first, because the one being looked for is nearly always the last one.
//!
//! Typing filters rather than jumping, since a title is remembered as a few words out of the
//! middle of it rather than as the way it starts. Escape leaves without resuming anything, which
//! starts an ordinary session: nothing here can strand a user who opened it by mistake.

use crate::sessions::{self, Summary};
use crate::theme;
use bravebot_i18n::t;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use std::path::Path;

/// What the picker is showing and where the cursor is.
#[derive(Debug)]
pub struct Picker {
    /// Every session, newest first.
    sessions: Vec<Summary>,
    /// What has been typed to narrow it.
    search: String,
    /// Which of the matching sessions is under the cursor.
    selected: usize,
    /// The project these sessions belong to, for the heading.
    project: String,
}

impl Picker {
    pub fn new(sessions: Vec<Summary>, project: impl Into<String>) -> Self {
        Self {
            sessions,
            search: String::new(),
            selected: 0,
            project: project.into(),
        }
    }

    /// The sessions matching what has been typed, in order.
    ///
    /// Matched without regard to case and anywhere in the title, because a session is remembered
    /// by a word out of the middle of what was asked.
    pub fn matching(&self) -> Vec<&Summary> {
        let needle = self.search.to_lowercase();
        self.sessions
            .iter()
            .filter(|session| session.title.to_lowercase().contains(&needle))
            .collect()
    }

    /// The session under the cursor, if the list is not empty.
    pub fn chosen(&self) -> Option<&Summary> {
        self.matching().get(self.selected).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    fn down(&mut self) {
        let last = self.matching().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Narrow the list, keeping the cursor inside it.
    fn typed(&mut self, c: char) {
        self.search.push(c);
        self.clamp();
    }

    fn backspace(&mut self) {
        self.search.pop();
        self.clamp();
    }

    /// A search that no longer matches what was selected must not leave the cursor past the end.
    fn clamp(&mut self) {
        let last = self.matching().len().saturating_sub(1);
        self.selected = self.selected.min(last);
    }
}

/// What a key press did to the picker.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still choosing.
    Continue,
    /// Resume the session under the cursor.
    Resume,
    /// Leave without resuming anything, and start an ordinary session.
    Cancel,
    /// Leave without starting anything at all.
    Quit,
}

/// Interpret one key press.
///
/// Separated from the loop so it can be tested without a terminal.
pub fn handle_key(picker: &mut Picker, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
    if modifiers.contains(KeyModifiers::CONTROL) {
        return match code {
            // Raw mode delivers the interrupt as a key rather than as a signal, and someone
            // pressing it wants out of the program, not a different session.
            KeyCode::Char('c') => Outcome::Quit,
            _ => Outcome::Continue,
        };
    }

    match code {
        KeyCode::Esc => Outcome::Cancel,
        KeyCode::Enter => Outcome::Resume,
        KeyCode::Up => {
            picker.up();
            Outcome::Continue
        }
        KeyCode::Down => {
            picker.down();
            Outcome::Continue
        }
        KeyCode::Backspace => {
            picker.backspace();
            Outcome::Continue
        }
        KeyCode::Char(c) => {
            picker.typed(c);
            Outcome::Continue
        }
        _ => Outcome::Continue,
    }
}

/// What the picker settled on.
#[derive(Debug)]
pub enum Choice {
    /// Pick this session up.
    Resume(Box<sessions::Record>),
    /// Start an ordinary session: there was nothing to resume, or the user decided not to.
    /// Neither is a failure.
    Fresh,
    /// Start nothing. The user asked to leave.
    Quit,
}

/// Show the list and return what to do.
pub fn choose<B: Backend>(terminal: &mut Terminal<B>, project: &Path) -> Choice {
    let mut picker = Picker::new(sessions::list(project), project.display().to_string());
    if picker.is_empty() {
        return Choice::Fresh;
    }

    loop {
        if terminal.draw(|frame| draw(frame, &picker)).is_err() {
            return Choice::Fresh;
        }

        let Ok(event) = event::read() else {
            return Choice::Fresh;
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
            Outcome::Cancel => return Choice::Fresh,
            Outcome::Quit => return Choice::Quit,
            Outcome::Resume => {
                let Some(chosen) = picker.chosen() else {
                    continue;
                };
                let id = chosen.id.clone();
                return match sessions::load(project, &id) {
                    Some(record) => Choice::Resume(Box::new(record)),
                    None => Choice::Fresh,
                };
            }
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
            Constraint::Length(3), // search box
            Constraint::Length(1), // project
            Constraint::Min(1),    // list
            Constraint::Length(1), // keys
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {}", t!(resume_heading)),
            Style::default()
                .fg(theme::brand_primary())
                .add_modifier(Modifier::BOLD),
        ))),
        layout[0],
    );

    let search = if picker.search.is_empty() {
        Span::styled(
            t!(resume_search_placeholder),
            Style::default().fg(theme::muted()),
        )
    } else {
        Span::raw(picker.search.clone())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::raw(" "), search])).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::muted())),
        ),
        layout[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {}", picker.project),
            Style::default().fg(theme::muted()),
        ))),
        layout[2],
    );

    frame.render_widget(Paragraph::new(list_lines(picker, layout[3])), layout[3]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {}", t!(resume_keys)),
            Style::default().fg(theme::muted()),
        ))),
        layout[4],
    );
}

/// The list itself: two lines per session, and a word when nothing matches.
fn list_lines(picker: &Picker, area: Rect) -> Vec<Line<'static>> {
    let matching = picker.matching();
    if matching.is_empty() {
        return vec![Line::from(Span::styled(
            format!("  {}", t!(resume_nothing_matches)),
            Style::default().fg(theme::muted()),
        ))];
    }

    // Three rows per entry, so the window is what fits rather than what exists.
    let visible = (area.height as usize / 3).max(1);
    let first = picker.selected.saturating_sub(visible.saturating_sub(1));

    let mut lines = Vec::new();
    for (index, session) in matching.iter().enumerate().skip(first).take(visible) {
        let chosen = index == picker.selected;
        let marker = if chosen { "❯ " } else { "  " };
        let title = if chosen {
            Style::default()
                .fg(theme::brand_primary())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(theme::brand_primary())),
            Span::styled(session.title.clone(), title),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", describe(session)),
            Style::default().fg(theme::muted()),
        )));
        lines.push(Line::raw(""));
    }
    lines
}

/// The second line of an entry: when, where and how much.
fn describe(session: &Summary) -> String {
    let mut parts = vec![sessions::how_long_ago(session.updated)];
    if let Some(branch) = &session.branch {
        parts.push(branch.clone());
    }
    parts.push(sessions::size(session.bytes));
    parts.join("  ·  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, title: &str, updated: u64) -> Summary {
        Summary {
            id: id.to_string(),
            title: title.to_string(),
            branch: Some("main".to_string()),
            updated,
            bytes: 1024,
        }
    }

    fn picker() -> Picker {
        Picker::new(
            vec![
                summary("1", "Session recovery after laptop sleep", 300),
                summary("2", "User experience progress updates", 200),
                summary("3", "Launch the TUI", 100),
            ],
            "/work/bravebot",
        )
    }

    #[test]
    fn the_newest_session_is_the_one_under_the_cursor() {
        assert_eq!(picker().chosen().expect("a session").id, "1");
    }

    #[test]
    fn the_arrows_walk_the_list_and_stop_at_its_ends() {
        let mut picker = picker();
        handle_key(&mut picker, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(
            picker.chosen().expect("a session").id,
            "1",
            "walked past the top"
        );

        for _ in 0..5 {
            handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        }
        assert_eq!(
            picker.chosen().expect("a session").id,
            "3",
            "walked past the bottom"
        );
    }

    /// A session is remembered by a word out of the middle of what was asked, not by how the
    /// title starts.
    #[test]
    fn typing_narrows_the_list_by_any_part_of_a_title() {
        let mut picker = picker();
        for c in "PROGRESS".chars() {
            handle_key(&mut picker, KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(picker.matching().len(), 1);
        assert_eq!(picker.chosen().expect("a session").id, "2");
    }

    /// Narrowing the list under a cursor that was further down must not leave it pointing past
    /// the end, which would be a picker that resumes nothing when Enter is pressed.
    #[test]
    fn narrowing_the_list_keeps_the_cursor_inside_it() {
        let mut picker = picker();
        handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        for c in "recovery".chars() {
            handle_key(&mut picker, KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(picker.chosen().expect("a session").id, "1");
    }

    #[test]
    fn backspace_widens_it_again() {
        let mut picker = picker();
        for c in "zzz".chars() {
            handle_key(&mut picker, KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert!(picker.matching().is_empty());
        assert!(
            picker.chosen().is_none(),
            "a session was chosen from nothing"
        );

        for _ in 0..3 {
            handle_key(&mut picker, KeyCode::Backspace, KeyModifiers::NONE);
        }
        assert_eq!(picker.matching().len(), 3);
    }

    #[test]
    fn escape_leaves_without_resuming_anything() {
        let mut picker = picker();
        assert_eq!(
            handle_key(&mut picker, KeyCode::Esc, KeyModifiers::NONE),
            Outcome::Cancel
        );
        assert_eq!(
            handle_key(&mut picker, KeyCode::Enter, KeyModifiers::NONE),
            Outcome::Resume
        );
    }

    /// Ctrl-C is the key a user reaches for to get out of anything, and it must not be typed
    /// into the search box as a letter. Escape declines to resume, which leaves a session
    /// running; Ctrl-C asks for the program to end, so the two are not the same answer.
    #[test]
    fn ctrl_c_quits_rather_than_typing_a_letter() {
        let mut picker = picker();
        assert_eq!(
            handle_key(&mut picker, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Outcome::Quit
        );
        assert!(picker.search.is_empty());
    }

    #[test]
    fn an_entry_says_when_where_and_how_much() {
        let line = describe(&summary("1", "anything", sessions_now() - 120));
        assert!(line.contains("2 minutes ago"), "{line}");
        assert!(line.contains("main"), "{line}");
        assert!(line.contains("1.0KB"), "{line}");
    }

    fn sessions_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_secs()
    }
}
