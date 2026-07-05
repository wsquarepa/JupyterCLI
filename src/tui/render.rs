use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::app::{App, Focus, ServerRow, TerminalRow};
use super::{grid, theme};

pub fn draw(frame: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());
    let main = rows[0];
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(grid::server_pane_width(main.width)),
            Constraint::Min(0),
        ])
        .split(main);

    draw_servers(frame, app, panes[0]);
    if app.peek_visible() {
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(grid::peek_height(main.height)),
            ])
            .split(panes[1]);
        draw_grid(frame, app, right[0]);
        draw_peek(frame, app, right[1]);
    } else {
        draw_grid(frame, app, panes[1]);
    }
    draw_statusbar(frame, app, rows[1]);
    if let Some(dialog) = &app.dialog {
        super::dialogs::render_dialog(frame, dialog);
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(theme::BORDER_FOCUSED)
    } else {
        Style::default().fg(theme::BORDER_UNFOCUSED)
    }
}

fn state_color(server: &ServerRow) -> ratatui::style::Color {
    if server.ready {
        theme::STATE_READY
    } else if server.pending.is_some() {
        theme::STATE_PENDING
    } else {
        theme::STATE_STOPPED
    }
}

fn draw_servers(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Servers;
    let mut block = Block::new()
        .borders(Borders::ALL)
        .title(" Servers ")
        .border_style(border_style(focused));
    if focused {
        block = block.title_bottom(Line::from(" n: new  x: stop ").centered());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = usize::from(inner.height);
    if visible == 0 {
        return;
    }
    let offset = app.server_cursor.saturating_sub(visible.saturating_sub(1));
    let lines: Vec<Line> = app
        .servers
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible)
        .map(|(index, server)| server_line(app, index, server, inner.width))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn server_line(app: &App, index: usize, server: &ServerRow, width: u16) -> Line<'static> {
    let committed = app.committed_server.as_deref() == Some(server.display.as_str());
    let marker = if committed { "> " } else { "  " };
    let pad_len = usize::from(width)
        .saturating_sub(marker.chars().count())
        .saturating_sub(server.display.chars().count());
    if index == app.server_cursor {
        let selection = Style::default()
            .fg(theme::SELECTION_FG)
            .bg(theme::SELECTION_BG);
        Line::from(Span::styled(
            format!("{marker}{}{}", server.display, " ".repeat(pad_len)),
            selection,
        ))
    } else {
        Line::from(vec![
            Span::styled(
                marker.to_string(),
                Style::default().fg(theme::BORDER_FOCUSED),
            ),
            Span::styled(
                server.display.clone(),
                Style::default().fg(state_color(server)),
            ),
        ])
    }
}

