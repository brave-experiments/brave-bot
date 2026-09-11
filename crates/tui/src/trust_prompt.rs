//! Asking, at startup, whether to trust the working directory.
//!
//! The answer decides how the session behaves. Trusting the directory means work inside it
//! proceeds without a prompt for every write, because reads from it return trusted data.
//! Declining means everything is untrusted, so every write is shown, which is the correct
//! behaviour for a directory whose contents came from somewhere else.
//!
//! Nothing is trusted by default. An unreadable terminal, an unexpected key, or a lost event
//! stream all resolve to declining, because the failure mode of guessing wrong here is that a
//! session silently writes to files nobody vouched for.
//!
//! The one session that is not asked is the one bypassing every permission, which answers this
//! question along with the rest. [`answered_by`] is where that is decided.
//!
//! A settings file may name directories to open beside the working directory, and a name in one is
//! a request rather than a grant: each is put as its own question and only an accepted one is
//! opened. A file arriving with a checkout is the easiest thing on the machine to write to, so a
//! name in one deciding what is reachable and trusted would be reach granted by whatever last
//! edited it.

use bravebot_agent::PermissionMode;
use bravebot_core::trust::TrustStore;
use bravebot_i18n::t;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use std::path::Path;

use crate::theme;

/// What the user decided about the working directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Trust,
    Decline,
    /// Leave without starting a session at all.
    Leave,
}

/// Ask about `directory`, returning the trust map the session should start with.
///
/// Trusting records the workspace root, which covers everything beneath it. Declining records
/// nothing, leaving an empty map in which no path is trusted.
///
/// Asked afresh every time a session begins. The answer is standing permission for as long as
/// that session lasts and no longer: it is not written down anywhere a later launch will read it,
/// so nothing this grants can be inherited by a session whose user was never asked. A resumed
/// session is the one exception, and it inherits the answer its own user gave rather than
/// skipping the question, which is why this is not called at all in that case.
///
/// `None` is the third answer: the user pressed Ctrl-C, which is neither trusting nor declining
/// but a request to leave, so no session begins at all.
pub fn ask<B: Backend>(terminal: &mut Terminal<B>, directory: &Path) -> Option<TrustStore> {
    let answer = match terminal.draw(|frame| draw(frame, directory)) {
        Ok(_) => read_answer(),
        // A terminal that cannot be drawn to cannot carry the question.
        Err(_) => Answer::Decline,
    };

    trust_for(answer)
}

/// Ask about each directory a settings file named, returning the ones to open.
///
/// One question per directory, because each grants reach and trust over a tree of its own and one
/// key standing in for all of them would be a grant nobody read. What an answer of yes grants is
/// what `/add-dir` grants, so a directory a file named and a directory a person typed are open on
/// identical terms.
///
/// `None` is the request to leave, for the reason it is at the working directory's own question: a
/// session that began behind it is one nobody agreed to have.
pub fn ask_named<B: Backend>(
    terminal: &mut Terminal<B>,
    directories: &[String],
) -> Option<Vec<String>> {
    accepted(directories, |directory| {
        match terminal.draw(|frame| draw_named(frame, directory)) {
            Ok(_) => read_answer(),
            // A terminal that cannot be drawn to cannot carry the question.
            Err(_) => Answer::Decline,
        }
    })
}

/// Which of the directories to open, given how the question about each was answered.
///
/// Separated from the terminal so the decision can be tested without one, the way [`trust_for`]
/// is. Leaving opens none of them, including any accepted before it: a person on their way out is
/// not somebody who granted something.
fn accepted(directories: &[String], mut answer: impl FnMut(&str) -> Answer) -> Option<Vec<String>> {
    let mut opening = Vec::new();
    for directory in directories {
        match answer(directory) {
            Answer::Trust => opening.push(directory.clone()),
            Answer::Decline => continue,
            Answer::Leave => return None,
        }
    }
    Some(opening)
}

