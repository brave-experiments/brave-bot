//! Choosing which model to think with.
//!
//! Shown by `/model`. The list comes from the endpoints rather than from a set compiled in, so it
//! is whatever the backends actually offer today, and the choice is written to `~/.bravebot`: it
//! outlives the session that made it and applies in every directory.
//!
//! Drawn as a centred panel over the session, the shape every other question here takes. Typing
//! narrows the list, and the rows are grouped under the service that answers them, because a
//! roster is now several rosters at once: Brave's, a Bedrock account's, and whatever a gateway
//! serves, which on its own runs to hundreds of names.
//!
//! Nothing labelled is involved and nothing is quarantined. The names never reach a model: they
//! are drawn for a person, who picks one, and what they picked becomes the `model` field of later
//! requests. That field is routing, and a person choosing it off a list they read is the
//! endorsement for it, the same way a person approving a write destination is.

use bravebot_aichat::models::Model;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::theme;
use crate::wrap::display_width;
use bravebot_i18n::t;
use unicode_width::UnicodeWidthChar;

/// What the picker is showing and where the cursor is.
#[derive(Debug)]
pub struct Picker {
    /// Everything on offer, in the order the roster was composed in.
    models: Vec<Model>,
    /// What has been typed to narrow it.
    search: String,
    /// Which of the matching models is under the cursor.
    selected: usize,
    /// The one in use when the picker opened, so it can be marked as current.
    current: Option<String>,
}

impl Picker {
    /// Open on the model in use, since that is the row a user is looking for.
    pub fn new(models: Vec<Model>, current: Option<&str>) -> Self {
        let mut picker = Self {
            models,
            search: String::new(),
            selected: 0,
            current: current.map(str::to_string),
        };
        let selected = picker
            .matching()
            .iter()
            .position(|model| picker.is_current(model))
            .unwrap_or(0);
        picker.selected = selected;
        picker
    }

    /// The models matching what has been typed, grouped by the service that answers them.
    ///
    /// Matched without regard to case and anywhere in the row, because a model is remembered by a
    /// word out of the middle of its name, and by the service it is reached through as readily as
    /// by the name itself. Every word typed has to match something, so a second word narrows.
    pub fn matching(&self) -> Vec<&Model> {
        let terms: Vec<String> = self
            .search
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let order = self.services();
        let mut hits: Vec<&Model> = self
            .models
            .iter()
            .filter(|model| matches(model, &terms))
            .collect();
        // Stable, so the order a service listed its models in survives the grouping: that order is
        // the backend's own preference, or the order somebody wrote them down in.
        hits.sort_by_key(|model| {
            order
                .iter()
                .position(|service| *service == service_of(model))
                .unwrap_or(usize::MAX)
        });
        hits
    }

    /// Every service in the roster, in the order it is first mentioned.
    ///
    /// First mention rather than alphabetical, because the roster is already composed in the order
    /// somebody should meet it: what a settings file went out of its way to name comes before the
    /// roster everybody has.
    fn services(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for model in &self.models {
            let service = service_of(model);
            if !seen.contains(&service) {
                seen.push(service);
            }
        }
        seen
    }

    /// The model under the cursor.
    pub fn chosen(&self) -> Option<&Model> {
        self.matching().get(self.selected).copied()
    }

