use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Stylize as _;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph};

use super::app::{App, Focus};

const HINTS: &str = "q quit  r refresh  s start  x stop  n shell  Enter attach  k kill";

pub fn draw(frame: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(rows[0]);

    draw_servers(frame, app, panes[0]);
    draw_shells(frame, app, panes[1]);
    draw_statusbar(frame, app, rows[1]);
    // Task 6 overlays an open dialog here.
}

fn pane_block(title: String, focused: bool) -> Block<'static> {
    let border = if focused {
        BorderType::Thick
    } else {
        BorderType::Plain
    };
    Block::new()
        .borders(Borders::ALL)
        .border_type(border)
        .title(title)
}

fn draw_servers(frame: &mut Frame, app: &App, area: Rect) {
    let username = app.username.as_deref().unwrap_or("...");
    let mut title = format!("Servers @ {} ({username})", app.hub_name);
    if app.loading {
        title.push_str(" (refreshing)");
    }
    let items: Vec<ListItem> = app
        .servers
        .iter()
        .enumerate()
        .map(|(index, server)| {
            let state = if server.ready {
                "ready"
            } else if let Some(pending) = &server.pending {
                pending.as_str()
            } else {
                "stopped"
            };
            let options = if server.options.is_empty() {
                String::new()
            } else {
                let rendered: Vec<String> = server
                    .options
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                format!("  {}", rendered.join(" "))
            };
            let line = Line::from(format!("{:<12} {state:<9}{options}", server.display));
            let selected = index == app.selected_server;
            if selected {
                ListItem::new(line.reversed())
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let list = List::new(items).block(pane_block(title, app.focus == Focus::Servers));
    frame.render_widget(list, area);
}

fn draw_shells(frame: &mut Frame, app: &App, area: Rect) {
    let server = app
        .selected_server()
        .map(|s| s.display.clone())
        .unwrap_or_else(|| "-".to_string());
    let title = format!("Shells on {server}");
    let items: Vec<ListItem> = if app.shells.is_empty() {
        vec![ListItem::new(Line::from(
            "no shells; press n to create one",
        ))]
    } else {
        app.shells
            .iter()
            .enumerate()
            .map(|(index, shell)| {
                let activity = shell.last_activity.as_deref().unwrap_or("-");
                let line = Line::from(format!("{:<8} {activity}", shell.name));
                if index == app.selected_shell && app.focus == Focus::Shells {
                    ListItem::new(line.reversed())
                } else {
                    ListItem::new(line)
                }
            })
            .collect()
    };
    let list = List::new(items).block(pane_block(title, app.focus == Focus::Shells));
    frame.render_widget(list, area);
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
    use crate::tui::app::{App, AppEvent, ServerRow, ShellRow};
    use std::time::Instant;

    fn app_with_state() -> App {
        let mut app = App::new("icrn".to_string(), Default::default());
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                username: "ww41".to_string(),
                servers: vec![
                    ServerRow {
                        name: String::new(),
                        display: "default".to_string(),
                        ready: true,
                        pending: None,
                        options: serde_json::from_str(r#"{"resource": "2_a100"}"#).unwrap(),
                        url: Some("/user/ww41/".to_string()),
                    },
                    ServerRow {
                        name: "backup".to_string(),
                        display: "backup".to_string(),
                        ready: false,
                        pending: Some("spawn".to_string()),
                        options: Default::default(),
                        url: None,
                    },
                ],
            },
            Instant::now(),
        );
        app.apply(
            AppEvent::Shells {
                server: "default".to_string(),
                shells: vec![ShellRow {
                    name: "2".to_string(),
                    last_activity: Some("2026-07-05T00:00:00Z".to_string()),
                }],
            },
            Instant::now(),
        );
        app
    }

    fn rendered(app: &App) -> String {
        let backend = ratatui::backend::TestBackend::new(100, 14);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        buffer_text(&terminal)
    }

    #[test]
    fn dashboard_shows_servers_shells_and_hints() {
        let app = app_with_state();
        let text = rendered(&app);
        assert!(
            text.contains("Servers @ icrn (ww41)"),
            "buffer was:\n{text}"
        );
        assert!(text.contains("default"));
        assert!(text.contains("ready"));
        assert!(text.contains("backup"));
        assert!(text.contains("spawn"));
        assert!(text.contains("Shells on default"));
        assert!(text.contains("JupyterCLI v"));
        assert!(text.contains("Enter attach"));
    }

    #[test]
    fn info_status_replaces_hints_error_takes_the_bar() {
        let mut app = app_with_state();
        let now = Instant::now();
        app.set_status("server ready".to_string(), false, now);
        let text = rendered(&app);
        assert!(text.contains("server ready"));
        assert!(!text.contains("Enter attach"));

        app.set_status("token lacks the required scope".to_string(), true, now);
        let text = rendered(&app);
        assert!(text.contains("token lacks the required scope"));
        assert!(
            !text.contains("JupyterCLI v"),
            "error status must take over the whole bar"
        );
    }

    #[test]
    fn loading_flag_shows_in_title() {
        let mut app = app_with_state();
        app.loading = true;
        assert!(rendered(&app).contains("(refreshing)"));
    }
}