/// The map for a session whose mode has already answered, or `None` where a person must answer.
///
/// Bypassing answers this question along with every other one, and answers it yes. That mode
/// already approves vouching for each quarantined file the planner asks to read, so putting the
/// question would be asking for a grant the session is about to make anyway, one file at a time,
/// and a modal box before the first prompt is the most conspicuous thing there is to put to a
/// person who asked to be asked about nothing.
///
/// Separated from the loop so it can be tested without a terminal.
pub fn answered_by(mode: PermissionMode) -> Option<TrustStore> {
    match mode {
        PermissionMode::Bypass => Some(trusting_the_workspace()),
        PermissionMode::Ask | PermissionMode::AcceptEdits | PermissionMode::Plan => None,
    }
}

/// The map an answer starts the session with, or `None` for leaving.
fn trust_for(answer: Answer) -> Option<TrustStore> {
    match answer {
        Answer::Leave => None,
        Answer::Trust => Some(trusting_the_workspace()),
        Answer::Decline => Some(TrustStore::new()),
    }
}

/// The rule trusting the workspace records: the root, which covers everything beneath it.
///
/// One place, so the map reached without the question is the map a yes would have written.
fn trusting_the_workspace() -> TrustStore {
    let mut trust = TrustStore::new();
    trust.trust(".");
    trust
}

/// Block until the user answers.
fn read_answer() -> Answer {
    loop {
        match event::read() {
            // Presses only: the interface asks for disambiguated keys, so a release arrives too,
            // and answering a question twice grants standing permission on one keystroke.
            Ok(TermEvent::Key(key)) if key.kind != event::KeyEventKind::Press => continue,
            Ok(TermEvent::Key(key)) => match answer_for(key) {
                Some(answer) => return answer,
                None => continue,
            },
            Ok(_) => continue,
            Err(_) => return Answer::Decline,
        }
    }
}

/// Interpret one key press, or `None` for a key that answers nothing.
///
/// Separated from the loop so it can be tested without a terminal.
fn answer_for(key: KeyEvent) -> Option<Answer> {
    // Raw mode delivers Ctrl-C as a key rather than as a signal, so a prompt that ignored it
    // would be a screen with no way out: the interrupt everyone reaches for would do nothing.
    // It is not an answer to the question, so it starts nothing rather than declining.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(Answer::Leave),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('y' | 'Y') => Some(Answer::Trust),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(Answer::Decline),
        // Enter is deliberately not a yes: it is the key most likely to be pressed
        // out of habit, and this question grants standing permission.
        _ => None,
    }
}

/// Draw the question about the working directory.
fn draw(frame: &mut ratatui::Frame, directory: &Path) {
    let lines = vec![
        asking(
            t!(trust_directory_question),
            &directory.display().to_string(),
        ),
        Line::raw(""),
        // One line each, wrapped by the paragraph rather than broken here: a translation does
        // not break where the English did, and a sentence split into two spans cannot be rewrapped.
        Line::from(Span::raw(t!(trust_directory_explained))),
        Line::raw(""),
        Line::from(Span::styled(
            t!(trust_directory_regardless),
            Style::default().fg(theme::muted()),
        )),
        Line::raw(""),
        keys(t!(trust_directory_yes), t!(trust_directory_no)),
    ];

    panel(frame, t!(trust_directory_title), lines);
}

/// Draw the question about one directory a settings file named.
fn draw_named(frame: &mut ratatui::Frame, directory: &str) {
    let lines = vec![
        asking(t!(named_directory_question), directory),
        Line::raw(""),
        Line::from(Span::raw(t!(named_directory_explained))),
        Line::raw(""),
        Line::from(Span::styled(
            t!(named_directory_regardless),
            Style::default().fg(theme::muted()),
        )),
        Line::raw(""),
        keys(t!(named_directory_yes), t!(named_directory_no)),
    ];

    panel(frame, t!(named_directory_title), lines);
}

