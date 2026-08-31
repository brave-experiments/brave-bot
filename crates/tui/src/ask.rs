//! Putting the planner's question to the user, in the terminal.
//!
//! The same discipline as [`crate::confirm`], for the same reason: the turn is blocked waiting,
//! so an unreadable terminal, a lost event stream, or a key nobody defined must resolve to a
//! definite answer rather than to a hang. Here that answer is [`Answer::Declined`], which the
//! kernel turns into words the planner can act on.
//!
//! Reached through [`crate::confirm::TerminalConfirmer`], which is the terminal's whole side of
//! the human channel: it draws a diff for a write and this picker for a question.
//!
//! Declining is always one keypress away. A question the user does not want is a question they
//! can dismiss, and a picker with no exit would make the model able to stall the session.

use bravebot_core::ask::{Answer, Asking, Prompt, Row};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::theme;

/// Longest answer the user can type.
///
/// Generous for a sentence and far short of anything that would take the field off screen. The
/// answer is one line, not a document: a long explanation belongs in the next prompt.
const MAX_TYPED: usize = 500;

/// Columns the marker occupies, and so the indent a detail line sits at to align under its label.
const MARKER_WIDTH: usize = 4;

/// What the row offering the free-text field says.
///
/// An entry in the list rather than only a key, because an affordance nobody can see is one most
/// people never find, and answering in your own words is the way out of a set of options that
/// does not contain the answer.
const OWN_WORDS: &str = "Answer in my own words";

/// Widest a tag is drawn, in characters.
///
/// Capped here rather than in the kernel because it is a fact about the box, not about the
/// question: what the tag says is the model's, how much of it fits is the terminal's. A tag that
/// ran on would push the question it labels off the line and cost the person the sentence they
/// have to answer.
const CHIP_WIDTH: usize = 12;

/// Put a series of questions and wait for the answers.
///
/// Standalone because a turn running on a worker thread cannot hold the terminal: the main
/// thread calls this on its behalf.
///
/// Returns no answers wherever it cannot ask, which the kernel reads as a decline for every
/// question. Saying nothing is the one reply that cannot be wrong about how many questions there
/// were.
pub fn ask<B: Backend>(terminal: &mut Terminal<B>, asking: &Asking) -> Vec<Answer> {
    if asking.prompts.is_empty() {
        return Vec::new();
    }
    let mut picker = Picker::new(asking);

    loop {
        // A terminal that cannot show the question cannot collect a considered answer, so
        // decline rather than take a keypress against something nobody saw.
        if terminal.draw(|frame| picker.draw(frame)).is_err() {
            return Vec::new();
        }

        let key = match event::read() {
            // Presses only. The interface asks the terminal for disambiguated keys, which reports
            // releases too, and a release taken for a press answers the next question with the key
            // that answered this one.
            Ok(TermEvent::Key(key)) if key.kind == event::KeyEventKind::Press => key.code,
            Ok(_) => continue,
            // Losing the event stream must not invent an answer.
            Err(_) => return Vec::new(),
        };

        match picker.press(key) {
            Step::Waiting => continue,
            Step::Answered(answers) => return answers,
        }
    }
}

/// What a keypress did.
enum Step {
    Waiting,
    Answered(Vec<Answer>),
}

/// Where the person is within the question on screen.
///
/// Its own value so moving to the next question is one assignment. Marks and half-typed text
/// belong to the question they were made against, and carrying either into the next one would
/// put words in the person's mouth.
struct State {
    cursor: usize,
    /// Toggled options, in the order the user picked them, so the planner reads them back in the
    /// order they were chosen rather than in the order the model wrote them.
    chosen: Vec<usize>,
    /// The free-text field, once opened.
    typed: Option<String>,
    /// First option drawn, so a long list scrolls rather than hiding the cursor.
    offset: usize,
}

impl State {
    fn starting(prompt: &Prompt) -> Self {
        Self {
            cursor: 0,
            chosen: Vec::new(),
            // A question with no options can only be answered in the user's own words, so the
            // field opens itself rather than presenting an empty list to navigate.
            typed: prompt.rows.is_empty().then(String::new),
            offset: 0,
        }
    }
}

/// The picker's state between keypresses.
///
/// Separate from the drawing so the key discipline can be tested without a terminal: what a key
/// does is the part that must not drift.
struct Picker<'a> {
    asking: &'a Asking,
    /// Which question is on screen.
    at: usize,
    /// Settled so far, one per question, in the order they were asked.
    answers: Vec<Answer>,
    here: State,
}

impl<'a> Picker<'a> {
    fn new(asking: &'a Asking) -> Self {
        Self {
            asking,
            at: 0,
            answers: Vec::new(),
            here: State::starting(&asking.prompts[0]),
        }
    }

    fn prompt(&self) -> &Prompt {
        &self.asking.prompts[self.at]
    }

    /// Where the free-text row sits: one past the last option.
    fn own_words(&self) -> usize {
        self.prompt().rows.len()
    }

    fn on_own_words(&self) -> bool {
        self.here.cursor == self.own_words()
    }

    /// Take this question's answer and move on, finishing when there is nothing left to ask.
    ///
    /// One place decides the series is over, and it decides on the number of questions, never on
    /// what any of them was answered with. That is what makes skipping and finishing the same
    /// operation: an answer the person declined is still an answer to the question they were on.
    fn settle(&mut self, answer: Answer) -> Step {
        self.answers.push(answer);
        self.at += 1;
        if self.at == self.asking.prompts.len() {
            return Step::Answered(std::mem::take(&mut self.answers));
        }
        self.here = State::starting(self.prompt());
        Step::Waiting
    }