    fn down(&mut self) {
        let last = self.matching().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn typed(&mut self, c: char) {
        self.search.push(c);
        self.follow();
    }

    fn backspace(&mut self) {
        self.search.pop();
        self.follow();
    }

    /// Keep the cursor on the model it was on, and fall to the first match where that model no
    /// longer matches.
    ///
    /// A cursor that stayed at its index would jump to an unrelated model on every keystroke, and
    /// one that reset to the top would lose the row somebody had already found by scrolling.
    fn follow(&mut self) {
        let was = self.chosen().map(|model| model.key.clone());
        let found = self
            .matching()
            .iter()
            .position(|model| Some(&model.key) == was.as_ref());
        self.selected = found.unwrap_or(0);
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

/// The service a row is filed under.
///
/// Brave's own roster names no service of its own, and is named here rather than left blank: a list
/// whose other sections say who answers them reads as though the unlabelled rows came from nowhere.
fn service_of(model: &Model) -> &str {
    model
        .provider
        .as_deref()
        .unwrap_or(t!(picker_service_brave))
}

/// Whether every word typed appears somewhere in the row.
///
/// The key as well as the name, because a gateway row is picked by a slug somebody read in another
/// tool, and the service, because "the one on openrouter" is how a person holds it.
fn matches(model: &Model, terms: &[String]) -> bool {
    let haystack =
        format!("{} {} {}", model.display_name, model.key, service_of(model)).to_lowercase();
    terms.iter().all(|term| haystack.contains(term))
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
        // A letter narrows the list rather than walking it: with a gateway roster on offer the list
        // is hundreds of rows, and no arrangement of them makes arrowing to one reasonable.
        KeyCode::Char(c) => {
            picker.typed(c);
            Outcome::Continue
        }
        _ => Outcome::Continue,
    }
}

/// Show the list over whatever `behind` draws, and return the model the user picked, or `None` if
/// they kept the current one.
///
/// `behind` is the session. Drawn first each frame so the panel sits over the transcript somebody
/// opened it from, rather than replacing it with a page that looks like a different program.
pub fn choose<B: Backend>(
    terminal: &mut Terminal<B>,
    models: Vec<Model>,
    current: Option<&str>,
    mut behind: impl FnMut(&mut Frame),
) -> Option<Model> {
    let mut picker = Picker::new(models, current);

    loop {
        // A terminal that cannot be drawn to cannot carry the question, so nothing is changed.
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

/// Draw the panel.
fn draw(frame: &mut Frame, picker: &Picker) {
    let area = centred(frame.area(), picker.rows_needed());
    frame.render_widget(Clear, area);

    // The theme's own colours rather than the terminal's. `Clear` only empties the cells, so without
    // this the panel is drawn on whatever the terminal's default background is and a theme chosen
    // for readability stops applying at the moment somebody opens a picker.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::brand_primary()))
        .title(Span::styled(
            format!(" {} ", t!(model_picker_heading)),
            Style::default()
                .fg(theme::brand_primary())
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(theme::background()).fg(theme::text()));
    let inside = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // search
            Constraint::Length(1), // blank
            Constraint::Min(1),    // list
            Constraint::Length(1), // blank
            Constraint::Length(1), // keys
        ])
        .split(inside);

    let search = match picker.search.is_empty() {
        true => Span::styled(
            t!(model_picker_search_placeholder),
            Style::default().fg(theme::muted()),
        ),
        false => Span::styled(picker.search.clone(), Style::default().fg(theme::text())),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::raw(" "), search])),
        layout[0],
    );
    // The terminal's own cursor, so the box a person is typing into is the one blinking, and so it
    // is where a screen reader and a terminal's own selection put it.
    let caret = layout[0].x + 1 + display_width(&picker.search) as u16;
    frame.set_cursor_position(Position::new(
        caret.min(layout[0].right().saturating_sub(1)),
        layout[0].y,
    ));

    frame.render_widget(Paragraph::new(list_lines(picker, layout[2])), layout[2]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", t!(model_picker_keys)),
            Style::default().fg(theme::muted()),
        ))),
        layout[4],
    );
}

/// One drawn row of the list.
enum Row<'a> {
    /// A heading naming the service that answers everything under it.
    Service(&'a str),
    /// A model, and where it sits among the matching ones.
    Model(usize, &'a Model),
    /// The space between one service and the next.
    Gap,
}

impl Picker {
    /// How many rows the list would take if nothing had to scroll, for sizing the panel.
    fn rows_needed(&self) -> usize {
        rows(&self.matching()).len()
    }
}

/// The rows the matching models make: a heading, its models, a gap, the next heading.
fn rows<'a>(matching: &[&'a Model]) -> Vec<Row<'a>> {
    let mut rows = Vec::new();
    let mut service: Option<&str> = None;
    for (index, model) in matching.iter().enumerate() {
        let this = service_of(model);
        if service != Some(this) {
            if service.is_some() {
                rows.push(Row::Gap);
            }
            rows.push(Row::Service(this));
            service = Some(this);
        }
        rows.push(Row::Model(index, model));
    }
    rows
}