/// What is being asked, and the path it is being asked about.
///
/// The path is the whole of what an answer is about, so it is drawn rather than summarised.
fn asking(question: &str, directory: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{question} "),
            Style::default()
                .fg(theme::brand_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            directory.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("?"),
    ])
}

/// The answers on offer: the same two keys and the same way out at either question.
fn keys(yes: &str, no: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "  y",
            Style::default()
                .fg(theme::ok())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {yes}    ")),
        Span::styled(
            "n",
            Style::default()
                .fg(theme::fail())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {no}    ")),
        Span::styled(
            "ctrl-c",
            Style::default()
                .fg(theme::muted())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", t!(quit)),
            Style::default().fg(theme::muted()),
        ),
    ])
}

/// Draw one question, in a box whose every cell the theme paints.
///
/// One implementation for both questions here, because the chrome is what says the question is the
/// system's own: `Clear` empties the cells under the panel without colouring them, so the
/// background and text colour are set for the block rather than for the border alone.
fn panel(frame: &mut ratatui::Frame, title: &str, lines: Vec<Line<'static>>) {
    let area = centred(frame.area());
    frame.render_widget(Clear, area);

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme::brand_primary()))
                    .title(format!(" {title} "))
                    .style(Style::default().bg(theme::background()).fg(theme::text())),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// A centred box, sized to the terminal but never larger than it.
