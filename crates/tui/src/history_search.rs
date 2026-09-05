//! Searching the prompts already sent.
//!
//! Opened with Ctrl-R, the chord every shell answers with the same question. Up walks the history
//! one prompt at a time, which is the right way in when the thing wanted is the last one and no
//! way in at all when it is the hundredth: what a person remembers of an old prompt is a word out
//! of the middle of it, not how far back it was.
//!
//! Drawn over the transcript in place of the box, newest at the bottom, so the row a search starts
//! on is the one nearest where the eye already is and the list grows upwards into space the
//! transcript can spare.
//!
//! Nothing here sends anything. Enter puts the prompt in the box, where the person reads it and
//! presses Enter themselves, which is what makes it theirs: a history file can be edited, on a
//! shared machine by somebody else, so a stored line is content until a keystroke adopts it.

use crate::history::Entry;
use crate::state::Session;
use crate::theme;
use crate::wrap::{self, display_width};
use bravebot_i18n::t;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

/// The most prompts shown at once.
///
/// A window rather than the whole history, because the list is drawn over the transcript and the
/// transcript is what a person is searching in the middle of reading. Enough rows to recognise a
/// prompt among its neighbours, which is what the list is for; the rest is what typing is for.
const MOST_ROWS: usize = 8;

/// The width below which the full prompt is not shown beside the list.
///
/// Under this the two columns are narrow enough that neither says anything: the list would be cut
/// mid-word and the preview would be a column of single words.
const ROOM_FOR_A_PREVIEW: u16 = 80;

/// How wide the age column is, in columns.
const AGE_WIDTH: usize = 8;

/// What is being searched for, and where the cursor is in the answer.
#[derive(Debug, Default)]
pub struct Search {
    /// What has been typed to narrow the list.
    needle: String,
    /// How far up from the newest match the cursor is. Zero is the newest.
    ///
    /// Counted from the newest rather than as an index, because the newest is where the list is
    /// read from and typing changes how many matches there are below it.
    up: usize,
    /// Whether the list is narrowed to the workspace this session is running in.
    here: bool,
}

impl Search {
    /// Open, looking for `needle`.
    ///
    /// Seeded with whatever was in the box, since somebody who typed half a prompt and then
    /// reached for the history was already saying what they were looking for.
    pub fn looking_for(needle: impl Into<String>) -> Self {
        Self {
            needle: needle.into(),
            up: 0,
            here: false,
        }
    }

    pub fn needle(&self) -> &str {
        &self.needle
    }

    /// Whether the list is narrowed to this workspace.
    pub fn here(&self) -> bool {
        self.here
    }

    /// Add a character, and go back to the newest match.
    ///
    /// The cursor moves rather than staying where it was: a narrowed list is a different list, and
    /// an index into the old one points at a prompt the person never selected.
    pub fn typed(&mut self, c: char) {
        self.needle.push(c);
        self.up = 0;
    }

    /// Remove the last character. Reports whether there was one, so the caller can decide what
    /// backspacing past the start means.
    pub fn backspace(&mut self) -> bool {
        self.up = 0;
        self.needle.pop().is_some()
    }

    /// Move to an older match, stopping at the oldest.
    pub fn older(&mut self, matches: usize) {
        self.up = (self.up + 1).min(matches.saturating_sub(1));
    }

    /// Move to a newer match, stopping at the newest.
    pub fn newer(&mut self) {
        self.up = self.up.saturating_sub(1);
    }

    /// Swap between every prompt and this workspace's, from the newest match either way.
    pub fn scope(&mut self) {
        self.here = !self.here;
        self.up = 0;
    }

    /// The prompts this search answers with, oldest first.
    ///
    /// `project` is the workspace the session runs in, and is what the narrowed scope compares
    /// against. An entry recorded before workspaces were kept belongs to none, so it is in the
    /// wide list only.
    pub fn matching<'a>(&self, entries: &'a [Entry], project: Option<&str>) -> Vec<&'a Entry> {
        let terms: Vec<String> = self
            .needle
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        entries
            .iter()
            .filter(|entry| {
                !self.here || entry.project.as_deref() == project.filter(|p| !p.is_empty())
            })
            .filter(|entry| {
                let haystack = entry.prompt.to_lowercase();
                terms.iter().all(|term| haystack.contains(term))
            })
            .collect()
    }

    /// Which of `matching` the cursor is on, as an index from the oldest.
    pub fn at(&self, matches: usize) -> Option<usize> {
        matches
            .checked_sub(1)?
            .checked_sub(self.up.min(matches - 1))
    }
}