    fn press(&mut self, key: KeyCode) -> Step {
        match &mut self.here.typed {
            Some(text) => match key {
                KeyCode::Char(c) if text.chars().count() < MAX_TYPED => {
                    text.push(c);
                    Step::Waiting
                }
                KeyCode::Backspace => {
                    text.pop();
                    Step::Waiting
                }
                KeyCode::Enter => {
                    let answer = text.trim().to_string();
                    if answer.is_empty() {
                        // Nothing typed is not an answer. Escape is how you leave.
                        return Step::Waiting;
                    }
                    self.settle(Answer::Typed(answer))
                }
                // Back to the list, or out altogether if there was no list to begin with.
                KeyCode::Esc => {
                    self.here.typed = None;
                    if self.prompt().rows.is_empty() {
                        return self.settle(Answer::Declined);
                    }
                    Step::Waiting
                }
                _ => Step::Waiting,
            },
            None => match key {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.here.cursor = self.here.cursor.saturating_sub(1);
                    Step::Waiting
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Past the last option sits the free-text row, so the range is one longer
                    // than the list.
                    self.here.cursor = (self.here.cursor + 1).min(self.own_words());
                    Step::Waiting
                }
                // Marks in both kinds of question. A key that works on one and silently does
                // nothing on the other reads as broken, and the person presses it again rather
                // than reaching for enter.
                KeyCode::Char(' ') if !self.on_own_words() => {
                    self.toggle(self.here.cursor);
                    Step::Waiting
                }
                // On the free-text row both keys mean the same thing, which is what the row is
                // for: there is nothing there to mark.
                KeyCode::Char(' ') | KeyCode::Char('o') => {
                    self.here.typed = Some(String::new());
                    Step::Waiting
                }
                KeyCode::Enter if self.on_own_words() => {
                    self.here.typed = Some(String::new());
                    Step::Waiting
                }
                // Confirms what is toggled, or the option under the cursor when nothing is. One
                // rule for both kinds of question, and no state the user cannot get out of.
                KeyCode::Enter => {
                    let picked = if self.here.chosen.is_empty() {
                        self.prompt()
                            .rows
                            .get(self.here.cursor)
                            .map(|row| vec![row.index])
                            .unwrap_or_default()
                    } else {
                        self.here.chosen.clone()
                    };
                    if picked.is_empty() {
                        return Step::Waiting;
                    }
                    self.settle(Answer::Chosen(picked))
                }
                KeyCode::Esc => self.settle(Answer::Declined),
                _ => Step::Waiting,
            },
        }
    }

    /// Mark or unmark the option under the cursor.
    ///
    /// Where the question takes one answer, marking a second replaces the first: a list that let
    /// two be marked would then have to decide between them, and the person would have said one
    /// thing and been reported as saying another.
    fn toggle(&mut self, cursor: usize) {
        let Some(index) = self.prompt().rows.get(cursor).map(|row| row.index) else {
            return;
        };
        let multiple = self.prompt().multiple;
        match self.here.chosen.iter().position(|i| *i == index) {
            Some(at) => {
                self.here.chosen.remove(at);
            }
            None => {
                if !multiple {
                    self.here.chosen.clear();
                }
                self.here.chosen.push(index);
            }
        }
    }

    fn is_chosen(&self, row: &Row) -> bool {
        self.here.chosen.contains(&row.index)
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = centred(frame.area());
        frame.render_widget(Clear, area);

        let mut lines = Vec::new();
        if let Some(chip) = chip(&self.prompt().header) {
            lines.push(Line::from(chip));
        }
        lines.push(Line::from(Span::styled(
            self.prompt().question.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::raw(""));

        // An option costs a line for its label and another for its detail. Where any option has
        // one, the list is spaced out as well, since two unspaced options each carrying a second
        // line read as four options rather than two.
        let spaced = self.prompt().rows.iter().any(|row| row.detail.is_some());
        let gap = usize::from(spaced);

        // Everything drawn below the list: the two borders, the spacer above the tail, and the
        // tail itself. With the field open that is the field and its own keys; otherwise it is
        // the free-text row, whatever space is kept above it, and the key hints.
        //
        // The hints are measured rather than assumed to be one line. On a narrow box they wrap,
        // and the line they wrap onto has to be counted or the way out is drawn off the bottom.
        let inside = (area.width as usize).saturating_sub(2).max(1);
        let hint: usize = self
            .keys()
            .iter()
            .map(|span| span.content.chars().count())
            .sum();
        let tail = if self.here.typed.is_some() {
            2
        } else {
            hint.div_ceil(inside).max(1) + 1 + gap
        };
        let reserved = lines.len() + 3 + tail;
        let budget = (area.height as usize).saturating_sub(reserved).max(1);
        let height = |row: &Row, first: bool| {
            1 + usize::from(row.detail.is_some()) + if first { 0 } else { gap }
        };

        // Scrolled against the tallest an option can be, so the estimate is never larger than
        // what actually fits and the cursor cannot land below the last drawn line.
        let tallest = 1 + gap + gap;
        self.scroll_to_cursor((budget / tallest).max(1));

        // Twice, because the line saying how many options were cut is itself a line, and it
        // exists only once something has been cut. Fitting the list first and then discovering
        // there is no room left to say so is how an option ends up silently off the bottom.
        let mut visible = self.fitting(budget, height);
        if visible < self.prompt().rows.len() {
            visible = self.fitting(budget.saturating_sub(1), height);
        }

        for (nth, row) in self
            .prompt()
            .rows
            .iter()
            .skip(self.here.offset)
            .take(visible)
            .enumerate()
        {
            if nth > 0 && spaced {
                lines.push(Line::raw(""));
            }

            let here = row.index == self.here.cursor;
            // A mark has to be visible or pressing space looks like nothing happened. On a
            // one-answer question the cursor arrow gives way to the mark, so the list stays
            // uncluttered until the person actually picks something.
            let marker = if self.prompt().multiple {
                if self.is_chosen(row) { "[x] " } else { "[ ] " }
            } else if self.is_chosen(row) {
                "  ◉ "
            } else if here {
                "  › "
            } else {
                "    "
            };
            let style = if here {
                Style::default()
                    .fg(theme::brand_primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(row.label.clone(), style),
            ]));

            // Beneath the label rather than beside it, aligned under it, so a long explanation
            // cannot push the option it belongs to off the edge of the box.
            if let Some(detail) = &row.detail {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(MARKER_WIDTH)),
                    Span::styled(detail.clone(), Style::default().fg(theme::muted())),
                ]));
            }
        }

        // A list cut short must say so, or an option nobody scrolled to reads as an option that
        // was never offered.
        let hidden = self.prompt().rows.len() - visible;
        if hidden > 0 {
            lines.push(Line::from(Span::styled(
                format!("    … {hidden} more, use the arrow keys"),
                Style::default().fg(theme::muted()),
            )));
        }

        // Last in the list, and drawn only while the field it opens is closed: with the field on
        // screen the person is already answering in their own words, and a row inviting them to
        // do what they are doing is one more thing to read.
        //
        // No mark of its own in either kind of question, because it is a way in rather than a
        // choice: a checkbox beside it would say it could be picked alongside the options.
        if self.here.typed.is_none() {
            if spaced {
                lines.push(Line::raw(""));
            }
            let style = if self.on_own_words() {
                Style::default()
                    .fg(theme::brand_primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::muted())
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if self.on_own_words() {
                        "  › "
                    } else {
                        "    "
                    },
                    style,
                ),
                Span::styled(OWN_WORDS, style),
            ]));
        }

        lines.push(Line::raw(""));
        match &self.here.typed {
            Some(text) => {
                lines.push(Line::from(vec![
                    Span::styled(
                        "  > ",
                        Style::default()
                            .fg(theme::brand_primary())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(text.clone()),
                    Span::styled("▏", Style::default().fg(theme::brand_primary())),
                ]));
                // The field needs its own keys. Drawing none, as this did, leaves a person who
                // opened it with no way out that they can see, and the way out is the hint they
                // most need.
                lines.push(Line::from(self.field_keys()));
            }
            None => lines.push(Line::from(self.keys())),
        }

        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme::brand_primary()))
                        .title(self.title()),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    /// What the box is called, and where the person is in the series.
    ///
    /// A lone question is not counted: "1 of 1" is noise on the common case, and the count is
    /// there to tell someone partway through a series how much of it is left.
    fn title(&self) -> String {
        if self.asking.prompts.len() == 1 {
            return " the agent is asking ".to_string();
        }
        format!(
            " the agent is asking ({} of {}) ",
            self.at + 1,
            self.asking.prompts.len()
        )
    }

    fn keys(&self) -> Vec<Span<'static>> {
        let bold = Style::default()
            .fg(theme::brand_primary())
            .add_modifier(Modifier::BOLD);
        // Short enough to stay on one line in a narrow terminal. A key hint that wraps is a key
        // hint the person skims past, and the way out is the one they must not miss.
        let mut spans = vec![Span::styled("  ↑↓", bold), Span::raw(" move   ")];
        spans.push(Span::styled("space", bold));
        spans.push(Span::raw(if self.prompt().multiple {
            " pick any   "
        } else {
            " pick   "
        }));
        spans.push(Span::styled("enter", bold));
        spans.push(Span::raw(" answer   "));
        // No hint for the free-text field: it is a row in the list now, and a line this short
        // cannot afford to say twice what the person can already read.
        spans.push(Span::styled(
            "esc",
            Style::default()
                .fg(theme::fail())
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" skip"));
        spans
    }

    /// Keys for the free-text field, which are not the keys for the list.
    fn field_keys(&self) -> Vec<Span<'static>> {
        let bold = Style::default()
            .fg(theme::brand_primary())
            .add_modifier(Modifier::BOLD);
        vec![
            Span::styled("  enter", bold),
            Span::raw(" answer   "),
            Span::styled(
                "esc",
                Style::default()
                    .fg(theme::fail())
                    .add_modifier(Modifier::BOLD),
            ),
            // Escape goes back to the options where there are any, and out where there are none.
            Span::raw(if self.prompt().rows.is_empty() {
                " skip the question"
            } else {
                " back to the options"
            }),
        ]
    }

    /// How many options fit in this many lines, counting from the first one drawn.
    ///
    /// At least one, always: a box too small for even one option should show the option and be
    /// cut off, rather than show an empty list and say everything is hidden.
    fn fitting(&self, budget: usize, height: impl Fn(&Row, bool) -> usize) -> usize {
        let mut used = 0;
        let mut visible = 0;
        for (nth, row) in self.prompt().rows.iter().skip(self.here.offset).enumerate() {
            let cost = height(row, nth == 0);
            if visible > 0 && used + cost > budget {
                break;
            }
            used += cost;
            visible += 1;
        }
        visible
    }

    fn scroll_to_cursor(&mut self, visible: usize) {
        if visible == 0 {
            return;
        }
        // The free-text row is drawn below the list rather than in it, so a cursor resting there
        // scrolls the list to its end and no further.
        let cursor = self.here.cursor.min(self.own_words().saturating_sub(1));
        if cursor < self.here.offset {
            self.here.offset = cursor;
        } else if cursor >= self.here.offset + visible {
            self.here.offset = cursor + 1 - visible;
        }
    }
}

