//! Asking the user about a write or a run, in the terminal.
//!
//! Turns run synchronously, so this can draw a prompt and block on a keypress from inside
//! the turn that requested the write. The alternative, collecting writes and asking
//! afterwards, would mean the model continuing on the assumption a write had happened.
//!
//! Nothing is approved by default. An unreadable terminal, an unexpected key, or a lost
//! event all resolve to refusal.

use bua_agent::confirm::{
    Confirmer, Decision, Intent, OutputRequest, RunDecision, RunRequest, WriteRequest,
};
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

    fn confirm_run(&mut self, request: &RunRequest) -> RunDecision {
        ask_run(self.terminal, request).decision()
    }

    fn confirm_read_output(&mut self, request: &OutputRequest) -> Decision {
        ask_output(self.terminal, request).decision()
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

/// What the user did with a run question.
///
/// Three answers rather than the write prompt's two. "Yes, and stop asking" is a different thing
/// from "yes", and it is the one that changes what happens next time, so it is a key of its own
/// rather than a follow-up question nobody would read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAnswer {
    Approve,
    /// Run it, and vouch for its programs for the rest of the session.
    ApproveAlways,
    Reject,
    /// Refuse the run and stop the turn that asked for it.
    Interrupt,
}

impl RunAnswer {
    /// What to tell the waiting turn. Interrupting refuses and vouches for nothing, since a turn
    /// being stopped is not consent to what it was stopped at.
    pub fn decision(self) -> RunDecision {
        match self {
            RunAnswer::Approve => RunDecision::approve(),
            RunAnswer::ApproveAlways => RunDecision::approve_always(),
            RunAnswer::Reject | RunAnswer::Interrupt => RunDecision::reject(),
        }
    }
}

/// Interpret one key press at a run prompt, or `None` for a key that answers nothing.
///
/// Separated from the loop so it can be tested without a terminal.
fn run_answer_for(key: KeyEvent) -> Option<RunResponse> {
    // The prompt blocks the whole interface, so without this Ctrl-C would do nothing at the one
    // moment a user is most likely to press it.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(RunResponse::Answer(RunAnswer::Interrupt)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('y' | 'Y') => Some(RunResponse::Answer(RunAnswer::Approve)),
        KeyCode::Char('a' | 'A') => Some(RunResponse::Answer(RunAnswer::ApproveAlways)),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(RunResponse::Answer(RunAnswer::Reject)),
        KeyCode::Up => Some(RunResponse::Scroll(-1)),
        KeyCode::Down => Some(RunResponse::Scroll(1)),
        KeyCode::PageUp => Some(RunResponse::Scroll(-10)),
        KeyCode::PageDown => Some(RunResponse::Scroll(10)),
        KeyCode::Home => Some(RunResponse::Scroll(i16::MIN)),
        KeyCode::End => Some(RunResponse::Scroll(i16::MAX)),
        // Enter is deliberately not an approval: it is the key most likely to be pressed out of
        // habit, and this prompt starts a program.
        _ => None,
    }
}

/// What a key press did at a run prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunResponse {
    Answer(RunAnswer),
    Scroll(i16),
}

/// Draw the prompt for a run and wait for an answer.
///
/// Standalone as well as available through [`TerminalConfirmer`], for the same reason the write
/// prompt is: a turn on a worker thread cannot hold the terminal, so the main thread calls this on
/// its behalf.
pub fn ask_run<B: Backend>(terminal: &mut Terminal<B>, request: &RunRequest) -> RunAnswer {
    let mut scroll = 0u16;
    loop {
        let mut most = 0u16;
        // A terminal that cannot be drawn to cannot carry a question, so refuse rather than run
        // something unseen.
        if terminal
            .draw(|frame| most = draw_run(frame, request, scroll))
            .is_err()
        {
            return RunAnswer::Reject;
        }

        match event::read() {
            Ok(TermEvent::Key(key)) => match run_answer_for(key) {
                Some(RunResponse::Answer(answer)) => return answer,
                Some(RunResponse::Scroll(by)) => {
                    scroll = scroll.saturating_add_signed(by).min(most);
                }
                None => continue,
            },
            Ok(_) => continue,
            // Losing the event stream must not run anything.
            Err(_) => return RunAnswer::Reject,
        }
    }
}