fn centred(area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(5),
            Constraint::Percentage(90),
            Constraint::Percentage(5),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// The characters one question puts on a terminal, in reading order.
    ///
    /// Takes the drawing rather than a path, so both questions are measured by one helper and an
    /// assertion about what a prompt says cannot be made of one and forgotten on the other.
    fn rendered(draw_it: impl FnOnce(&mut ratatui::Frame)) -> String {
        let mut terminal = Terminal::new(TestBackend::new(72, 20)).expect("terminal");
        terminal.draw(draw_it).expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// Every cell the border encloses carries the theme's own colours, for one question.
    ///
    /// `Clear` empties the cells under a panel without colouring them, so a prompt that styled
    /// only its border is a hole in the palette: themed border, terminal-default everything else.
    fn paints_the_themes_chrome(draw_it: impl FnOnce(&mut ratatui::Frame)) {
        let _held = theme::exclusive();
        let theme = theme::find("nord").expect("nord is built in");
        theme::apply(&theme);
        let painted = theme::background();

        let mut terminal = Terminal::new(TestBackend::new(72, 20)).expect("terminal");
        terminal.draw(draw_it).expect("draw");
        let buffer = terminal.backend().buffer().clone();
        theme::apply_brave();

        let inside = centred(*buffer.area());
        // Every cell the border encloses, including the rows the prose did not reach: an unpainted
        // row below the keys is the same hole as an unpainted one beside them.
        for y in 1..inside.height - 1 {
            for x in 1..inside.width - 1 {
                let cell = &buffer[(inside.x + x, inside.y + y)];
                assert_eq!(
                    cell.bg, painted,
                    "the cell at {x},{y} kept the terminal's own background"
                );
                assert_ne!(
                    cell.fg,
                    Color::Reset,
                    "the cell at {x},{y} kept the terminal's own text colour"
                );
            }
        }
    }

    fn names(directories: &[&str]) -> Vec<String> {
        directories.iter().map(|d| d.to_string()).collect()
    }

    /// The flag says nothing is put to the person, and a modal box before the first prompt is the
    /// most conspicuous thing there is to put. The map is the one a yes would have written, which
    /// is what the mode is about to grant anyway: it approves vouching for every quarantined file
    /// the planner reads, so the workspace ends up trusted a file at a time regardless.
    #[test]
    fn bypassing_trusts_the_workspace_instead_of_asking() {
        let trust = answered_by(PermissionMode::Bypass).expect("bypassing answers the question");

        assert!(trust.is_trusted("."));
        assert!(trust.is_trusted("src/main.rs"), "the rule covers the tree");
    }

    /// Every other mode answers fewer questions than this one, so none of them may answer the one
    /// that grants standing permission over a whole tree.
    #[test]
    fn every_other_mode_leaves_the_question_to_the_person() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
        ] {
            assert!(answered_by(mode).is_none(), "{mode:?} answered it");
        }
    }

    #[test]
    fn the_prompt_names_the_directory_and_both_answers() {
        let output = rendered(|frame| draw(frame, Path::new("/home/me/project")));
        assert!(output.contains("/home/me/project"));
        assert!(output.contains("trust it"));
        assert!(output.contains("every write"));
    }

    /// The question must say what saying yes actually does, since it grants standing
    /// permission rather than approving one action.
    #[test]
    fn the_prompt_explains_the_consequence() {
        let output = rendered(|frame| draw(frame, Path::new("/tmp/x")));
        assert!(output.contains("trusted"), "no mention of trust: {output}");
        // Wrapping can split a phrase across lines, so assert on a short fragment.
        assert!(
            output.contains("Say no if you"),
            "no guidance on when to decline: {output}"
        );
    }

    /// This is the first screen of a session and it grants standing permission over a whole tree,
    /// so its chrome is the first thing that says the question is the system's own.
    #[test]
    fn the_prompt_paints_the_themes_background_inside_its_border() {
        paints_the_themes_chrome(|frame| draw(frame, Path::new("/home/me/project")));
    }

    /// A settings file arrives with whatever produced the checkout, so a directory named in one is
    /// a request. Only the answer opens it, and a name nobody accepted leaves the path as
    /// unreachable and unvouched for as any other outside the workspace.
    #[test]
    fn a_directory_a_file_named_is_opened_only_where_the_person_accepts_it() {
        let named = names(&["/home/me/notes", "/home/me/.ssh", "/srv/shared"]);

        let accepted_one = accepted(&named, |directory| match directory {
            "/home/me/notes" => Answer::Trust,
            _ => Answer::Decline,
        })
        .expect("answering still starts a session");
        assert_eq!(accepted_one, names(&["/home/me/notes"]));

        let accepted_none =
            accepted(&named, |_| Answer::Decline).expect("declining still starts a session");
        assert!(
            accepted_none.is_empty(),
            "a declined name was opened anyway: {accepted_none:?}"
        );
    }

    /// Leaving is the answer to no question, so a name accepted before it is not a grant either:
    /// a session that opened a directory on the way out would be acting on an answer nobody
    /// finished giving.
    #[test]
    fn leaving_at_one_of_the_questions_opens_nothing() {
        let named = names(&["/home/me/notes", "/home/me/.ssh"]);

        let answered = accepted(&named, |directory| match directory {
            "/home/me/notes" => Answer::Trust,
            _ => Answer::Leave,
        });

        assert!(answered.is_none(), "leaving started a session anyway");
    }

    /// The path is the whole of what the answer is about, and it is the one thing a settings file
    /// chose rather than the person reading the box.
    #[test]
    fn the_named_prompt_shows_the_directory_it_would_open() {
        let output = rendered(|frame| draw_named(frame, "/home/me/.ssh"));
        assert!(output.contains("/home/me/.ssh"), "no path: {output}");
        assert!(output.contains("open it"));
        assert!(output.contains("leave it closed"));
    }

    /// Opening a directory grants two things at once, reach and trust, and neither is on the
    /// screen unless the question says so. The question also has to say where it came from: a box
    /// naming a directory the person has never typed is otherwise unexplained.
    #[test]
    fn the_named_prompt_explains_what_opening_does() {
        let output = rendered(|frame| draw_named(frame, "/srv/shared"));
        // Wrapping can split a phrase across lines, so assert on short fragments.
        assert!(
            output.contains("settings file"),
            "no mention of where the name came from: {output}"
        );
        assert!(output.contains("trusted"), "no mention of trust: {output}");
    }

    /// This question needs the theme's own chrome for the reason the working directory's does: the
    /// frame is what says the question is the system's and not something a file being read is
    /// asking.
    #[test]
    fn the_named_prompt_paints_the_themes_background_inside_its_border() {
        paints_the_themes_chrome(|frame| draw_named(frame, "/home/me/.ssh"));
    }

    /// Both questions, since either can be the first thing a session draws on a small terminal.
    #[test]
    fn a_tiny_terminal_still_renders() {
        let mut terminal = Terminal::new(TestBackend::new(24, 8)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, Path::new("/tmp/x")))
            .expect("must not panic on a small area");
        terminal
            .draw(|frame| draw_named(frame, "/tmp/x"))
            .expect("must not panic on a small area");
    }

    /// Trusting records the root, which covers the whole tree. Asked of the answer rather than of
    /// a store built here, for the reason [`declining_trusts_nothing`] is: a `trust_for` that
    /// returned an empty map on a yes would satisfy a test that trusted `.` itself, and every
    /// write would be shown to somebody who said yes.
    #[test]
    fn trusting_covers_the_whole_workspace() {
        let trust = trust_for(Answer::Trust).expect("trusting starts a session");
        assert!(trust.is_trusted("."));
        assert!(trust.is_trusted("src/main.rs"));
        assert!(trust.is_trusted("deep/nested/file.txt"));
    }

    /// The key that grants standing permission over a whole tree is the deliberate one and no
    /// other. Enter is the key most likely to be pressed out of habit, so it answers nothing: a
    /// keystroke made without reading must not be the answer that costs the most to get wrong.
    #[test]
    fn only_y_trusts_and_enter_answers_nothing() {
        let pressed = |code| KeyEvent::new(code, KeyModifiers::NONE);

        assert_eq!(answer_for(pressed(KeyCode::Char('y'))), Some(Answer::Trust));
        assert_eq!(answer_for(pressed(KeyCode::Char('Y'))), Some(Answer::Trust));
        assert_eq!(
            answer_for(pressed(KeyCode::Char('n'))),
            Some(Answer::Decline)
        );
        assert_eq!(
            answer_for(pressed(KeyCode::Char('N'))),
            Some(Answer::Decline)
        );
        assert_eq!(answer_for(pressed(KeyCode::Esc)), Some(Answer::Decline));
        assert_eq!(answer_for(pressed(KeyCode::Enter)), None);
    }

    /// Ctrl-C is the interrupt everyone reaches for, and raw mode turns it into an ordinary key
    /// press. A prompt that ignored it would be a screen with no way out.
    #[test]
    fn ctrl_c_leaves_rather_than_answering_the_question() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(answer_for(key), Some(Answer::Leave));
    }

    /// Leaving is not a quiet decline: a session that started anyway would be one the user
    /// never agreed to have.
    #[test]
    fn leaving_starts_no_session() {
        assert!(trust_for(Answer::Leave).is_none());
        assert!(trust_for(Answer::Decline).is_some());
    }

    /// A plain `c` is not an interrupt, and neither is any other control chord.
    #[test]
    fn only_ctrl_c_leaves() {
        assert_eq!(
            answer_for(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            answer_for(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            None
        );
    }

    /// Declining leaves nothing trusted, so every write is shown. Asked of the answer rather than
    /// of a store built here, which is what the clause is about: a `trust_for` that trusted `.`
    /// on a decline would have satisfied a test that only built its own empty store.
    #[test]
    fn declining_trusts_nothing() {
        let trust = trust_for(Answer::Decline).expect("declining still starts a session");
        assert!(trust.is_empty());
        assert!(!trust.is_trusted("src/main.rs"));
        assert!(!trust.is_trusted("."));
    }
}