/// The list itself, and a word when the search matches nothing.
fn list_lines(picker: &Picker, area: Rect) -> Vec<Line<'static>> {
    let matching = picker.matching();
    if matching.is_empty() {
        return vec![Line::from(Span::styled(
            format!(" {}", t!(model_picker_nothing_matches)),
            Style::default().fg(theme::muted()),
        ))];
    }

    let rows = rows(&matching);
    let visible = (area.height as usize).max(1);
    let cursor = rows
        .iter()
        .position(|row| matches!(row, Row::Model(index, _) if *index == picker.selected))
        .unwrap_or(0);
    let (heading, first, shown) = window(&rows, cursor, visible);

    let mut lines: Vec<Line<'static>> = heading.map(service_line).into_iter().collect();
    lines.extend(rows.iter().skip(first).take(shown).map(|row| match row {
        Row::Service(service) => service_line(service),
        Row::Gap => Line::raw(""),
        Row::Model(index, model) => model_line(
            model,
            *index == picker.selected,
            picker.is_current(model),
            area.width as usize,
        ),
    }));
    lines
}

/// The heading to hold at the top, where the window starts, and how many of its rows are left.
///
/// A list scrolled into a gateway's roster has left that gateway's heading far above the screen,
/// and rows that no longer say whose they are is the thing sections were drawn for. So the heading
/// the cursor sits under is kept on the top line, at the cost of one row of the list, and only for
/// as long as the real one is off screen.
fn window<'a>(rows: &[Row<'a>], cursor: usize, visible: usize) -> (Option<&'a str>, usize, usize) {
    let first = start(rows.len(), cursor, visible);
    // Nothing to hold a heading over on a list two rows tall: the row under the cursor is what a
    // person is there to read, and a heading that displaced it would leave the panel saying only
    // whose models these are and never which.
    if visible <= 2 || matches!(rows.get(first), Some(Row::Service(_))) {
        return (None, first, visible);
    }

    let heading = rows[..=cursor.min(rows.len().saturating_sub(1))]
        .iter()
        .rev()
        .find_map(|row| match row {
            Row::Service(service) => Some(*service),
            _ => None,
        });
    let shown = visible.saturating_sub(1).max(1);
    (heading, start(rows.len(), cursor, shown), shown)
}

/// Where a window of `visible` rows starts with the cursor inside it.
fn start(rows: usize, cursor: usize, visible: usize) -> usize {
    if rows <= visible {
        return 0;
    }
    cursor
        .saturating_sub(visible.saturating_sub(1))
        .min(rows - visible)
}

/// A service heading: who answers for everything drawn under it.
fn service_line(service: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {service}"),
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD),
    ))
}

/// One model's row: a mark for the one in use, the name, and what the row costs on the right.
fn model_line(model: &Model, selected: bool, current: bool, width: usize) -> Line<'static> {
    // Filled edge to edge rather than only under the words, since a bar that stopped at the end of
    // a short name would read as a highlight over the text rather than as the row being chosen.
    let (name, mark, tag) = match selected {
        true => {
            let on_bar = Style::default().fg(theme::on_primary());
            (
                on_bar.add_modifier(Modifier::BOLD),
                on_bar,
                on_bar.add_modifier(Modifier::BOLD),
            )
        }
        false => (
            Style::default().fg(theme::text()),
            Style::default().fg(theme::ok()),
            Style::default().fg(theme::running()),
        ),
    };

    // Said rather than left to be discovered by a request failing: without a subscription a premium
    // model returns 403 and the session looks broken for no stated reason.
    let label = match model.premium {
        true => format!("{}  ", t!(picker_premium)),
        false => String::new(),
    };
    // The mark rather than a word, because the row a person is looking for is found by scanning a
    // column, and a word at the end of a name is not in one.
    let marker = match current {
        true => " ● ",
        false => "   ",
    };

    let room = width.saturating_sub(display_width(marker) + display_width(&label));
    let shown = clipped(&model.display_name, room);
    let padding = " ".repeat(room.saturating_sub(display_width(&shown)));

    let line = Line::from(vec![
        Span::styled(marker.to_string(), mark),
        Span::styled(shown, name),
        Span::raw(padding),
        Span::styled(label, tag),
    ]);
    match selected {
        true => line.style(Style::default().bg(theme::brand_primary())),
        false => line,
    }
}