/// How tall the panel is over a frame of `area`, given what it has to show.
///
/// Sized to the list, so a search with three answers is three rows rather than a box of blanks,
/// and capped so the transcript keeps something even on a short terminal.
pub fn height(session: &Session, area: Rect) -> u16 {
    let matches = session.history_matches().len();
    // A floor where the full prompt is drawn beside the list, since a box two rows tall is a
    // border with nothing in it: the rows a short list does not need are what that column shows
    // the prompt in.
    let least = match area.width >= ROOM_FOR_A_PREVIEW && matches > 0 {
        true => 5,
        false => 1,
    };
    let rows = matches.clamp(least, MOST_ROWS) as u16;
    // The title, the filter box and its border, and the key line.
    (rows + 5).min(area.height.saturating_sub(2).max(1))
}

/// Draw the panel.
pub fn draw(frame: &mut Frame, area: Rect, session: &Session) {
    let Some(search) = session.history_search() else {
        return;
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::background()).fg(theme::text())),
        area,
    );

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title and scope
            Constraint::Min(1),    // the matches, and the one under the cursor in full
            Constraint::Length(3), // what is being typed
            Constraint::Length(1), // keys
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {}", t!(history_search_title)),
                Style::default()
                    .fg(theme::brand_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", Style::default().fg(theme::muted())),
            Span::styled(
                match search.here() {
                    true => t!(history_scope_here),
                    false => t!(history_scope_everywhere),
                },
                Style::default().fg(theme::accent()),
            ),
        ])),
        layout[0],
    );

    draw_matches(frame, layout[1], session, search);
    draw_needle(frame, layout[2], search);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", t!(history_search_keys)),
            Style::default().fg(theme::muted()),
        ))),
        layout[3],
    );
}

/// The list, and beside it the prompt under the cursor in full.
fn draw_matches(frame: &mut Frame, area: Rect, session: &Session, search: &Search) {
    let matching = session.history_matches();
    if matching.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {}", t!(history_search_nothing_matches)),
                Style::default().fg(theme::muted()),
            ))),
            area,
        );
        return;
    }

    // The full prompt beside the list rather than under it: a row is one line of what may be a
    // paragraph, and the difference between two prompts that begin alike is further in than a row
    // has room for.
    let (list, preview) = match area.width >= ROOM_FOR_A_PREVIEW {
        true => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                // A column of space between the two, so a prompt cut at the edge of the list does
                // not read as running into the border beside it.
                .constraints([
                    Constraint::Percentage(55),
                    Constraint::Length(2),
                    Constraint::Min(20),
                ])
                .split(area);
            (split[0], Some(split[2]))
        }
        false => (area, None),
    };

    let at = search.at(matching.len()).unwrap_or(0);
    let visible = (list.height as usize).max(1);
    // The newest is at the bottom, so the window is taken from the end and slides up as the cursor
    // walks back through older prompts.
    let last = (at + 1)
        .max(matching.len().min(visible))
        .min(matching.len());
    let first = last.saturating_sub(visible);

    let mut lines = Vec::new();
    for (index, entry) in matching.iter().enumerate().take(last).skip(first) {
        let chosen = index == at;
        // Said on the first and last rows drawn rather than in a scrollbar, because the question a
        // person has of a window is whether the thing they want is outside it.
        let marker = match (
            chosen,
            index == first && first > 0,
            index + 1 == last && last < matching.len(),
        ) {
            (true, ..) => "❯",
            (_, true, _) => "↑",
            (_, _, true) => "↓",
            _ => " ",
        };
        let words = match chosen {
            true => Style::default()
                .fg(theme::brand_primary())
                .add_modifier(Modifier::BOLD),
            false => Style::default().fg(theme::text()),
        };

        let age = format!("{:>width$}", age_of(entry), width = AGE_WIDTH);
        let room = (list.width as usize).saturating_sub(AGE_WIDTH + 4);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker} "),
                Style::default().fg(theme::brand_primary()),
            ),
            Span::styled(age, Style::default().fg(theme::muted())),
            Span::raw("  "),
            Span::styled(clipped(entry.opening(), room), words),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), list);

    if let (Some(preview), Some(entry)) = (preview, matching.get(at)) {
        draw_preview(frame, preview, entry);
    }
}

