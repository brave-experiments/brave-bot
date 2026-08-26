//! Asking the user about a write, in the terminal.
//!
//! Turns run synchronously, so this can draw a prompt and block on a keypress from inside
//! the turn that requested the write. The alternative, collecting writes and asking
//! afterwards, would mean the model continuing on the assumption a write had happened.
//!
//! Nothing is approved by default. An unreadable terminal, an unexpected key, or a lost
//! event all resolve to refusal.

use bua_agent::confirm::{Confirmer, Decision, Intent, WriteRequest};
use bua_agent::diff::Change;
use bua_core::ask::{Answer as UserAnswer, Asking};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

/// Unchanged lines shown either side of a change, for orientation.
const CONTEXT_LINES: usize = 2;

/// Prompts in the terminal for each write.
pub struct TerminalConfirmer<'t, B: Backend> {
    terminal: &'t mut Terminal<B>,
}

impl<'t, B: Backend> TerminalConfirmer<'t, B> {
    pub fn new(terminal: &'t mut Terminal<B>) -> Self {
        Self { terminal }
    }
}

impl<B: Backend> Confirmer for TerminalConfirmer<'_, B> {
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        ask(self.terminal, request).decision()
    }

    fn ask_user(&mut self, asking: &Asking) -> Vec<UserAnswer> {
        crate::ask::ask(self.terminal, asking)
    }
}

/// What the user did with the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Approve,
    Reject,
    /// Refuse the write and stop the turn that asked for it.
    Interrupt,
}

impl Answer {
    /// What to tell the waiting turn. Interrupting refuses, since a turn being stopped is not
    /// consent to the write it was stopped at.
    pub fn decision(self) -> Decision {
        match self {
            Answer::Approve => Decision::Approve,
            Answer::Reject | Answer::Interrupt => Decision::Reject,
        }
    }
}

/// Draw the prompt and wait for an answer.
///
/// Standalone as well as available through [`TerminalConfirmer`], because a turn running on a
/// worker thread cannot hold the terminal: the main thread calls this on its behalf.
pub fn ask<B: Backend>(terminal: &mut Terminal<B>, request: &WriteRequest) -> Answer {
    let mut scroll = 0u16;
    loop {
        // A terminal that cannot be drawn to cannot carry a question, so refuse rather
        // than proceed unseen.
        // How far the body can scroll is only knowable once it has been laid out at the width
        // it will be drawn at, so it comes back out of the closure.
        let mut most = 0u16;
        if terminal
            .draw(|frame| most = draw(frame, request, scroll))
            .is_err()
        {
            return Answer::Reject;
        }

        match event::read() {
            Ok(TermEvent::Key(key)) => match answer_for(key) {
                Some(Response::Answer(answer)) => return answer,
                Some(Response::Scroll(by)) => {
                    scroll = scroll.saturating_add_signed(by).min(most);
                }
                None => continue,
            },
            Ok(_) => continue,
            // Losing the event stream must not approve anything.
            Err(_) => return Answer::Reject,
        }
    }
}

/// What a key press did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Response {
    Answer(Answer),
    /// Move the body by this many rows, positive being further down.
    Scroll(i16),
}

/// Interpret one key press, or `None` for a key that answers nothing.
///
/// Separated from the loop so it can be tested without a terminal.
fn answer_for(key: KeyEvent) -> Option<Response> {
    // The prompt blocks the whole interface, so without this Ctrl-C would do nothing at the one
    // moment a user is most likely to press it. It stops the turn as well as refusing, because
    // someone reaching for the interrupt wants the work to stop, not just this write.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(Response::Answer(Answer::Interrupt)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('y' | 'Y') => Some(Response::Answer(Answer::Approve)),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(Response::Answer(Answer::Reject)),
        // A diff longer than the box is the one most worth reading before answering.
        KeyCode::Up => Some(Response::Scroll(-1)),
        KeyCode::Down => Some(Response::Scroll(1)),
        KeyCode::PageUp => Some(Response::Scroll(-10)),
        KeyCode::PageDown => Some(Response::Scroll(10)),
        KeyCode::Home => Some(Response::Scroll(i16::MIN)),
        KeyCode::End => Some(Response::Scroll(i16::MAX)),
        // Enter is deliberately not an approval: it is the key most likely to
        // be pressed out of habit.
        _ => None,
    }
}