fn draw_grid(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Grid;
    let mut block = Block::new()
        .borders(Borders::ALL)
        .border_style(border_style(focused));
    block = match app.committed_row() {
        Some(server) => {
            let mut titled = block.title(format!(
                " Terminals on {} ({}) ",
                server.display,
                app.terminals.len()
            ));
            if !server.options.is_empty() {
                let rendered: Vec<String> = server
                    .options
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                titled = titled.title_top(
                    Line::from(Span::styled(
                        format!(" {} ", rendered.join(" ")),
                        Style::default().fg(theme::DIMMED),
                    ))
                    .right_aligned(),
                );
            }
            titled
        }
        None => block.title(" Terminals "),
    };
    if focused {
        block = block
            .title_bottom(Line::from(" Enter: attach  n: new  x: kill  Esc: back ").centered());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.committed_server.is_none() {
        return;
    }
    let cards = app.displayed_terminals();
    if cards.is_empty() {
        let hint = Paragraph::new(
            Line::from(Span::styled(
                "no terminals; press n to create one",
                Style::default().fg(theme::DIMMED),
            ))
            .centered(),
        );
        let y = inner.y + inner.height / 2;
        if inner.height > 0 {
            frame.render_widget(
                hint,
                Rect::new(inner.x, y.min(inner.bottom() - 1), inner.width, 1),
            );
        }
        return;
    }

    let columns = grid::columns_for_width(inner.width);
    let offsets = grid::row_offsets(inner.width, columns);
    let visible_rows = grid::visible_card_rows(inner.height);
    for (index, terminal) in cards.iter().enumerate() {
        let row = index / columns;
        if row < app.grid_scroll || row >= app.grid_scroll + visible_rows {
            continue;
        }
        let col = index % columns;
        let x = inner.x + offsets[col];
        let y = inner.y + ((row - app.grid_scroll) as u16) * (grid::CARD_HEIGHT + grid::CARD_VGAP);
        let rect = Rect::new(x, y, grid::CARD_WIDTH, grid::CARD_HEIGHT);
        draw_card(
            frame,
            terminal,
            focused && index == app.grid_cursor,
            rect,
            inner,
        );
    }

    if app.terminals.len() > grid::DISPLAY_CAP && inner.height >= 2 {
        let bottom = Rect::new(inner.x, inner.bottom() - 2, inner.width, 2);
        let notice = Paragraph::new(vec![
            Line::from("More terminals exist, but cannot be displayed.").centered(),
            Line::from("Use the CLI to manage them instead.").centered(),
        ])
        .style(Style::default().fg(theme::DIMMED));
        frame.render_widget(Clear, bottom);
        frame.render_widget(notice, bottom);
    }
}

fn draw_card(frame: &mut Frame, terminal: &TerminalRow, hovered: bool, rect: Rect, bounds: Rect) {
    // Geometry comes from the same grid module App uses, but a resize between
    // App math and this frame could overflow; skip rather than clip garbage.
    if rect.right() > bounds.right() || rect.bottom() > bounds.bottom() {
        return;
    }
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(border_style(hovered));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines = vec![
        Line::from(">_"),
        Line::from(""),
        Line::from(grid::card_label(&terminal.name)).centered(),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_peek(frame: &mut Frame, app: &App, area: Rect) {
    let name = app
        .peek
        .as_ref()
        .map(|p| p.terminal.clone())
        .or_else(|| app.hover.as_ref().map(|h| h.terminal.clone()));
    let title = match name {
        Some(name) => format!(" Peek: {} ", grid::card_label(&name)),
        None => " Peek ".to_string(),
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style(false));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(peek) = &app.peek else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "connecting...",
                Style::default().fg(theme::DIMMED),
            )),
            inner,
        );
        return;
    };
    if let Some(error) = &peek.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                error.clone(),
                Style::default().fg(theme::STATUS_ERROR_BG),
            )),
            inner,
        );
        return;
    }
    if !peek.connected {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "connecting...",
                Style::default().fg(theme::DIMMED),
            )),
            inner,
        );
        return;
    }
    // Tail-anchor the emulated screen: show the window ending at the last
    // painted row so a shell prompt (bottom of the screen) stays visible while
    // a full-screen app's most recent region shows through.
    let rows: Vec<String> = peek.parser.screen().rows(0, 400).collect();
    let Some(last) = rows.iter().rposition(|r| r.chars().any(|c| c != ' ')) else {
        return;
    };
    let height = usize::from(inner.height);
    let start = (last + 1).saturating_sub(height);
    let width = usize::from(inner.width);
    let lines: Vec<Line> = rows[start..=last]
        .iter()
        .map(|r| Line::from(r.chars().take(width).collect::<String>()))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

const GLOBAL_HINTS: &str = "Tab: switch focus | r: refresh | q: quit ";

fn draw_statusbar(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(status) = &app.status
        && status.error
    {
        let bar = Paragraph::new(Line::from(format!(" {} ", status.text)).centered()).style(
            Style::default()
                .fg(theme::STATUS_ERROR_FG)
                .bg(theme::STATUS_ERROR_BG),
        );
        frame.render_widget(bar, area);
        return;
    }
    let bar_style = Style::default()
        .fg(theme::STATUS_BAR_FG)
        .bg(theme::STATUS_BAR_BG);
    frame.render_widget(Paragraph::new("").style(bar_style), area);

    let width = usize::from(area.width);
    let notification: Option<Span<'static>> = app.status.as_ref().map(|status| {
        let text: String = format!(" {} ", status.text).chars().take(width).collect();
        Span::styled(
            text,
            Style::default()
                .fg(theme::STATUS_INFO_FG)
                .bg(theme::STATUS_INFO_BG),
        )
    });
    let notif_width = notification
        .as_ref()
        .map(|s| s.content.chars().count())
        .unwrap_or(0);

    let mut core: Vec<Span<'static>> = Vec::new();
    if let Some((glyph, label)) = app.spinner() {
        core.push(Span::styled(
            format!("{glyph} {label}  "),
            Style::default().fg(theme::SPINNER).bg(theme::STATUS_BAR_BG),
        ));
    }
    core.push(Span::styled(GLOBAL_HINTS, bar_style));
    let mut right = trim_spans(core, width.saturating_sub(notif_width));
    if let Some(notification) = notification {
        right.push(notification);
    }
    let right_width: usize = right.iter().map(|s| s.content.chars().count()).sum();

    let left_max = width.saturating_sub(right_width).saturating_sub(1);
    let left: String = format!(
        " JupyterCLI v{}  {} ({})",
        env!("CARGO_PKG_VERSION"),
        app.hub_name,
        app.username.as_deref().unwrap_or("...")
    )
    .chars()
    .take(left_max)
    .collect();

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(left, bar_style))),
        Rect::new(area.x, area.y, left_max as u16, 1),
    );
    if right_width > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(right)),
            Rect::new(
                area.x + (width - right_width) as u16,
                area.y,
                right_width as u16,
                1,
            ),
        );
    }
}