/// The prompt under the cursor, wrapped, with a word for what did not fit.
fn draw_preview(frame: &mut Frame, area: Rect, entry: &Entry) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::muted()));
    let inside = block.inner(area);
    frame.render_widget(block, area);

    let rows = wrap::wrap(&entry.prompt, inside.width.max(1) as usize, 0).rows;
    let room = inside.height as usize;
    let mut lines: Vec<Line> = rows
        .iter()
        .take(room.saturating_sub(usize::from(rows.len() > room)))
        .map(|row| {
            Line::from(Span::styled(
                row.clone(),
                Style::default().fg(theme::text()),
            ))
        })
        .collect();
    if rows.len() > room {
        lines.push(Line::from(Span::styled(
            t!(history_search_more_lines, count = rows.len() - lines.len()),
            Style::default().fg(theme::muted()),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inside);
}

/// The box the search is typed into.
fn draw_needle(frame: &mut Frame, area: Rect, search: &Search) {
    let typed = match search.needle().is_empty() {
        true => Span::styled(
            t!(history_search_placeholder),
            Style::default().fg(theme::muted()),
        ),
        false => Span::styled(
            search.needle().to_string(),
            Style::default().fg(theme::text()),
        ),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ⌕ ", Style::default().fg(theme::muted())),
            typed,
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::brand_primary())),
        ),
        area,
    );

    // The terminal's own cursor, so what is being typed into is the thing that blinks.
    let caret = area.x + 4 + display_width(search.needle()) as u16;
    frame.set_cursor_position(Position::new(
        caret.min(area.right().saturating_sub(2)),
        area.y + 1,
    ));
}

/// How long ago a prompt was sent, in a column's worth of characters.
///
/// Compact rather than the phrasing the session list uses: this sits in a gutter beside every row
/// rather than in a sentence, and "13 minutes ago" beside each of eight rows is a column wider
/// than some of the prompts it is labelling.
///
/// Blank for a prompt stored before times were kept, since a made-up age is worse than none.
fn age_of(entry: &Entry) -> String {
    let Some(at) = entry.at else {
        return String::new();
    };
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
        .saturating_sub(at);

    match seconds {
        0..=59 => t!(history_age_now).to_string(),
        60..=3_599 => t!(history_age_minutes, count = seconds / 60),
        3_600..=86_399 => t!(history_age_hours, count = seconds / 3_600),
        86_400..=2_591_999 => t!(history_age_days, count = seconds / 86_400),
        _ => t!(history_age_months, count = seconds / 2_592_000),
    }
}

