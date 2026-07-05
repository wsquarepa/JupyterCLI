use ratatui::Frame;

pub fn draw_placeholder(frame: &mut Frame) {
    use ratatui::text::Line;
    use ratatui::widgets::{Block, Borders, Paragraph};
    let block = Block::new().borders(Borders::ALL).title("JupyterCLI");
    let body = Paragraph::new(vec![
        Line::from("Loading the dashboard."),
        Line::from("Press q to quit."),
    ])
    .block(block);
    frame.render_widget(body, frame.area());
}

#[cfg(test)]
pub(crate) fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = *buffer.area();
    let mut text = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_names_the_product() {
        let backend = ratatui::backend::TestBackend::new(40, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(draw_placeholder).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("JupyterCLI"), "buffer was:\n{text}");
        assert!(text.contains("Press q to quit."));
    }
}