/// Draw the run confirmation, returning how far its body can be scrolled.
///
/// One line per stage, rendered by [`bua_core::Stage::display`], which quotes unambiguously: two
/// different argument vectors cannot come out looking alike, so what the reviewer reads names
/// exactly the argv the endorsement will be bound to.
fn draw_run(frame: &mut ratatui::Frame, request: &RunRequest, scroll: u16) -> u16 {
    let area = centred(frame.area());
    frame.render_widget(Clear, area);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Run ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                crate::confirm::stage_count(request.pipeline.len()),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  in {}", request.directory),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::raw(""),
    ];

    for (index, stage) in request.pipeline.stages.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}  ", index + 1),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                stage.display(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        // The binary, under the name. A name is not a program: $PATH decides what `grep` means,
        // and a person about to vouch for one should be looking at what they are vouching for.
        if let Some(path) = request.resolved.get(index) {
            lines.push(Line::from(Span::styled(
                format!("       {path}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::raw(""));

    // Said every time, because it is true every time and it is the thing a reviewer is most likely
    // to assume otherwise. A program here is not sandboxed and runs with the access the user's own
    // shell would give it.
    lines.push(Line::from(Span::styled(
        "  this is not sandboxed: it runs with the access your own shell has",
        Style::default().fg(Color::Yellow),
    )));

    // The second and independent reason to be careful, on confidentiality rather than integrity.
    // Only said when it applies, so it does not become noise that hides the case it is for.
    if request.releases_private() {
        lines.push(Line::from(Span::styled(
            "  it is also being fed your own data, which leaves here with it",
            Style::default().fg(Color::Red),
        )));
    }

    // What `a` would actually grant, in as many words. It is two things, not one, and the second
    // is the one nothing else in the interface would tell them: what the command prints stops
    // being quarantined and the model reads it. Nothing checks that assertion, so the person
    // making it has to be asked for it in those terms.
    let vouching = request.would_vouch_for();
    if !request.releases_private() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  a: trust this exact command for the rest of this session",
            Style::default().fg(Color::DarkGray),
        )));
        // The command first, then what trusting it means. The claims are about this, so a reader
        // should have it in front of them before reading them.
        for command in &vouching {
            lines.push(Line::from(Span::styled(
                format!("       {}", command.display()),
                Style::default().add_modifier(Modifier::BOLD),
            )));
        }
        lines.push(Line::from(Span::styled(
            "     which means both:",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "       it runs again unasked, side effects and all",
            Style::default().fg(Color::DarkGray),
        )));
        // The half nothing else in the interface would reveal, so it is the half that is coloured.
        lines.push(Line::from(Span::styled(
            "       what it prints is trusted, and the model reads it",
            Style::default().fg(Color::Yellow),
        )));
        // Exact arguments, so the narrowness is visible rather than assumed the other way.
        lines.push(Line::from(Span::styled(
            "     these arguments only: git log would not cover git push",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Private input asks every time whatever is remembered, so offering to stop asking would
        // be offering something that will not happen.
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  private input is asked about every time, so this one cannot be remembered",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let mut key_spans = vec![
        Span::styled(
            "  y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" run it    "),
    ];
    // Offered only where it would do something. Private input asks every time whatever is
    // remembered, so the key would promise something that will not happen.
    if !request.releases_private() {
        key_spans.push(Span::styled(
            "a",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        key_spans.push(Span::raw(" always    "));
    }
    key_spans.extend([
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" don't    "),
        Span::styled(
            "ctrl-c",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" stop the turn", Style::default().fg(Color::DarkGray)),
    ]);
    let keys = Line::from(key_spans);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Magenta))
        .title(" run this? ");
    let inside = block.inner(area);
    frame.render_widget(block, area);

    // One row for the keys, the rest for the stages, split before the body is laid out so the
    // question keeps its row whatever the body turns out to be.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inside);

    let body = Paragraph::new(lines).wrap(Wrap { trim: false });
    let drawn = body.line_count(rows[0].width) as u16;
    let furthest = drawn.saturating_sub(rows[0].height);
    let offset = scroll.min(furthest);
    frame.render_widget(body.scroll((offset, 0)), rows[0]);

    let mut keys = keys;
    if furthest > 0 {
        let below = furthest - offset;
        keys.push_span(Span::styled(
            if below > 0 {
                format!("   ↑↓ {below} more")
            } else {
                "   ↑↓ back".to_string()
            },
            Style::default().fg(Color::Magenta),
        ));
    }
    frame.render_widget(Paragraph::new(keys), rows[1]);

    furthest
}

/// `1 stage`, `2 stages`.
fn stage_count(count: usize) -> String {
    if count == 1 {
        format!("{count} stage")
    } else {
        format!("{count} stages")
    }
}

/// Draw the prompt for reading a command's output and wait for an answer.
///
/// The one prompt whose body is the thing being decided about rather than a description of it. It
/// reuses the write prompt's keys and scrolling, because the answer is the same shape: yes, no, or
/// stop.
pub fn ask_output<B: Backend>(terminal: &mut Terminal<B>, request: &OutputRequest) -> Answer {
    let mut scroll = 0u16;
    loop {
        let mut most = 0u16;
        // A terminal that cannot be drawn to cannot show the output, and approving output nobody
        // was shown is the one thing this question cannot mean.
        if terminal
            .draw(|frame| most = draw_output(frame, request, scroll))
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
            Err(_) => return Answer::Reject,
        }
    }
}

/// Draw the output for reading, returning how far it can be scrolled.
///
/// Every line of the output carries the margin bar the transcript draws down anything the model
/// was not allowed to read, and the content never gets to draw its own. A block claiming "output
/// ends here" ends nothing: the bar is the structure, and it is outside what the program wrote.
fn draw_output(frame: &mut ratatui::Frame, request: &OutputRequest, scroll: u16) -> u16 {
    let area = centred(frame.area());
    frame.render_widget(Clear, area);

    let marked = Style::default().fg(Color::Yellow);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Read ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                stage_count_of(request.lines(), "line", "lines"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  printed by {}", request.command),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  the model has not seen this. Approving puts it in its context, and it will act \
             on it.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::raw(""),
    ];

    // An empty result is a fact worth stating. Drawing nothing would read as a prompt that failed
    // to render, and the reviewer would be deciding about a blank box.
    if request.output.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("┃ ", marked),
            Span::styled("(it printed nothing)", Style::default().fg(Color::DarkGray)),
        ]));
    }
    for line in request.output.lines() {
        lines.push(Line::from(vec![
            Span::styled("┃ ", marked),
            Span::raw(line.to_string()),
        ]));
    }

    let keys = Line::from(vec![
        Span::styled(
            "  y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" let it read this    "),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" keep it back    "),
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
        .title(" let the model read this? ");
    let inside = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inside);

    let body = Paragraph::new(lines).wrap(Wrap { trim: false });
    let drawn = body.line_count(rows[0].width) as u16;
    let furthest = drawn.saturating_sub(rows[0].height);
    let offset = scroll.min(furthest);
    frame.render_widget(body.scroll((offset, 0)), rows[0]);

    let mut keys = keys;
    if furthest > 0 {
        let below = furthest - offset;
        keys.push_span(Span::styled(
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

/// `1 line`, `2 lines`.
fn stage_count_of(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
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

    fn a_run(private: bool) -> RunRequest {
        let pipeline = bua_core::Pipeline::new(vec![
            bua_core::Stage::new("git", vec!["log".into(), "--oneline".into()]),
            bua_core::Stage::new("sed", vec!["-n".into(), "1,10p".into()]),
        ]);
        RunRequest {
            pipeline: if private {
                pipeline.with_stdin(bua_core::label::Label::trusted_private())
            } else {
                pipeline
            },
            resolved: vec!["/usr/bin/git".into(), "/usr/bin/sed".into()],
            directory: "/home/someone/project".into(),
        }
    }

    /// Wide enough that the lines under test are not wrapped by the box, since what is being
    /// checked is the wording rather than the layout.
    fn rendered_run(request: &RunRequest) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_run(frame, request, 0);
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

    /// A reviewer has to see the argv, the binary behind each name, and where it will run. All
    /// three change what the run means, and none of them can be inferred from the others.
    #[test]
    fn a_run_prompt_shows_the_argv_the_binary_and_the_directory() {
        let drawn = rendered_run(&a_run(false));
        assert!(drawn.contains("git log --oneline"), "{drawn}");
        assert!(drawn.contains("sed -n 1,10p"), "{drawn}");
        assert!(drawn.contains("/usr/bin/git"), "the binary is not shown");
        assert!(drawn.contains("/home/someone/project"), "{drawn}");
    }

    /// Said every time, because it is true every time and it is the thing a reviewer is most
    /// likely to assume otherwise.
    #[test]
    fn a_run_prompt_says_it_is_not_sandboxed() {
        assert!(rendered_run(&a_run(false)).contains("not sandboxed"));
    }

    /// Vouching grants two things, and the prompt has to ask for both in as many words. The
    /// second is the one nothing else in the interface would reveal: what the command prints stops
    /// being quarantined and the model reads it. Nothing checks that assertion, so the person
    /// making it must be asked for it explicitly.
    #[test]
    fn a_run_prompt_asks_for_the_side_effects_and_the_output_together() {
        let drawn = rendered_run(&a_run(false));
        assert!(
            drawn.contains("runs again unasked"),
            "the prompt does not say vouching covers running it again: {drawn}"
        );
        assert!(
            drawn.contains("side effects"),
            "the prompt does not say vouching covers the side effects: {drawn}"
        );
        assert!(
            drawn.contains("what it prints is trusted"),
            "the prompt does not say vouching trusts the output: {drawn}"
        );
    }

    /// The entry is a command, not a program, and the prompt shows it with its arguments so the
    /// narrowness is visible rather than assumed the other way around.
    #[test]
    fn a_run_prompt_names_the_exact_command_it_would_vouch_for() {
        let drawn = rendered_run(&a_run(false));
        assert!(
            drawn.contains("/usr/bin/git log --oneline"),
            "the prompt does not name the arguments being vouched for: {drawn}"
        );
        assert!(
            drawn.contains("would not cover git push"),
            "the prompt does not say the entry is one command: {drawn}"
        );
    }

    /// Private input asks every time whatever is remembered, so the key that offers to stop
    /// asking is not offered: it would promise something that will not happen.
    #[test]
    fn a_run_that_releases_private_data_offers_no_standing_permission() {
        let drawn = rendered_run(&a_run(true));
        assert!(
            drawn.contains("cannot be remembered"),
            "the prompt offered to remember a run that will always ask: {drawn}"
        );
        assert!(
            drawn.contains("your own data"),
            "the confidentiality reason was not given: {drawn}"
        );
    }

    /// Three answers, and the one that grants a standing permission is a key of its own rather
    /// than a follow-up question nobody would read.
    #[test]
    fn the_run_keys_separate_running_once_from_running_always() {
        let once = run_answer_for(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(once, Some(RunResponse::Answer(RunAnswer::Approve)));
        assert!(!RunAnswer::Approve.decision().remember);

        let always = run_answer_for(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(always, Some(RunResponse::Answer(RunAnswer::ApproveAlways)));
        assert!(RunAnswer::ApproveAlways.decision().remember);
    }

    /// Enter is the key most likely to be pressed out of habit, and this prompt starts a program.
    #[test]
    fn enter_does_not_approve_a_run() {
        assert_eq!(
            run_answer_for(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
    }

    /// Interrupting refuses and vouches for nothing: a turn being stopped is not consent to what
    /// it was stopped at, let alone standing consent.
    #[test]
    fn ctrl_c_refuses_the_run_and_vouches_for_nothing() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            run_answer_for(key),
            Some(RunResponse::Answer(RunAnswer::Interrupt))
        );
        let decision = RunAnswer::Interrupt.decision();
        assert!(!decision.approved());
        assert!(!decision.remember);
    }

    /// Saying no refuses this run without vouching for anything or stopping the turn.
    #[test]
    fn saying_no_to_a_run_vouches_for_nothing() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(
            run_answer_for(key),
            Some(RunResponse::Answer(RunAnswer::Reject))
        );
        assert!(!RunAnswer::Reject.decision().remember);
    }

    fn an_output(text: &str) -> OutputRequest {
        OutputRequest {
            command: "find /Applications -name 'Brave Browser Nightly.app'".into(),
            output: text.into(),
            reference: "ref:5".into(),
        }
    }

    fn rendered_output(request: &OutputRequest) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_output(frame, request, 0);
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

    /// The whole point of this prompt: the bytes themselves are what the person decides about, so
    /// they have to be on the screen, along with which command printed them.
    #[test]
    fn the_output_prompt_shows_the_bytes_and_the_command() {
        let drawn = rendered_output(&an_output("/Applications/Brave Browser Nightly.app\n"));
        assert!(
            drawn.contains("/Applications/Brave Browser Nightly.app"),
            "{drawn}"
        );
        assert!(drawn.contains("find /Applications"), "{drawn}");
    }

    /// Every line carries the margin bar the transcript draws down anything the model has not been
    /// allowed to read, and the content never draws its own, so a block claiming the output has
    /// ended ends nothing.
    #[test]
    fn output_is_drawn_inside_the_margin_it_cannot_forge() {
        let drawn = rendered_output(&an_output("first\nsecond\nthird"));
        assert_eq!(
            drawn.matches('┃').count(),
            3,
            "one bar per line of output, drawn outside what the program wrote: {drawn}"
        );
    }

    /// A command that printed nothing is a fact worth stating. An empty box reads as a prompt that
    /// failed to render, and the reviewer would be answering about nothing.
    #[test]
    fn output_that_is_empty_says_so() {
        assert!(rendered_output(&an_output("")).contains("printed nothing"));
    }

    /// The person has to be told what approving does, since the consequence is not visible in the
    /// bytes: they go into the planner's context and it acts on them.
    #[test]
    fn the_output_prompt_says_what_approving_does() {
        let drawn = rendered_output(&an_output("Darwin"));
        assert!(drawn.contains("has not seen this"), "{drawn}");
        assert!(drawn.contains("act on it"), "{drawn}");
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