/// A prompt cut to the room there is for it, with an ellipsis where it was cut.
fn clipped(line: &str, room: usize) -> String {
    if display_width(line) <= room {
        return line.to_string();
    }
    let mut kept = String::new();
    for c in line.chars() {
        if display_width(&kept) + 2 > room {
            break;
        }
        kept.push(c);
    }
    kept.push('…');
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const HERE: &str = "/work/here";
    const ELSEWHERE: &str = "/work/elsewhere";

    /// Seconds ago, as a stamp.
    fn ago(seconds: u64) -> Option<u64> {
        Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(0)
                .saturating_sub(seconds),
        )
    }

    fn entry(prompt: &str, project: Option<&str>, seconds: u64) -> Entry {
        Entry {
            prompt: prompt.to_string(),
            at: ago(seconds),
            project: project.map(str::to_string),
        }
    }

    /// A session whose history is `entries`, running in [`HERE`], with the search open.
    fn searching(entries: Vec<Entry>) -> Session {
        let mut session = Session::new("none").in_workspace(HERE);
        session.history = History::from_entries(entries);
        assert!(session.open_history_search(), "the search did not open");
        session
    }

    fn sent() -> Vec<Entry> {
        vec![
            entry("why is the picker slow?", Some(HERE), 3 * 3_600),
            entry("run the tests", Some(ELSEWHERE), 25 * 60),
            entry("PICKER: add a search box", Some(HERE), 8 * 60),
            entry("commit that", Some(HERE), 30),
        ]
    }

    fn typing(session: &mut Session, text: &str) {
        for c in text.chars() {
            session.type_into_history_search(c);
        }
    }

    fn shown(session: &Session) -> Vec<String> {
        session
            .history_matches()
            .iter()
            .map(|entry| entry.prompt.clone())
            .collect()
    }

    /// The newest prompt is the one a search is nearly always about, and it is the row nearest the
    /// box the person just pressed the key in.
    #[test]
    fn the_search_opens_on_the_newest_prompt() {
        let session = searching(sent());
        assert_eq!(
            session.history_match().expect("a prompt").prompt,
            "commit that"
        );
    }

    #[test]
    fn typing_narrows_the_list_to_the_prompts_that_match() {
        let mut session = searching(sent());
        typing(&mut session, "picker");
        assert_eq!(
            shown(&session),
            ["why is the picker slow?", "PICKER: add a search box"],
            "case decided which prompts matched"
        );
    }

    /// A second word narrows rather than widening: two words a person remembers are two things
    /// they are sure of, and an entry holding one of them is not what they asked for.
    #[test]
    fn every_word_typed_has_to_match() {
        let mut session = searching(sent());
        typing(&mut session, "picker search");
        assert_eq!(shown(&session), ["PICKER: add a search box"]);
    }

    /// A narrowed list is a different list, and the row an index pointed into it is a prompt
    /// nobody chose. The newest match is where a person is looking after they type.
    #[test]
    fn narrowing_puts_the_cursor_on_the_newest_match() {
        let mut session = searching(sent());
        session.history_search_older();
        session.history_search_older();
        assert_eq!(
            session.history_match().expect("a prompt").prompt,
            "run the tests"
        );

        typing(&mut session, "picker");
        assert_eq!(
            session.history_match().expect("a prompt").prompt,
            "PICKER: add a search box"
        );
    }

    /// Walking off either end would look like the key had stopped working, so the cursor stays.
    #[test]
    fn the_cursor_stops_at_the_oldest_and_at_the_newest() {
        let mut session = searching(sent());
        for _ in 0..10 {
            session.history_search_older();
        }
        assert_eq!(
            session.history_match().expect("a prompt").prompt,
            "why is the picker slow?"
        );

        for _ in 0..10 {
            session.history_search_newer();
        }
        assert_eq!(
            session.history_match().expect("a prompt").prompt,
            "commit that"
        );
    }

    /// History is one file for every checkout, which is what makes it worth having and what makes
    /// it noisy: most of what is in it was asked somewhere else.
    #[test]
    fn the_scope_narrows_to_the_prompts_sent_from_this_workspace() {
        let mut session = searching(sent());
        session.scope_history_search();
        assert_eq!(
            shown(&session),
            [
                "why is the picker slow?",
                "PICKER: add a search box",
                "commit that"
            ],
            "a prompt from another checkout was offered as this project's"
        );

        session.scope_history_search();
        assert_eq!(shown(&session).len(), 4, "the wide list did not come back");
    }

    /// A prompt stored before workspaces were kept belongs to no project. Guessing one would file
    /// somebody's whole history under whichever checkout they happened to open next.
    #[test]
    fn a_prompt_from_before_workspaces_were_kept_is_in_the_wide_list_only() {
        let mut session = searching(vec![Entry::recalled("from an older history")]);
        assert_eq!(shown(&session), ["from an older history"]);

        session.scope_history_search();
        assert!(shown(&session).is_empty());
    }

    /// Backspacing widens it again, one character at a time.
    #[test]
    fn backspacing_widens_the_list() {
        let mut session = searching(sent());
        typing(&mut session, "picker");
        assert_eq!(shown(&session).len(), 2);

        for _ in 0..6 {
            assert!(session.backspace_history_search());
        }
        assert_eq!(shown(&session).len(), 4);
        assert!(
            !session.backspace_history_search(),
            "an empty search reported something to delete"
        );
    }

    fn rendered_at(session: &Session, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, frame.area(), session))
            .expect("draw succeeds");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn rendered(session: &Session) -> String {
        rendered_at(session, 100, 14)
    }

    /// Two prompts that begin alike are told apart by when they were sent, which is the one thing
    /// about an old prompt a person is sure of.
    #[test]
    fn an_age_is_drawn_beside_every_prompt_that_has_one() {
        let session = searching(sent());
        let output = rendered(&session);
        assert!(
            output.contains(&t!(history_age_hours, count = 3)),
            "{output}"
        );
        assert!(
            output.contains(&t!(history_age_minutes, count = 25)),
            "{output}"
        );
        assert!(output.contains(t!(history_age_now)), "{output}");
    }

    /// A made-up age is worse than none: it would say a prompt was sent in 1970, or that every old
    /// prompt was sent at whatever moment the file was last rewritten.
    #[test]
    fn a_prompt_with_no_age_is_drawn_without_one() {
        let output = rendered(&searching(vec![Entry::recalled("from an older history")]));
        assert!(output.contains("from an older history"), "{output}");
        assert!(!output.contains("ago"), "{output}");
    }

    /// A row is one line of what may be a paragraph, and the difference between two prompts that
    /// begin alike is further in than a row has room for.
    #[test]
    fn the_prompt_under_the_cursor_is_drawn_in_full() {
        let session = searching(vec![entry(
            "rewrite the picker\nand keep the sections\nand the cursor",
            Some(HERE),
            60,
        )]);
        let output = rendered(&session);
        assert!(output.contains("and the cursor"), "{output}");
    }

    /// The panel is a few rows over a transcript, so a pasted essay cannot be shown whole. What is
    /// left is said rather than cut off silently, since the part not shown is what a person is
    /// deciding about.
    #[test]
    fn a_prompt_too_long_for_the_panel_says_how_much_is_left() {
        let essay = (0..40)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output = rendered(&searching(vec![entry(&essay, Some(HERE), 60)]));
        assert!(output.contains('…') && output.contains('+'), "{output}");
    }

    /// Two columns on a narrow terminal are two columns too narrow to read. The list is the half
    /// that answers the question, so it is the half that is kept.
    #[test]
    fn a_narrow_panel_keeps_the_list_and_drops_the_full_prompt() {
        let session = searching(vec![entry("commit that\nand push it", Some(HERE), 60)]);
        assert!(
            rendered_at(&session, 100, 14).contains("and push it"),
            "the full prompt was not drawn where there was room for it"
        );

        let narrow = rendered_at(&session, 60, 14);
        assert!(narrow.contains("commit that"), "{narrow}");
        assert!(
            !narrow.contains("and push it"),
            "the full prompt was drawn into a column too narrow to read: {narrow}"
        );
    }

    /// A window over a long history has to say that it is one, or a person concludes their prompt
    /// is gone and retypes it.
    #[test]
    fn a_list_taller_than_the_panel_says_there_is_more_above() {
        // Oldest first, the order history is kept in, so `prompt 39` is the newest.
        let many: Vec<Entry> = (0..40)
            .map(|n| entry(&format!("prompt {n}"), Some(HERE), 60 * (40 - n)))
            .collect();
        let output = rendered(&searching(many));
        assert!(
            output.contains("prompt 39"),
            "the newest is not shown: {output}"
        );
        assert!(!output.contains("prompt 0 "), "{output}");
        assert!(
            output.contains('↑'),
            "nothing said the list went on: {output}"
        );
    }

    /// Nothing on screen and no word for it reads as a mode that has broken rather than a search
    /// that is too narrow.
    #[test]
    fn a_search_matching_nothing_says_so() {
        let mut session = searching(sent());
        typing(&mut session, "nothing named this");
        let output = rendered(&session);
        assert!(
            output.contains(t!(history_search_nothing_matches)),
            "{output}"
        );
        assert!(session.history_match().is_none());
    }

    /// Which prompts are on offer is half of what the list means, and the scope that decides it is
    /// not something a person can see any other way.
    #[test]
    fn the_scope_is_named_on_the_panel() {
        let mut session = searching(sent());
        assert!(rendered(&session).contains(t!(history_scope_everywhere)));

        session.scope_history_search();
        assert!(rendered(&session).contains(t!(history_scope_here)));
    }
}