/// Put remembered answers back beside the fresh ones, in the order the questions were asked.
///
/// The picker is only shown the questions nobody has settled yet, so what it hands back is
/// shorter than the series and lines up with nothing. This is where the two are woven together.
///
/// A fresh answer that never arrived leaves a decline rather than pulling the next one forward.
/// Shifting answers up would report the person as having said, about one question, what they
/// said about another.
pub(crate) fn in_order(known: Vec<Option<Answer>>, fresh: Vec<Answer>) -> Vec<Answer> {
    let mut fresh = fresh.into_iter();
    known
        .into_iter()
        .map(|earlier| earlier.or_else(|| fresh.next()).unwrap_or(Answer::Declined))
        .collect()
}

/// The tag a question is shown under, or nothing where it has none.
///
/// Reversed rather than merely coloured so it reads as a tag at a glance, which is the whole
/// point of it: with several questions in a series the sentence says what is being asked and the
/// tag says which of the pending decisions this one is.
fn chip(header: &str) -> Option<Span<'static>> {
    if header.is_empty() {
        return None;
    }
    let mut text: String = header.chars().take(CHIP_WIDTH).collect();
    if header.chars().count() > CHIP_WIDTH {
        text = header.chars().take(CHIP_WIDTH - 1).collect();
        text.push('\u{2026}');
    }
    Some(Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(theme::brand_primary())
            .add_modifier(Modifier::REVERSED),
    ))
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
    use bravebot_core::ask::{Choice, Question};
    use ratatui::backend::TestBackend;

    fn prompt(multiple: bool) -> Prompt {
        bravebot_core::ask::prompt(&Question::new(
            "Cache layer",
            "Which cache layer?",
            vec![
                Choice::new("HTTP", Some("in front of the handler".into())),
                Choice::new("Query", None),
                Choice::new("Neither", None),
            ],
            multiple,
        ))
    }

    /// A series of one, which is what most of these tests are about.
    fn one(prompt: &Prompt) -> Asking {
        Asking {
            prompts: vec![prompt.clone()],
        }
    }

    fn answer(prompt: &Prompt, keys: &[KeyCode]) -> Option<Answer> {
        answers(&one(prompt), keys).map(|mut given| given.remove(0))
    }

    fn answers(asking: &Asking, keys: &[KeyCode]) -> Option<Vec<Answer>> {
        let mut picker = Picker::new(asking);
        for key in keys {
            if let Step::Answered(given) = picker.press(*key) {
                return Some(given);
            }
        }
        None
    }

    fn rendered(prompt: &Prompt) -> String {
        screen_rows(prompt, 80, 24).join("\n")
    }

    /// The screen as separate rows, so a test can assert what sits on a line of its own rather
    /// than only that the text is somewhere on the display.
    fn screen_rows(prompt: &Prompt, width: u16, height: u16) -> Vec<String> {
        rows_after(&one(prompt), &[], width, height)
    }

    /// The screen once some keys have been pressed, for asserting what a later question in a
    /// series looks like.
    fn rows_after(asking: &Asking, keys: &[KeyCode], width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let mut picker = Picker::new(asking);
        for key in keys {
            picker.press(*key);
        }
        terminal.draw(|frame| picker.draw(frame)).expect("drawn");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Which column this text starts at, counted in characters rather than bytes: the border and
    /// the cursor marker are multi-byte, so byte offsets do not line up between rows.
    fn column_of(row: &str, text: &str) -> usize {
        let at = row
            .find(text)
            .unwrap_or_else(|| panic!("{text} is not on this row"));
        row[..at].chars().count()
    }

    /// Which row holds this text, for asserting on the layout rather than its contents.
    fn row_holding(rows: &[String], text: &str) -> usize {
        rows.iter()
            .position(|row| row.contains(text))
            .unwrap_or_else(|| panic!("{text} is not on screen"))
    }

    #[test]
    fn moving_and_confirming_picks_the_option_under_the_cursor() {
        assert_eq!(
            answer(&prompt(false), &[KeyCode::Down, KeyCode::Enter]),
            Some(Answer::Chosen(vec![1]))
        );
    }

    #[test]
    fn vim_keys_move_as_the_arrows_do() {
        assert_eq!(
            answer(&prompt(false), &[KeyCode::Char('j'), KeyCode::Enter]),
            answer(&prompt(false), &[KeyCode::Down, KeyCode::Enter])
        );
    }

    /// The cursor must not run off either end, or Enter would confirm nothing and the picker
    /// would look stuck. Its last stop is the free-text row, one past the options.
    #[test]
    fn the_cursor_stops_at_the_ends_of_the_list() {
        let up = [KeyCode::Up, KeyCode::Up, KeyCode::Enter];
        assert_eq!(answer(&prompt(false), &up), Some(Answer::Chosen(vec![0])));

        let down = [KeyCode::Down; 9];
        let mut keys = down.to_vec();
        keys.extend([KeyCode::Up, KeyCode::Enter]);
        assert_eq!(
            answer(&prompt(false), &keys),
            Some(Answer::Chosen(vec![2])),
            "the cursor ran past the free-text row"
        );
    }

    #[test]
    fn several_options_can_be_picked_when_the_question_allows_it() {
        let keys = [
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Char(' '),
            KeyCode::Enter,
        ];
        assert_eq!(
            answer(&prompt(true), &keys),
            Some(Answer::Chosen(vec![0, 2]))
        );
    }

    #[test]
    fn a_pick_can_be_taken_back() {
        let keys = [
            KeyCode::Char(' '),
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Char(' '),
            KeyCode::Enter,
        ];
        assert_eq!(answer(&prompt(true), &keys), Some(Answer::Chosen(vec![1])));
    }

    /// Space marks on a one-answer question too. A key that works on one kind of question and
    /// silently does nothing on the other reads as broken, and the person presses it again
    /// instead of reaching for enter.
    #[test]
    fn space_marks_the_option_on_a_single_answer_question() {
        let keys = [KeyCode::Char(' '), KeyCode::Enter];
        assert_eq!(answer(&prompt(false), &keys), Some(Answer::Chosen(vec![0])));
    }

    /// Marking is not answering, so the cursor can still be moved afterwards and the mark is what
    /// enter confirms, not wherever the cursor ended up.
    #[test]
    fn a_mark_survives_moving_the_cursor() {
        let keys = [
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Enter,
        ];
        assert_eq!(answer(&prompt(false), &keys), Some(Answer::Chosen(vec![0])));
    }

    /// One answer means one mark. Two marks would leave the picker deciding between them, and the
    /// person would have said one thing and been reported as saying another.
    #[test]
    fn marking_a_second_option_replaces_the_first_when_only_one_answer_is_allowed() {
        let keys = [
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Char(' '),
            KeyCode::Enter,
        ];
        assert_eq!(answer(&prompt(false), &keys), Some(Answer::Chosen(vec![1])));
    }

    /// And a mark can be taken off again, leaving enter to confirm the cursor as it would have.
    #[test]
    fn a_mark_on_a_single_answer_question_can_be_taken_back() {
        let keys = [
            KeyCode::Char(' '),
            KeyCode::Char(' '),
            KeyCode::Down,
            KeyCode::Enter,
        ];
        assert_eq!(answer(&prompt(false), &keys), Some(Answer::Chosen(vec![1])));
    }

    fn platforms() -> Prompt {
        bravebot_core::ask::prompt(&Question::new(
            "Platforms",
            "Which platforms?",
            vec![Choice::new("Linux", None), Choice::new("macOS", None)],
            true,
        ))
    }

    fn a_series() -> Asking {
        Asking {
            prompts: vec![prompt(false), platforms()],
        }
    }

    /// The point of a series: one question is settled and the next is put, without the turn
    /// having to go back to the model in between.
    #[test]
    fn answering_one_question_moves_on_to_the_next() {
        assert_eq!(
            answers(&a_series(), &[KeyCode::Enter]),
            None,
            "the series ended on its first answer"
        );
        assert_eq!(
            answers(&a_series(), &[KeyCode::Enter, KeyCode::Enter]),
            Some(vec![Answer::Chosen(vec![0]), Answer::Chosen(vec![0])])
        );
    }

    /// The kernel pairs answers to questions by position, so the order they come back in is the
    /// order the questions were asked, whatever order the person moved through the options in.
    #[test]
    fn the_answers_come_back_in_the_order_the_questions_were_asked() {
        let given = answers(
            &a_series(),
            &[
                KeyCode::Down,
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        )
        .expect("the series finished");
        assert_eq!(
            given,
            vec![Answer::Chosen(vec![1]), Answer::Chosen(vec![0, 1])]
        );
    }

    /// Escape skips the question on screen rather than abandoning the series. A person who
    /// cannot answer one of four should not lose the three they can.
    #[test]
    fn skipping_a_question_moves_on_rather_than_abandoning_the_series() {
        let given = answers(&a_series(), &[KeyCode::Esc, KeyCode::Enter]).expect("finished");
        assert_eq!(given, vec![Answer::Declined, Answer::Chosen(vec![0])]);
    }

    /// And the last question is the one that ends it, however it was settled.
    #[test]
    fn skipping_the_last_question_still_ends_the_series() {
        let given = answers(&a_series(), &[KeyCode::Enter, KeyCode::Esc]).expect("finished");
        assert_eq!(given, vec![Answer::Chosen(vec![0]), Answer::Declined]);
    }

    /// Marks belong to the question they were made against. Carrying them forward would report
    /// the person as having said something about a question they had not reached.
    #[test]
    fn marks_from_one_question_do_not_carry_into_the_next() {
        let given = answers(
            &a_series(),
            &[
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Enter,
            ],
        )
        .expect("finished");
        assert_eq!(
            given,
            vec![Answer::Chosen(vec![2]), Answer::Chosen(vec![0])],
            "the second question inherited the first question's cursor or marks"
        );
    }

    /// Half-typed text is no different: it was written in answer to the question on screen.
    #[test]
    fn a_field_left_open_does_not_follow_the_person_to_the_next_question() {
        let given = answers(
            &a_series(),
            &[
                KeyCode::Char('o'),
                KeyCode::Char('h'),
                KeyCode::Char('i'),
                KeyCode::Enter,
                KeyCode::Enter,
            ],
        )
        .expect("finished");
        assert_eq!(
            given,
            vec![Answer::Typed("hi".into()), Answer::Chosen(vec![0])]
        );
    }

    /// Someone partway through a series needs to know how much of it is left, or a second
    /// question arriving where they expected to be finished reads as the agent looping.
    #[test]
    fn the_position_in_the_series_is_shown() {
        let rows = rows_after(&a_series(), &[], 80, 24);
        assert!(
            rows.iter().any(|row| row.contains("(1 of 2)")),
            "the first question did not say where it sat: {rows:?}"
        );

        let later = rows_after(&a_series(), &[KeyCode::Enter], 80, 24);
        assert!(
            later.iter().any(|row| row.contains("(2 of 2)")),
            "the second question did not say where it sat: {later:?}"
        );
    }

    /// A lone question cannot be anywhere else in anything, so counting it is noise.
    #[test]
    fn a_lone_question_is_not_labelled_one_of_one() {
        let rows = screen_rows(&prompt(false), 80, 24);
        assert!(
            !rows.iter().any(|row| row.contains("1 of 1")),
            "a single question counted itself: {rows:?}"
        );
    }

    /// The next question has to be the one drawn. Leaving the first on screen would take the
    /// answer to one question against the text of another.
    #[test]
    fn the_next_question_is_the_one_drawn() {
        let later = rows_after(&a_series(), &[KeyCode::Enter], 80, 24);
        let screen = later.join("\n");
        assert!(screen.contains("Which platforms?"), "{screen}");
        assert!(!screen.contains("Which cache layer?"), "{screen}");
    }

    /// A way in that nobody can see is one most people never find, and answering in your own
    /// words is the way out of a set of options that does not contain the answer.
    #[test]
    fn answering_in_your_own_words_is_offered_in_the_list() {
        let rows = screen_rows(&prompt(false), 80, 24);
        assert!(
            rows.iter().any(|row| row.contains(OWN_WORDS)),
            "the free-text row is not on screen: {rows:?}"
        );
    }

    /// It sits after the options rather than among them, because it is a way in rather than a
    /// choice and putting it in the middle would read as one of the answers.
    #[test]
    fn the_free_text_row_sits_below_every_option() {
        let rows = screen_rows(&prompt(false), 80, 24);
        assert!(
            row_holding(&rows, OWN_WORDS) > row_holding(&rows, "Neither"),
            "the free-text row was drawn among the options: {rows:?}"
        );
    }

    /// The cursor has to reach it, or it is a row the person can read and not use.
    #[test]
    fn the_cursor_reaches_the_free_text_row_past_the_last_option() {
        let keys = [
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Char('h'),
            KeyCode::Char('i'),
            KeyCode::Enter,
        ];
        assert_eq!(
            answer(&prompt(false), &keys),
            Some(Answer::Typed("hi".into()))
        );
    }

    /// Space on that row means what enter means. There is nothing there to mark, so a key that
    /// marks elsewhere and does nothing here would read as broken.
    #[test]
    fn space_on_the_free_text_row_opens_the_field_rather_than_marking() {
        let keys = [
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Char(' '),
            KeyCode::Char('n'),
            KeyCode::Char('o'),
            KeyCode::Enter,
        ];
        assert_eq!(
            answer(&prompt(true), &keys),
            Some(Answer::Typed("no".into()))
        );
    }

    /// With the field open the person is already answering in their own words, so a row inviting
    /// them to do what they are doing is one more thing to read.
    #[test]
    fn the_free_text_row_makes_way_for_the_field_it_opens() {
        let rows = rows_after(&one(&prompt(false)), &[KeyCode::Char('o')], 80, 24);
        assert!(
            !rows.iter().any(|row| row.contains(OWN_WORDS)),
            "the row was still drawn under the open field: {rows:?}"
        );
    }

    /// Marks survive a look at the field: escape comes back to the list, and what was picked
    /// before opening it is still picked.
    #[test]
    fn opening_the_field_and_leaving_it_keeps_what_was_marked() {
        let keys = [
            KeyCode::Char(' '),
            KeyCode::Char('o'),
            KeyCode::Esc,
            KeyCode::Enter,
        ];
        assert_eq!(answer(&prompt(true), &keys), Some(Answer::Chosen(vec![0])));
    }

    /// The weave the main loop depends on. Fresh answers fill the gaps the remembered ones left,
    /// and every question keeps its own place.
    #[test]
    fn remembered_answers_keep_their_places_beside_the_fresh_ones() {
        let known = vec![
            Some(Answer::Chosen(vec![1])),
            None,
            Some(Answer::Declined),
            None,
        ];
        let fresh = vec![Answer::Typed("second".into()), Answer::Chosen(vec![0])];
        assert_eq!(
            in_order(known, fresh),
            vec![
                Answer::Chosen(vec![1]),
                Answer::Typed("second".into()),
                Answer::Declined,
                Answer::Chosen(vec![0]),
            ]
        );
    }

    /// A series everyone has already answered asks nothing, and still reports every answer.
    #[test]
    fn a_series_answered_entirely_from_memory_asks_nothing() {
        let known = vec![Some(Answer::Chosen(vec![0])), Some(Answer::Declined)];
        assert_eq!(
            in_order(known, Vec::new()),
            vec![Answer::Chosen(vec![0]), Answer::Declined]
        );
    }

    /// An interface that answered fewer than it was shown leaves the rest declined. Pulling the
    /// next answer forward would report the person as having said, about one question, what they
    /// said about another.
    #[test]
    fn an_interface_that_answered_fewer_leaves_the_rest_declined() {
        let known = vec![None, None, None];
        let fresh = vec![Answer::Chosen(vec![0])];
        assert_eq!(
            in_order(known, fresh),
            vec![Answer::Chosen(vec![0]), Answer::Declined, Answer::Declined]
        );
    }

    /// A picker shown nothing draws nothing and answers nothing, which is what lets the main
    /// loop call it unconditionally.
    #[test]
    fn a_series_with_no_questions_in_it_is_not_drawn() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        assert!(ask(&mut terminal, &Asking::default()).is_empty());
    }

    /// Pressing space has to show, or it reads as a key that does nothing.
    #[test]
    fn a_marked_option_is_drawn_as_marked() {
        let question = one(&prompt(false));
        let mut picker = Picker::new(&question);
        picker.press(KeyCode::Char(' '));

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| picker.draw(frame)).expect("drawn");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            screen.contains("◉"),
            "a marked option is not shown as marked"
        );
    }

    #[test]
    fn the_user_can_answer_in_their_own_words() {
        let keys = [
            KeyCode::Char('o'),
            KeyCode::Char('n'),
            KeyCode::Char('o'),
            KeyCode::Enter,
        ];
        assert_eq!(
            answer(&prompt(false), &keys),
            Some(Answer::Typed("no".into()))
        );
    }

    /// Backspace has to work, or a typo can only be fixed by abandoning the answer.
    #[test]
    fn typing_can_be_corrected() {
        let keys = [
            KeyCode::Char('o'),
            KeyCode::Char('n'),
            KeyCode::Char('x'),
            KeyCode::Backspace,
            KeyCode::Char('o'),
            KeyCode::Enter,
        ];
        assert_eq!(
            answer(&prompt(false), &keys),
            Some(Answer::Typed("no".into()))
        );
    }

    /// An empty field is not an answer, for the same reason Enter does not approve a write: it
    /// is the key most likely to be pressed out of habit.
    #[test]
    fn an_empty_field_is_not_an_answer() {
        let keys = [KeyCode::Char('o'), KeyCode::Enter, KeyCode::Enter];
        assert_eq!(answer(&prompt(false), &keys), None);
    }

    /// Escape from the field goes back to the list rather than out of the question, so a person
    /// who opened it by mistake has not lost the options.
    #[test]
    fn leaving_the_field_returns_to_the_options() {
        let keys = [
            KeyCode::Char('o'),
            KeyCode::Char('x'),
            KeyCode::Esc,
            KeyCode::Down,
            KeyCode::Enter,
        ];
        assert_eq!(answer(&prompt(false), &keys), Some(Answer::Chosen(vec![1])));
    }

    /// Declining must always be reachable, or a question could hold the session open.
    #[test]
    fn escape_declines() {
        assert_eq!(
            answer(&prompt(false), &[KeyCode::Esc]),
            Some(Answer::Declined)
        );
        assert_eq!(
            answer(&prompt(true), &[KeyCode::Esc]),
            Some(Answer::Declined)
        );
    }

    #[test]
    fn a_question_with_no_options_opens_the_field_at_once() {
        let bare = bravebot_core::ask::prompt(&Question::new(
            "Branch",
            "Which branch?",
            Vec::new(),
            false,
        ));
        assert_eq!(
            answer(
                &bare,
                &[KeyCode::Char('m'), KeyCode::Char('e'), KeyCode::Enter]
            ),
            Some(Answer::Typed("me".into()))
        );
    }

    /// With no list to fall back to, escape from the field is a decline rather than a dead end.
    #[test]
    fn escaping_a_question_with_no_options_declines() {
        let bare = bravebot_core::ask::prompt(&Question::new(
            "Branch",
            "Which branch?",
            Vec::new(),
            false,
        ));
        assert_eq!(answer(&bare, &[KeyCode::Esc]), Some(Answer::Declined));
    }

    /// A typed answer is bounded so the field cannot be pushed off the screen. The cap drops
    /// further keystrokes rather than truncating what is already there.
    #[test]
    fn a_typed_answer_is_bounded() {
        let question = one(&prompt(false));
        let mut picker = Picker::new(&question);
        picker.press(KeyCode::Char('o'));
        for _ in 0..(MAX_TYPED + 50) {
            picker.press(KeyCode::Char('a'));
        }
        match picker.press(KeyCode::Enter) {
            Step::Answered(given) => match given.first() {
                Some(Answer::Typed(text)) => assert_eq!(text.chars().count(), MAX_TYPED),
                other => panic!("the field did not answer: {other:?}"),
            },
            _ => panic!("the field did not answer"),
        }
    }

    #[test]
    fn the_question_and_every_option_are_drawn() {
        let screen = rendered(&prompt(false));
        for expected in [
            "Which cache layer?",
            "HTTP",
            "in front of the handler",
            "Query",
            "Neither",
        ] {
            assert!(screen.contains(expected), "{expected} is not on screen");
        }
    }

    /// The way out has to be visible. A person who cannot see how to skip will answer something
    /// rather than nothing, and the planner will take it as their view.
    #[test]
    fn the_keys_say_how_to_skip() {
        assert!(rendered(&prompt(false)).contains("esc skip"));
    }

    /// The way out must survive a narrow terminal. It may wrap, but it may not fall off the
    /// screen: a person who cannot see how to skip will answer something rather than nothing, and
    /// the planner will take it as their view.
    #[test]
    fn the_way_out_survives_a_narrow_terminal() {
        for multiple in [false, true] {
            let mut terminal = Terminal::new(TestBackend::new(48, 20)).expect("terminal");
            let question = one(&prompt(multiple));
            let mut picker = Picker::new(&question);
            terminal.draw(|frame| picker.draw(frame)).expect("drawn");

            let screen: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(screen.contains("esc"), "the escape hint is off screen");
            assert!(screen.contains("skip"), "the escape hint is off screen");
        }
    }

    /// A question that takes several answers has to say so, or the person picks one and the
    /// others are never considered.
    #[test]
    fn a_multiple_choice_question_shows_how_to_pick_several() {
        assert!(rendered(&prompt(true)).contains("pick any"));
        assert!(!rendered(&prompt(false)).contains("pick any"));
    }

    /// Space is offered on both kinds of question, since it now works on both.
    #[test]
    fn the_keys_offer_space_whatever_the_question() {
        assert!(rendered(&prompt(false)).contains("space"));
        assert!(rendered(&prompt(true)).contains("space"));
    }

    /// The tag is what tells one question from another when several arrive together, so it has
    /// to be on screen and above the sentence it labels.
    #[test]
    fn the_tag_is_drawn_above_the_question() {
        let rows = screen_rows(&prompt(false), 80, 24);
        let top = row_holding(&rows, "the agent is asking");
        assert_eq!(row_holding(&rows, "Cache layer"), top + 1, "{rows:?}");
        assert_eq!(
            row_holding(&rows, "Which cache layer?"),
            top + 2,
            "{rows:?}"
        );
    }

    /// A tag the model wrote long must not cost the person the sentence they have to answer.
    #[test]
    fn a_long_tag_is_shortened_rather_than_pushing_the_question_off_the_line() {
        let wordy = bravebot_core::ask::prompt(&Question::new(
            "An extremely long tag nobody could read at a glance",
            "Which cache layer?",
            vec![Choice::new("HTTP", None)],
            false,
        ));
        let rows = screen_rows(&wordy, 80, 24);
        assert!(
            rows.iter().any(|row| row.contains('\u{2026}')),
            "a long tag was not shortened: {rows:?}"
        );
        assert!(
            row_holding(&rows, "Which cache layer?") > 0,
            "the question was pushed off the box"
        );
    }

    /// Nothing is drawn for a question with no tag. An empty chip is a smear of colour the
    /// person has to work out the meaning of.
    #[test]
    fn a_question_with_no_tag_draws_no_chip() {
        let untagged = bravebot_core::ask::prompt(&Question::new(
            "",
            "Which cache layer?",
            vec![Choice::new("HTTP", None)],
            false,
        ));
        let rows = screen_rows(&untagged, 80, 24);
        assert_eq!(
            row_holding(&rows, "Which cache layer?"),
            row_holding(&rows, "the agent is asking") + 1,
            "an empty tag still took a line: {rows:?}"
        );
    }

    /// A list too long for the box must say what is missing. Silence would read as an option
    /// that was never offered.
    #[test]
    fn a_list_cut_short_says_so() {
        let many = bravebot_core::ask::prompt(&Question::new(
            "File",
            "Which file?",
            (0..60)
                .map(|i| Choice::new(format!("file-{i}.rs"), None))
                .collect(),
            false,
        ));
        assert!(rendered(&many).contains("more, use the arrow keys"));
    }

    /// A detail belongs under its label, not beside it. Beside it, a long explanation is pushed
    /// off the edge of the box or wraps into the next option and reads as part of it.
    #[test]
    fn an_option_detail_sits_on_its_own_line_beneath_the_label() {
        let rows = screen_rows(&prompt(false), 80, 24);
        let label = row_holding(&rows, "HTTP");
        let detail = row_holding(&rows, "in front of the handler");

        assert_eq!(
            detail,
            label + 1,
            "the detail is not on the line directly beneath its label"
        );
        assert!(
            !rows[label].contains("in front of the handler"),
            "the detail is still beside the label: {}",
            rows[label]
        );
    }

    /// Aligned under the label rather than at the margin, so the pair reads as one option.
    #[test]
    fn a_detail_is_indented_to_line_up_with_its_label() {
        let rows = screen_rows(&prompt(false), 80, 24);
        let label = &rows[row_holding(&rows, "HTTP")];
        let detail = &rows[row_holding(&rows, "in front of the handler")];

        assert_eq!(
            column_of(label, "HTTP"),
            column_of(detail, "in front of the handler"),
            "the detail does not line up with its label"
        );
    }

    /// With a second line under each option, unspaced options read as twice as many options.
    #[test]
    fn options_carrying_details_are_spaced_apart() {
        let rows = screen_rows(&prompt(false), 80, 24);
        let detail = row_holding(&rows, "in front of the handler");
        assert_eq!(
            row_holding(&rows, "Query"),
            detail + 2,
            "the next option runs straight into the detail above it"
        );
    }

    /// And a list with nothing to explain stays compact: blank lines between bare options would
    /// waste the height a long list needs.
    #[test]
    fn options_without_details_are_not_spaced_apart() {
        let bare = bravebot_core::ask::prompt(&Question::new(
            "Which",
            "Which?",
            vec![Choice::new("One", None), Choice::new("Two", None)],
            false,
        ));
        let rows = screen_rows(&bare, 80, 24);
        assert_eq!(
            row_holding(&rows, "Two"),
            row_holding(&rows, "One") + 1,
            "bare options were spaced apart"
        );
    }

    /// The detail is drawn dimmed, so the eye lands on the options rather than the explanations.
    #[test]
    fn a_detail_is_drawn_dimmer_than_its_label() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        let question = prompt(false);
        let asking = one(&question);
        let mut picker = Picker::new(&asking);
        terminal.draw(|frame| picker.draw(frame)).expect("drawn");
        let buffer = terminal.backend().buffer().clone();

        let rows = screen_rows(&question, 80, 24);
        let y = row_holding(&rows, "in front of the handler") as u16;
        let x = column_of(&rows[y as usize], "in front of the handler") as u16;
        assert_eq!(buffer[(x, y)].symbol(), "i", "not the start of the detail");
        assert_eq!(buffer[(x, y)].fg, theme::muted());
    }

    /// A tall option list still has to leave room for the keys. If the height of the details were
    /// not counted, the last options would be drawn over the line that says how to answer.
    #[test]
    fn a_detailed_list_still_leaves_room_for_the_keys() {
        let many = bravebot_core::ask::prompt(&Question::new(
            "File",
            "Which file?",
            (0..40)
                .map(|i| Choice::new(format!("file-{i}.rs"), Some(format!("the {i}th one"))))
                .collect(),
            false,
        ));
        let rows = screen_rows(&many, 80, 24);
        assert!(
            rows.iter().any(|row| row.contains("esc")),
            "the keys were pushed off the box by the option list"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("more, use the arrow keys")),
            "the list was cut short without saying so"
        );
    }
    /// The hint line must fit the box it is drawn in. Wrapped onto a second line it reads as two
    /// half-hints, and the keys are what a person scans when they do not know what to press.
    #[test]
    fn the_keys_fit_on_one_line() {
        for multiple in [false, true] {
            let rows = screen_rows(&prompt(multiple), 72, 20);
            let line = row_holding(&rows, "esc");
            assert!(
                rows[line].contains("skip"),
                "the hints wrapped onto a second line: {}",
                rows[line]
            );
        }
    }
    /// A person who opens the field must be able to see how to get out of it. Drawing the field
    /// with no keys at all leaves them with a cursor and no idea what escape does.
    #[test]
    fn the_field_shows_its_own_keys() {
        let question = one(&prompt(false));
        let mut picker = Picker::new(&question);
        picker.press(KeyCode::Char('o'));

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| picker.draw(frame)).expect("drawn");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(
            screen.contains("enter"),
            "the field does not say how to answer"
        );
        assert!(
            screen.contains("back to the options"),
            "no way back is offered"
        );
    }

    /// And where there were no options to go back to, escape leaves the question, so the hint has
    /// to say that instead of offering a list that is not there.
    #[test]
    fn a_field_with_no_options_behind_it_offers_to_skip() {
        let bare = one(&bravebot_core::ask::prompt(&Question::new(
            "Branch",
            "Which branch?",
            Vec::new(),
            false,
        )));
        let mut picker = Picker::new(&bare);

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| picker.draw(frame)).expect("drawn");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(screen.contains("skip the question"));
        assert!(!screen.contains("back to the options"));
    }
}
