#[test]
fn a_default_terminal_size_renders_content() {
    use bravebot_tui::render;
    use bravebot_tui::state::Session;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let session = Session::new("kernel-enforced");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal
        .draw(|f| {
            render::draw(f, &session);
        })
        .expect("draw");
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        text.contains("bravebot"),
        "status bar missing: {:?}",
        &text[..200.min(text.len())]
    );
    assert!(text.contains("Ask a question"), "hint missing");
}