/// A name cut to the room there is for it, with an ellipsis where it was cut.
///
/// The beginning is what is kept: a model name is told apart by its first words, and a gateway slug
/// begins with the upstream that serves it.
fn clipped(name: &str, room: usize) -> String {
    if display_width(name) <= room {
        return name.to_string();
    }
    let mut kept = String::new();
    for c in name.chars() {
        if display_width(&kept) + c.width().unwrap_or(0) + 1 > room {
            break;
        }
        kept.push(c);
    }
    kept.push('…');
    kept
}

/// A centred panel sized to the list, never larger than the terminal.
///
/// Wider than the theme picker, since a gateway slug is a good deal longer than a theme's name, and
/// tall enough to show the list where the terminal has the room.
fn centred(area: Rect, rows: usize) -> Rect {
    let available = area.width.saturating_sub(4);
    let width = available.min(76).max(available.min(28));
    // Borders, the search box and the blank under it, and the key line under its own blank.
    let outside = area.height.saturating_sub(2).max(1);
    let height = (rows as u16)
        .saturating_add(6)
        .min(outside)
        .max(outside.min(8));

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
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
            provider: None,
            conversation_tokens: None,
        }
    }

    fn served_by(service: &str, key: &str, display: &str) -> Model {
        Model {
            provider: Some(service.to_string()),
            ..model(key, display, false)
        }
    }

    fn offered() -> Vec<Model> {
        vec![
            Model::automatic(),
            model("claude-3-sonnet", "Claude 4 Sonnet", true),
            model("llama-3-8b-instruct", "Llama 3 8B", false),
        ]
    }

    /// A roster of the shape a configured session has: a gateway's models, then the ones every
    /// build can reach.
    fn from_two_services() -> Vec<Model> {
        vec![
            served_by("OpenRouter", "openrouter/z-ai/glm-4.6", "z-ai/glm-4.6"),
            served_by(
                "OpenRouter",
                "openrouter/moonshot/kimi-k2",
                "moonshot/kimi-k2",
            ),
            Model::automatic(),
            model("claude-3-sonnet", "Claude 4 Sonnet", true),
        ]
    }

    fn typing(picker: &mut Picker, text: &str) {
        for c in text.chars() {
            handle_key(picker, KeyCode::Char(c), KeyModifiers::NONE);
        }
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

    /// A gateway serves hundreds of models. Arrowing to one is not a way to choose, so a letter
    /// narrows the list instead of walking it.
    #[test]
    fn typing_narrows_the_list_to_the_models_that_match() {
        let mut picker = Picker::new(from_two_services(), None);
        typing(&mut picker, "kimi");

        let matching: Vec<&str> = picker
            .matching()
            .iter()
            .map(|model| model.display_name.as_str())
            .collect();
        assert_eq!(matching, ["moonshot/kimi-k2"]);
        assert_eq!(
            picker.chosen().expect("a model").display_name,
            "moonshot/kimi-k2"
        );
    }

    /// A model is looked for by any part of what is on the row, and by the service it is reached
    /// through: "the sonnet on openrouter" is how somebody holds a roster of several services.
    #[test]
    fn a_search_matches_the_service_and_the_requestable_name_too() {
        let mut picker = Picker::new(from_two_services(), None);
        typing(&mut picker, "openrouter glm");

        let matching: Vec<&str> = picker
            .matching()
            .iter()
            .map(|model| model.key.as_str())
            .collect();
        assert_eq!(matching, ["openrouter/z-ai/glm-4.6"]);
    }

    /// Case is not how a person remembers a name, and a roster mixes both.
    #[test]
    fn a_search_ignores_case() {
        let mut picker = Picker::new(offered(), None);
        typing(&mut picker, "SONNET");
        assert_eq!(picker.matching().len(), 1);
    }

    #[test]
    fn backspace_widens_the_list_again() {
        let mut picker = Picker::new(from_two_services(), None);
        typing(&mut picker, "kimi");
        for _ in 0..4 {
            handle_key(&mut picker, KeyCode::Backspace, KeyModifiers::NONE);
        }
        assert_eq!(picker.matching().len(), from_two_services().len());
    }

    /// A cursor that stayed at its index would land on an unrelated model with every keystroke,
    /// which is worst exactly where the list is long enough to need searching.
    #[test]
    fn the_cursor_stays_on_the_model_it_was_on_while_the_search_narrows() {
        let mut picker = Picker::new(from_two_services(), None);
        handle_key(&mut picker, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(
            picker.chosen().expect("a model").display_name,
            "moonshot/kimi-k2"
        );

        typing(&mut picker, "moonshot");
        assert_eq!(
            picker.chosen().expect("a model").display_name,
            "moonshot/kimi-k2"
        );
    }

    /// Where what was under the cursor no longer matches, the first thing that does is the one a
    /// person is looking at, and Enter has to select that rather than nothing.
    #[test]
    fn the_cursor_falls_to_the_first_match_when_what_it_was_on_is_filtered_out() {
        let mut picker = Picker::new(from_two_services(), None);
        typing(&mut picker, "glm");
        assert_eq!(
            picker.chosen().expect("a model").display_name,
            "z-ai/glm-4.6"
        );
    }

    /// Enter on a search that matches nothing must not choose whatever was under the cursor
    /// before: the row it would pick is not on the screen.
    #[test]
    fn a_search_matching_nothing_leaves_nothing_to_choose() {
        let mut picker = Picker::new(offered(), None);
        typing(&mut picker, "nothing named this");
        assert!(picker.chosen().is_none());
    }

    fn rendered_at(picker: &Picker, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
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

    fn rendered(picker: &Picker) -> String {
        rendered_at(picker, 60, 20)
    }

    /// A premium model returns 403 without a subscription, so which rows those are has to be on the
    /// screen rather than discovered by a request failing.
    #[test]
    fn a_premium_model_says_so() {
        let output = rendered(&Picker::new(offered(), None));
        assert!(output.contains(t!(picker_premium)), "{output}");
    }

    /// Which one is in use answers the question a user opened the picker to ask. A mark rather than
    /// a word, so it is found by scanning one column of a list that may be hundreds of rows.
    #[test]
    fn the_model_in_use_is_marked() {
        let output = rendered(&Picker::new(offered(), Some("claude-3-sonnet")));
        assert!(output.contains("● Claude 4 Sonnet"), "{output}");
        assert!(!output.contains("● Llama 3 8B"), "{output}");
    }

    /// The list is drawn from the display names, never the keys: a key is for a request field and
    /// says nothing to a person choosing.
    #[test]
    fn the_list_shows_names_a_person_reads() {
        let output = rendered(&Picker::new(offered(), None));
        assert!(output.contains("Automatic"), "{output}");
        assert!(output.contains("Llama 3 8B"), "{output}");
        assert!(!output.contains("llama-3-8b-instruct"), "{output}");
    }

    /// The same slug is reachable through more than one service, billed and credentialled
    /// differently, so a list of several rosters has to say which rows are whose.
    #[test]
    fn every_service_heads_its_own_section() {
        let output = rendered(&Picker::new(from_two_services(), None));
        assert!(output.contains("OpenRouter"), "{output}");
        assert!(output.contains(t!(picker_service_brave)), "{output}");
        // The gateway said once, over its rows, rather than repeated on each of them.
        assert_eq!(output.matches("OpenRouter").count(), 1, "{output}");
    }

    /// A section is one run of rows: a roster arriving interleaved would otherwise head the same
    /// service twice and leave a person unsure which of the two headings theirs was under.
    #[test]
    fn a_service_that_appears_twice_in_the_roster_is_still_one_section() {
        let interleaved = vec![
            served_by("OpenRouter", "openrouter/one", "one"),
            model("claude-3-sonnet", "Claude 4 Sonnet", false),
            served_by("OpenRouter", "openrouter/two", "two"),
        ];
        let picker = Picker::new(interleaved, None);
        let shown: Vec<&str> = picker
            .matching()
            .iter()
            .map(|model| model.display_name.as_str())
            .collect();
        assert_eq!(shown, ["one", "two", "Claude 4 Sonnet"]);
    }

    /// Scrolled into the middle of a long roster, the rows on screen still have to say whose they
    /// are, or the heading is only ever visible for the models at the top of a service.
    #[test]
    fn a_heading_stays_above_the_rows_when_the_list_is_scrolled() {
        let mut roster = vec![Model::automatic()];
        for index in 0..40 {
            roster.push(served_by(
                "OpenRouter",
                &format!("openrouter/model-{index}"),
                &format!("model-{index}"),
            ));
        }
        let mut picker = Picker::new(roster, None);
        for _ in 0..30 {
            handle_key(&mut picker, KeyCode::Down, KeyModifiers::NONE);
        }

        let output = rendered_at(&picker, 60, 14);
        let under_the_cursor = picker.chosen().expect("a model").display_name.clone();
        assert!(
            output.contains(&under_the_cursor),
            "the cursor is off screen: {output}"
        );
        assert!(output.contains("OpenRouter"), "{output}");
    }

    /// Nothing on screen and no word for it reads as a picker that has broken, rather than as a
    /// search that is too narrow.
    #[test]
    fn a_search_matching_nothing_says_so() {
        let mut picker = Picker::new(offered(), None);
        typing(&mut picker, "zzz");
        let output = rendered(&picker);
        assert!(
            output.contains(t!(model_picker_nothing_matches)),
            "{output}"
        );
    }

    /// A picker that covered the screen would replace the session with a page that looks like a
    /// different program. The same centred panel every other question here takes.
    #[test]
    fn the_picker_is_drawn_as_a_centred_panel() {
        let picker = Picker::new(offered(), None);
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("terminal");
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new("the session behind it"),
                    Rect::new(0, 0, 60, 1),
                );
                draw(frame, &picker);
            })
            .expect("draw succeeds");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("the session behind it"), "{screen}");
    }

    /// A panel that grew past the frame would draw nothing at all on a terminal somebody has
    /// squeezed into a corner.
    #[test]
    fn the_panel_stays_inside_a_tiny_terminal() {
        let picker = Picker::new(offered(), None);
        let area = Rect::new(0, 0, 20, 6);
        let panel = centred(area, picker.rows_needed());
        assert!(panel.width <= area.width, "{panel:?}");
        assert!(panel.height <= area.height, "{panel:?}");
        // Drawn rather than only measured, since a panel wider than the frame panics on render.
        rendered_at(&picker, 20, 6);
    }

    /// A name too long for the panel is cut where it runs out of room. Cutting the front would
    /// take the upstream off a gateway slug, which is the half that says what the model is.
    #[test]
    fn a_name_too_long_for_the_panel_keeps_its_beginning() {
        let long = model(
            "some/very-long-vendor/model-name-that-will-not-fit",
            "some/very-long-vendor/model-name-that-will-not-fit",
            false,
        );
        let output = rendered_at(&Picker::new(vec![long], None), 30, 12);
        assert!(output.contains("some/very-long"), "{output}");
        assert!(output.contains('…'), "{output}");
    }
}