/// Draw the confirmation over the session, returning how far its body can be scrolled.
///
/// The keys are drawn as a row of their own rather than as the last line of the body. They used
/// to be the last line, kept on screen by capping the diff, and the cap counted lines while the
/// paragraph drew wrapped rows: a diff with long lines pushed the question off the bottom, so the
/// prompt asked nothing and the answer went to a screen that never showed what it was for.
fn draw(frame: &mut ratatui::Frame, request: &WriteRequest, scroll: u16) -> u16 {
    let area = centred(frame.area());
    frame.render_widget(Clear, area);

    let (verb, colour) = match request.intent {
        Intent::Create => ("Create", Color::Green),
        Intent::Overwrite => ("Overwrite", Color::Yellow),
        Intent::Edit => ("Edit", Color::Cyan),
    };

    let diff = request.diff();

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{verb} "),
                Style::default().fg(colour).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                request.path.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  +{} -{}", diff.added(), diff.removed()),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::raw(""),
    ];

    // A diff that could not be computed must say so. Rendering nothing would read as
    // "this write changes nothing", which is the opposite of the truth.
    if !diff.is_exact() {
        lines.push(Line::from(Span::styled(
            format!(
                "  the change is too large to show: {} lines replace {}",
                diff.added(),
                diff.removed()
            ),
            Style::default().fg(Color::Yellow),
        )));
    }

    // The same margin the transcript draws down everything the model was not allowed to read.
    // A body out of a quarantined file is that, and the person about to approve it is the only
    // one who will ever see it: they should be able to tell which kind of review this is.
    let marked = Style::default().fg(Color::Yellow);
    let margin = if request.untrusted { "┃ " } else { "  " };
    if request.untrusted {
        lines.push(Line::from(Span::styled(
            format!("{margin}untrusted: nobody has read this, and the model never saw it"),
            marked,
        )));
    }

    // All of it. What does not fit is scrolled to, rather than dropped: the hunks nobody shows
    // you are exactly the ones an approval is supposed to cover.
    let changes = diff.condensed(CONTEXT_LINES);
    for change in changes.iter() {
        let body = match change {
            Change::Added(text) => {
                Span::styled(format!("+{text}"), Style::default().fg(Color::Green))
            }
            Change::Removed(text) => {
                Span::styled(format!("-{text}"), Style::default().fg(Color::Red))
            }
            Change::Kept(text) => {
                Span::styled(format!(" {text}"), Style::default().fg(Color::DarkGray))
            }
            Change::Elided(count) => Span::styled(
                format!(" … {count} unchanged lines"),
                Style::default().fg(Color::DarkGray),
            ),
        };
        lines.push(Line::from(vec![
            Span::styled(margin.to_string(), marked),
            body,
        ]));
    }

    let keys = Line::from(vec![
        Span::styled(
            "  y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" write it    "),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" leave it alone    "),
        Span::styled(
            "ctrl-c",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" stop the turn", Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" approve this write? ");
    let inside = block.inner(area);
    frame.render_widget(block, area);

    // One row for the keys, the rest for the diff. Split before the body is laid out, so the
    // question keeps its row whatever the body turns out to be.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inside);

    let body = Paragraph::new(lines).wrap(Wrap { trim: false });
    // Rows, not lines: the paragraph wraps, and the difference between the two is what used to
    // push the question off the screen.
    let drawn = body.line_count(rows[0].width) as u16;
    let furthest = drawn.saturating_sub(rows[0].height);
    let offset = scroll.min(furthest);
    frame.render_widget(body.scroll((offset, 0)), rows[0]);

    let mut keys = keys;
    if furthest > 0 {
        let below = furthest - offset;
        keys.push_span(Span::styled(
            // Short, because the row is as wide as the box and the keys come first: a hint
            // that gets clipped in half tells the reviewer less than no hint at all.
            if below > 0 {
                format!("   ↑↓ {below} more")
            } else {
                "   ↑↓ back".to_string()
            },
            Style::default().fg(Color::Cyan),
        ));
    }
    frame.render_widget(Paragraph::new(keys), rows[1]);

    furthest
}

/// A centred box, sized to the terminal but never larger than it.
fn centred(area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
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

    fn request(contents: &str, existing: Option<&str>) -> WriteRequest {
        WriteRequest {
            path: "src/main.rs".into(),
            contents: contents.into(),
            intent: if existing.is_some() {
                Intent::Overwrite
            } else {
                Intent::Create
            },
            existing: existing.map(str::to_string),
            untrusted: false,
        }
    }

    fn rendered(request: &WriteRequest) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, request, 0);
            })
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The prompt blocks everything else, so Ctrl-C must be answerable here too. It stops the
    /// turn rather than only refusing the write: a user reaching for the interrupt wants the
    /// work to stop.
    #[test]
    fn ctrl_c_refuses_the_write_and_stops_the_turn() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(answer_for(key), Some(Response::Answer(Answer::Interrupt)));
        assert_eq!(Answer::Interrupt.decision(), Decision::Reject);
    }

    /// Refusing one write leaves the turn running, which is what makes it different from
    /// interrupting.
    #[test]
    fn saying_no_does_not_stop_the_turn() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(answer_for(key), Some(Response::Answer(Answer::Reject)));
    }

    #[test]
    fn a_new_file_prompt_shows_the_path_and_body() {
        let output = rendered(&request("fn main() {}", None));
        assert!(output.contains("Create"));
        assert!(output.contains("src/main.rs"));
        assert!(output.contains("fn main()"));
        assert!(output.contains("write it"));
    }

    /// Overwriting is the dangerous case, so the prompt must show the lines it discards,
    /// not merely count them.
    #[test]
    fn an_overwrite_prompt_shows_what_it_replaces() {
        let output = rendered(&request("new", Some("a\nb\nc")));
        assert!(output.contains("Overwrite"));
        assert!(output.contains("+1 -3"), "no change counts: {output}");
        for lost in ["-a", "-b", "-c"] {
            assert!(
                output.contains(lost),
                "the discarded line {lost} was not shown: {output}"
            );
        }
        assert!(output.contains("+new"), "the new line was not shown");
    }

    /// A large body must not push the question off screen, and must not be cut short either.
    ///
    /// It used to be capped so the keys would fit, and the cap counted lines while the box drew
    /// wrapped rows, so a diff with long lines pushed the question off anyway: the prompt asked
    /// nothing, and a key pressed at it answered a question that was never on the screen.
    #[test]
    fn a_long_body_keeps_the_question_on_screen_and_offers_the_rest() {
        let body = (0..200)
            .map(|n| format!("line {n} {}", "wrapping words ".repeat(8)))
            .collect::<Vec<_>>()
            .join("\n");
        let output = rendered(&request(&body, None));

        assert!(output.contains("write it"), "the question was pushed off");
        assert!(
            output.contains("more"),
            "the reviewer was not told there is more to read: {output}"
        );
    }

    /// Scrolling reaches what the box could not show, which is the whole point of having it.
    #[test]
    fn the_rest_of_a_long_body_can_be_scrolled_to() {
        let body = (0..200)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let request = request(&body, None);

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        let mut furthest = 0;
        terminal
            .draw(|frame| furthest = draw(frame, &request, 0))
            .expect("draw");
        assert!(furthest > 0, "a 200 line body reported nothing to scroll");

        terminal
            .draw(|frame| {
                draw(frame, &request, furthest);
            })
            .expect("draw");
        let drawn: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(
            drawn.contains("line 199"),
            "the end of the diff could not be reached: {drawn}"
        );
        assert!(drawn.contains("write it"), "the question scrolled away");
    }

    /// The reason this exists: a one-line change to a large file must show that one line
    /// rather than a screenful of unchanged text.
    #[test]
    fn a_small_edit_in_a_large_file_shows_only_the_change() {
        let before: String = (0..300).map(|n| format!("line {n}\n")).collect::<String>();
        let after = before.replace("line 150\n", "line 150 changed\n");

        let output = rendered(&WriteRequest {
            path: "src/main.rs".into(),
            contents: after,
            existing: Some(before),
            intent: Intent::Edit,
            untrusted: false,
        });

        assert!(output.contains("Edit"));
        assert!(output.contains("+1 -1"), "wrong counts: {output}");
        assert!(
            output.contains("+line 150 changed"),
            "the change was not shown: {output}"
        );
        assert!(
            output.contains("unchanged lines"),
            "the unchanged bulk was not elided: {output}"
        );
        assert!(output.contains("write it"), "the question was pushed off");
    }

    /// A body out of a quarantined file is the one thing on the screen nobody has read. The
    /// person approving it is the only party who ever will, so the prompt says so and marks the
    /// hunks the same way the transcript marks everything else the model was not shown.
    #[test]
    fn an_untrusted_body_is_marked_in_the_prompt() {
        let output = rendered(&WriteRequest {
            path: "game.js".into(),
            contents: "const SPEED = 50;\n".into(),
            existing: Some("const SPEED = 100;\n".into()),
            intent: Intent::Overwrite,
            untrusted: true,
        });

        assert!(
            output.contains("untrusted"),
            "the reviewer was not told what they are reading: {output}"
        );
        assert!(output.contains("┃"), "the hunks were not marked: {output}");

        // A write of the model's own words is not marked, or the mark would mean nothing.
        let ordinary = rendered(&request("new\n", Some("old\n")));
        assert!(
            !ordinary.contains("┃"),
            "an ordinary write was marked as untrusted: {ordinary}"
        );
    }

    /// A diff too large to compute must not render as an empty change.
    #[test]
    fn an_uncomputable_diff_says_so() {
        let before: String = (0..3000).map(|n| format!("old {n}\n")).collect();
        let after: String = (0..3000).map(|n| format!("new {n}\n")).collect();

        let output = rendered(&WriteRequest {
            path: "src/main.rs".into(),
            contents: after,
            existing: Some(before),
            intent: Intent::Overwrite,
            untrusted: false,
        });

        assert!(
            output.contains("too large to show"),
            "an uncomputable diff rendered as nothing: {output}"
        );
    }

    #[test]
    fn a_tiny_terminal_still_renders_the_prompt() {
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, &request("x", None), 0);
            })
            .expect("must not panic on a small area");
    }
}
