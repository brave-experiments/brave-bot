//! Proves the copy path reads what was drawn rather than a reset buffer.
use bua_tui::select::Selection;
use bua_tui::{Session, render};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
fn copying_reads_what_the_screen_shows() {
    let mut session = Session::new("kernel-enforced");
    session.complete("the reply to copy".to_string(), Vec::new(), 0);

    let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("terminal");
    terminal
        .draw(|frame| render::draw(frame, &session))
        .expect("first draw");

    // The buffer the terminal will build next is reset, which is what made an earlier version of
    // this copy blanks. The frame that draws the screen is where the screen is.
    let text = {
        let completed = terminal
            .draw(|frame| render::draw(frame, &session))
            .expect("second draw");
        let mut selection = Selection::started_at(0, 0);
        selection.extend_to(11, 60);
        bua_tui::select::text(completed.buffer, &selection)
    };

    assert!(
        text.contains("the reply to copy"),
        "the screen came back empty: {text:?}"
    );
}