/// Truncate a span list to `budget` columns keeping the prefix, LibLLM style:
/// the global hints lose their tail as a notification claims the right edge.
fn trim_spans(spans: Vec<Span<'static>>, budget: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let len = span.content.chars().count();
        if used + len <= budget {
            used += len;
            out.push(span);
        } else {
            let keep = budget - used;
            if keep > 0 {
                let text: String = span.content.chars().take(keep).collect();
                out.push(Span::styled(text, span.style));
            }
            break;
        }
    }
    out
}

/// Fixed-size dialog rect centered in `area`, clamped to fit.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

pub fn dialog_block(title: &str) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .title(title.to_string())
        .border_style(Style::default().fg(theme::DIALOG))
}

/// Centered hint strip in the row directly below a dialog, skipped when the
/// dialog touches the bottom edge.
pub fn render_hints_below_dialog(frame: &mut Frame, dialog: Rect, area: Rect, hints: &str) {
    if area.bottom() <= dialog.bottom() {
        return;
    }
    let hint_area = Rect::new(dialog.x, dialog.bottom(), dialog.width, 1);
    frame.render_widget(Clear, hint_area);
    frame.render_widget(
        Paragraph::new(Line::from(hints.to_string()).centered()).style(
            Style::default()
                .fg(theme::STATUS_BAR_FG)
                .bg(theme::STATUS_BAR_BG),
        ),
        hint_area,
    );
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
    use crate::tui::app::{AppEvent, ServerRow, TerminalRow};
    use std::time::Instant;

    fn server_row(name: &str, ready: bool, options: &str) -> ServerRow {
        ServerRow {
            name: name.to_string(),
            display: if name.is_empty() {
                "default".to_string()
            } else {
                name.to_string()
            },
            ready,
            pending: None,
            options: serde_json::from_str(options).unwrap(),
            url: ready.then(|| format!("/user/ww41/{name}/")),
        }
    }

    fn terminal_rows(names: &[&str]) -> Vec<TerminalRow> {
        names
            .iter()
            .map(|n| TerminalRow {
                name: (*n).to_string(),
            })
            .collect()
    }

    /// App refreshed on a 100x30 frame with a ready default server carrying
    /// spawn options, plus a stopped named server.
    fn app_with_servers() -> (App, Instant) {
        let now = Instant::now();
        let mut app = App::new("icrn".to_string(), Default::default(), 999, (100, 30));
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 1,
                username: "ww41".to_string(),
                servers: vec![
                    server_row("", true, r#"{"resource": "2_a100"}"#),
                    server_row("backup", false, "{}"),
                ],
            },
            now,
        );
        (app, now)
    }

    fn committed(names: &[&str]) -> (App, Instant) {
        let (mut app, now) = app_with_servers();
        app.on_key(
            &crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
            now,
        );
        let effects = app.take_effects();
        let op = match effects.as_slice() {
            [crate::tui::app::Effect::FetchTerminals { op, .. }] => *op,
            other => panic!("unexpected effects: {other:?}"),
        };
        app.apply(
            AppEvent::Terminals {
                op,
                server: "default".to_string(),
                terminals: terminal_rows(names),
            },
            now,
        );
        (app, now)
    }

    fn rendered_sized(app: &App, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        buffer_text(&terminal)
    }

    fn rendered(app: &App) -> String {
        rendered_sized(app, 100, 30)
    }

    #[test]
    fn no_selection_leaves_the_grid_blank() {
        let (app, _) = app_with_servers();
        let text = rendered(&app);
        assert!(text.contains(" Servers "));
        assert!(text.contains(" Terminals "));
        assert!(!text.contains("Terminal 0"));
        assert!(!text.contains("no terminals"));
    }

    #[test]
    fn committed_grid_shows_cards_titles_and_options() {
        let (app, _) = committed(&["1", "2"]);
        let text = rendered(&app);
        assert!(text.contains("Terminals on default (2)"), "buffer:\n{text}");
        assert!(text.contains("resource=\"2_a100\""), "buffer:\n{text}");
        assert!(text.contains("Terminal 001"));
        assert!(text.contains("Terminal 002"));
        assert!(text.contains(">_"));
    }

    #[test]
    fn empty_committed_grid_hints_at_n() {
        let (app, _) = committed(&[]);
        assert!(rendered(&app).contains("no terminals; press n to create one"));
    }

    #[test]
    fn overflow_notice_pins_to_the_bottom() {
        let names: Vec<String> = (1..=1005).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (app, _) = committed(&refs);
        let text = rendered(&app);
        assert!(text.contains("More terminals exist, but cannot be displayed."));
        assert!(text.contains("Use the CLI to manage them instead."));
    }

    #[test]
    fn hints_follow_focus() {
        let (app, _) = app_with_servers();
        let text = rendered(&app);
        assert!(text.contains("n: new  x: stop"), "buffer:\n{text}");
        assert!(!text.contains("Enter: attach"));

        let (app, _) = committed(&["1"]);
        let text = rendered(&app);
        assert!(text.contains("Enter: attach  n: new  x: kill  Esc: back"));
        assert!(!text.contains("Enter: open"));
        assert!(!text.contains("n: new  x: stop"));
    }

    #[test]
    fn committed_marker_shows_when_cursor_moves_away() {
        let (mut app, now) = committed(&["1"]);
        app.on_key(
            &crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
            now,
        );
        app.on_key(
            &crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Down,
                crossterm::event::KeyModifiers::NONE,
            ),
            now,
        );
        let text = rendered(&app);
        assert!(text.contains("> default"), "buffer:\n{text}");
    }

    #[test]
    fn peek_pane_shows_connecting_then_lines() {
        let (mut app, now) = committed(&["1"]);
        let text = rendered(&app);
        assert!(text.contains(" Peek: Terminal 001 "), "buffer:\n{text}");
        assert!(text.contains("connecting..."));

        app.tick(now + crate::tui::app::PEEK_DEBOUNCE);
        let _ = app.take_effects();
        let op = *app.ops.keys().next_back().unwrap();
        app.apply(
            AppEvent::PeekOpened {
                op,
                terminal: "1".to_string(),
            },
            now,
        );
        app.apply(
            AppEvent::PeekChunk {
                terminal: "1".to_string(),
                text: "$ nvidia-smi\r\nGPU 0: A100\r\n".to_string(),
            },
            now,
        );
        let text = rendered(&app);
        assert!(text.contains("GPU 0: A100"), "buffer:\n{text}");
        assert!(!text.contains("connecting..."));

        // A cursor-addressed repaint must land on separate screen rows, not
        // concatenate the way a naive line stream would.
        app.apply(
            AppEvent::PeekChunk {
                terminal: "1".to_string(),
                text: "\u{1b}[2J\u{1b}[1;1Halpha\u{1b}[2;1Hbeta".to_string(),
            },
            now,
        );
        let text = rendered(&app);
        assert!(text.contains("alpha"), "buffer:\n{text}");
        assert!(text.contains("beta"), "buffer:\n{text}");
        assert!(!text.contains("alphabeta"), "buffer:\n{text}");
    }

    #[test]
    fn peek_pane_absent_without_grid_focus() {
        let (mut app, now) = committed(&["1"]);
        app.on_key(
            &crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
            now,
        );
        assert!(!rendered(&app).contains(" Peek"));
    }

    #[test]
    fn statusbar_left_names_product_hub_and_user() {
        let (app, _) = app_with_servers();
        let text = rendered(&app);
        assert!(text.contains("JupyterCLI v"));
        assert!(text.contains("icrn (ww41)"));
        assert!(text.contains("Tab: switch focus | r: refresh | q: quit"));
    }

    #[test]
    fn notification_shifts_the_global_hints() {
        let (mut app, now) = app_with_servers();
        app.set_status("server ready".to_string(), false, now);
        let wide = rendered_sized(&app, 120, 30);
        assert!(wide.contains("server ready"));
        assert!(wide.contains("q: quit"), "wide bars fit both");
        let narrow = rendered_sized(&app, 50, 30);
        assert!(narrow.contains("server ready"), "buffer:\n{narrow}");
        assert!(
            !narrow.contains("q: quit"),
            "hints must truncate before the notification does:\n{narrow}"
        );
    }

    #[test]
    fn error_takes_over_the_whole_bar() {
        let (mut app, now) = app_with_servers();
        app.set_status("token lacks the required scope".to_string(), true, now);
        let text = rendered(&app);
        assert!(text.contains("token lacks the required scope"));
        assert!(!text.contains("JupyterCLI v"));
        assert!(!text.contains("q: quit"));
    }

    #[test]
    fn spinner_shows_while_an_op_is_in_flight() {
        let now = Instant::now();
        // A fresh App holds the initial "refreshing" op until its event lands.
        let app = App::new("icrn".to_string(), Default::default(), 999, (100, 30));
        let text = rendered(&app);
        assert!(text.contains("refreshing"), "buffer:\n{text}");
        assert!(text.contains("⠋"));

        let mut app = app;
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 1,
                username: "ww41".to_string(),
                servers: vec![],
            },
            now,
        );
        let text = rendered(&app);
        assert!(!text.contains("refreshing"));
    }
}
