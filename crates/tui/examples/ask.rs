//! Drive the question picker by hand, with no model, no network and no credentials.
//!
//! ```sh
//! cargo run -p bua-tui --example ask
//! ```
//!
//! The series below stands in for one the planner would have sent: a question with details on
//! its options, one taking several answers, and one with no options at all. What the picker
//! returns is printed through the same kernel function the tool uses, so what appears on stdout
//! afterwards is exactly the text the planner would have been given.
//!
//! Worth pressing: enter and escape on each question, space on the multiple-answer one, the
//! arrow keys down onto the free-text row, and ctrl-c to confirm it does not answer anything.

use bua_core::ask::{Choice, Question, Series};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use std::io;

fn main() -> io::Result<()> {
    let series = Series::new(vec![
        Question::new(
            "Cache layer",
            "Which cache layer should the new endpoint use?",
            vec![
                Choice::new(
                    "HTTP",
                    Some("in front of the handler, cheapest to add".into()),
                ),
                Choice::new("Query", Some("at the database, catches more".into())),
            ],
            false,
        ),
        Question::new(
            "Platforms",
            "Which platforms must this ship on?",
            vec![
                Choice::new("Linux", None),
                Choice::new("macOS", None),
                Choice::new("Windows", None),
            ],
            true,
        ),
        Question::new(
            "Branch",
            "Which branch should this target?",
            Vec::new(),
            false,
        ),
    ]);
    let asking = bua_core::ask::asking(&series);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let answers = bua_tui::ask::ask(&mut terminal, &asking);

    // Restored before anything is printed, so the report lands on the ordinary screen rather
    // than on one that is about to be torn down.
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    println!("what the planner would be told:\n");
    println!("{}", bua_core::ask::describe_series(&series, &answers));
    Ok(())
}
