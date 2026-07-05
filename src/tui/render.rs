use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Stylize as _;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use super::app::{App, Focus};

const HINTS: &str = "Tab: switch focus | r: refresh | q: quit";

pub fn draw(frame: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
        .split(rows[0]);

    draw_servers(frame, app, panes[0]);
    draw_terminals(frame, app, panes[1]);
    draw_statusbar(frame, app, rows[1]);
    if let Some(dialog) = &app.dialog {
        super::dialogs::render_dialog(frame, dialog);
    }
}

fn draw_servers(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .servers
        .iter()
        .enumerate()
        .map(|(index, server)| {
            let line = Line::from(server.display.clone());
            if index == app.server_cursor {
                ListItem::new(line.reversed())
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" Servers @ {} ", app.hub_name));
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_terminals(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .displayed_terminals()
        .iter()
        .enumerate()
        .map(|(index, terminal)| {
            let line = Line::from(terminal.name.clone());
            if index == app.grid_cursor && app.focus == Focus::Grid {
                ListItem::new(line.reversed())
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let title = match &app.committed_server {
        Some(server) => format!(" Terminals on {server} "),
        None => " Terminals ".to_string(),
    };
    let block = Block::new().borders(Borders::ALL).title(title);
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_statusbar(frame: &mut Frame, app: &App, area: Rect) {
    let version = concat!("JupyterCLI v", env!("CARGO_PKG_VERSION"));
    let line = match &app.status {
        Some(status) if status.error => {
            frame.render_widget(
                Paragraph::new(Line::from(status.text.clone()).bold().reversed()),
                area,
            );
            return;
        }
        Some(status) => right_aligned(version, &status.text, area.width),
        None => right_aligned(version, HINTS, area.width),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn right_aligned(left: &str, right: &str, width: u16) -> Line<'static> {
    let pad = (width as usize)
        .saturating_sub(left.chars().count())
        .saturating_sub(right.chars().count());
    Line::from(vec![
        Span::raw(left.to_string()),
        Span::raw(" ".repeat(pad)),
        Span::raw(right.to_string()),
    ])
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
    fn stub_renders_servers_and_statusbar() {
        let app = App::new("icrn".to_string(), Default::default(), 999, (100, 14));
        let backend = ratatui::backend::TestBackend::new(100, 14);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Servers @ icrn"));
        assert!(text.contains("JupyterCLI v"));
    }
}
