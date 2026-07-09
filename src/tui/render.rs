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
    let border = if app.servers_loading() {
        Style::default().fg(super::anim::pulse_color(app.spinner_frame))
    } else {
        border_style(focused)
    };
    let mut block = Block::new()
        .borders(Borders::ALL)
        .title(" Servers ")
        .border_style(border);
    if focused {
        block = block.title_bottom(Line::from(" n: new  x: stop ").centered());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = usize::from(inner.height);
    if visible == 0 {
        return;
    }
    let total = app.servers.len() + 1;
    let offset = app.server_cursor.saturating_sub(visible.saturating_sub(1));
    let lines: Vec<Line> = (offset..total)
        .take(visible)
        .map(|index| match app.servers.get(index) {
            Some(server) => server_line(app, index, server, inner.width),
            None => synthetic_row_line(app, index, inner.width),
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn synthetic_row_line(app: &App, index: usize, width: u16) -> Line<'static> {
    let label = "+ new named server";
    if index == app.server_cursor {
        let pad_len = usize::from(width)
            .saturating_sub(2)
            .saturating_sub(label.chars().count());
        Line::from(Span::styled(
            format!("  {label}{}", " ".repeat(pad_len)),
            Style::default()
                .fg(theme::SELECTION_FG)
                .bg(theme::SELECTION_BG),
        ))
    } else {
        Line::from(Span::styled(
            format!("  {label}"),
            Style::default().fg(theme::DIMMED),
        ))
    }
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
    let border = if app.grid_loading() {
        Style::default().fg(super::anim::pulse_color(app.spinner_frame))
    } else {
        border_style(focused)
    };
    let mut block = Block::new().borders(Borders::ALL).border_style(border);
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
    if focused && app.spawn.is_none() {
        block = block.title_bottom(Line::from(" Enter: attach  n: new  x: kill ").centered());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(spawn) = &app.spawn {
        draw_spawn(frame, app, spawn, inner);
        return;
    }
    if app.committed_server.is_none() {
        return;
    }
    if app.skeleton_visible() {
        draw_skeleton(frame, app, inner);
        return;
    }
    let cards = app.displayed_terminals();
    if cards.is_empty() && app.ghost.is_none() {
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
        match &app.dissolve {
            Some(dissolve) if dissolve.terminal == terminal.name => {
                draw_dissolve_card(frame, dissolve, rect, inner);
            }
            _ => draw_card(
                frame,
                terminal,
                focused && index == app.grid_cursor,
                rect,
                inner,
            ),
        }
    }

    if let Some(ghost) = &app.ghost {
        let slot = cards.len();
        let row = slot / columns;
        if row >= app.grid_scroll && row < app.grid_scroll + visible_rows {
            let col = slot % columns;
            let x = inner.x + offsets[col];
            let y =
                inner.y + ((row - app.grid_scroll) as u16) * (grid::CARD_HEIGHT + grid::CARD_VGAP);
            let rect = Rect::new(x, y, grid::CARD_WIDTH, grid::CARD_HEIGHT);
            if rect.right() <= inner.right() && rect.bottom() <= inner.bottom() {
                draw_ghost_card(frame, ghost, app.spinner_frame, rect);
            }
        }
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

/// Skeleton cards in the last-known shape of the committed server's grid,
/// with a shimmer band sweeping the card interiors.
fn draw_skeleton(frame: &mut Frame, app: &App, inner: Rect) {
    let columns = grid::columns_for_width(inner.width);
    let offsets = grid::row_offsets(inner.width, columns);
    let visible_rows = grid::visible_card_rows(inner.height);
    for slot in 0..app.skeleton_count().min(columns * visible_rows) {
        let row = slot / columns;
        let col = slot % columns;
        let x = inner.x + offsets[col];
        let y = inner.y + (row as u16) * (grid::CARD_HEIGHT + grid::CARD_VGAP);
        let rect = Rect::new(x, y, grid::CARD_WIDTH, grid::CARD_HEIGHT);
        if rect.right() > inner.right() || rect.bottom() > inner.bottom() {
            break;
        }
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::SHIM[1]));
        let card_inner = block.inner(rect);
        frame.render_widget(block, rect);
        let lines: Vec<Line> = (0..card_inner.height)
            .map(|j| {
                Line::from(
                    (0..card_inner.width)
                        .map(|i| {
                            let level = super::anim::shimmer_level(app.spinner_frame, i, j);
                            Span::styled("░", Style::default().fg(theme::SHIM[level]))
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), card_inner);
    }
}

const SPAWN_BAR_WIDTH: u16 = 44;
const SPAWN_LOG_LINES: usize = 6;
/// Left-aligned `{secs:.1}s` column width in the spawn log (right-padded so
/// short stamps keep a gap before the message, matching the old 6-wide field).
const SPAWN_LOG_STAMP_WIDTH: usize = 6;

fn render_centered_line(frame: &mut Frame, inner: Rect, y: u16, line: Line<'static>) {
    if y >= inner.bottom() {
        return;
    }
    frame.render_widget(
        Paragraph::new(line.centered()),
        Rect::new(inner.x, y, inner.width, 1),
    );
}

/// Truncate `text` to `budget` columns, using a trailing `...` when clipped.
fn ellipsize(text: &str, budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    let len = text.chars().count();
    if len <= budget {
        return text.to_string();
    }
    if budget <= 3 {
        return ".".repeat(budget);
    }
    let mut out: String = text.chars().take(budget - 3).collect();
    out.push_str("...");
    out
}

/// One spawn log row: left-aligned timestamp column, message centered in
/// `width` when it fits without covering the stamp, otherwise left-aligned
/// after the stamp and ellipsized to the remaining columns.
fn format_spawn_log_line(width: usize, at_ticks: u32, message: &str) -> String {
    if width == 0 {
        return String::new();
    }
    let stamp = format!("{:.1}s", f64::from(at_ticks) / 10.0);
    let stamp_width = SPAWN_LOG_STAMP_WIDTH.max(stamp.chars().count());
    if width <= stamp_width {
        return ellipsize(&stamp, width);
    }
    let stamp_field = format!("{stamp:<stamp_width$}");
    let msg_len = message.chars().count();
    let ideal_start = width.saturating_sub(msg_len) / 2;
    let (msg_start, text) = if ideal_start >= stamp_width && ideal_start + msg_len <= width {
        (ideal_start, message.to_string())
    } else {
        let budget = width - stamp_width;
        if msg_len <= budget {
            (stamp_width, message.to_string())
        } else {
            (stamp_width, ellipsize(message, budget))
        }
    };
    let mut out = stamp_field;
    while out.chars().count() < msg_start {
        out.push(' ');
    }
    out.push_str(&text);
    out.chars().take(width).collect()
}

fn draw_spawn(frame: &mut Frame, app: &App, spawn: &super::app::SpawnView, inner: Rect) {
    use super::app::SpawnOutcome;

    let (title, accent) = match &spawn.outcome {
        Some(SpawnOutcome::Ready { .. }) => {
            (format!("{} is ready", spawn.server), theme::STATE_READY)
        }
        Some(SpawnOutcome::Failed { .. }) => ("spawn failed".to_string(), theme::STATUS_ERROR_BG),
        None => (format!("Starting {}", spawn.server), theme::STATE_PENDING),
    };
    let y0 = inner.y + inner.height / 5;
    render_centered_line(
        frame,
        inner,
        y0,
        Line::from(Span::styled(title, Style::default().fg(accent))),
    );

    let current = match &spawn.outcome {
        Some(SpawnOutcome::Failed { message, .. }) => message.clone(),
        _ => spawn
            .log
            .last()
            .map(|l| l.message.clone())
            .unwrap_or_else(|| "waiting for the hub".to_string()),
    };
    let message_line = if spawn.outcome.is_none() {
        let glyph =
            super::app::SPINNER_FRAMES[app.spinner_frame % super::app::SPINNER_FRAMES.len()];
        Line::from(vec![
            Span::styled(glyph.to_string(), Style::default().fg(theme::SPINNER)),
            Span::raw(format!("  {current}")),
        ])
    } else {
        Line::from(Span::styled(current, Style::default().fg(accent)))
    };
    render_centered_line(frame, inner, y0 + 2, message_line);

    let bar_width = SPAWN_BAR_WIDTH.min(inner.width.saturating_sub(8));
    let pct = spawn.shown as u16;
    let fill = usize::from(pct * bar_width / 100);
    let bar = Line::from(vec![
        Span::styled("[", Style::default().fg(theme::DIMMED)),
        Span::styled("█".repeat(fill), Style::default().fg(accent)),
        Span::styled(
            "░".repeat(usize::from(bar_width) - fill),
            Style::default().fg(theme::SHIM[0]),
        ),
        Span::styled("]", Style::default().fg(theme::DIMMED)),
        Span::raw(format!(" {pct:>3}%")),
    ]);
    render_centered_line(frame, inner, y0 + 4, bar);
    render_centered_line(
        frame,
        inner,
        y0 + 6,
        Line::from(Span::styled(
            format!("elapsed {:.1}s", f64::from(spawn.elapsed_ticks) / 10.0),
            Style::default().fg(theme::DIMMED),
        )),
    );

    let log_y = y0 + 9;
    let start = spawn.log.len().saturating_sub(SPAWN_LOG_LINES);
    let recent = &spawn.log[start..];
    let log_width = usize::from(inner.width);
    for (i, entry) in recent.iter().enumerate() {
        let y = log_y + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let newest = i + 1 == recent.len();
        let style = if newest {
            Style::default()
        } else {
            Style::default().fg(theme::DIMMED)
        };
        let text = format_spawn_log_line(log_width, entry.at_ticks, &entry.message);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(text, style))),
            Rect::new(inner.x, y, inner.width, 1),
        );
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

fn draw_dissolve_card(
    frame: &mut Frame,
    dissolve: &super::app::Dissolve,
    rect: Rect,
    bounds: Rect,
) {
    if rect.right() > bounds.right() || rect.bottom() > bounds.bottom() {
        return;
    }
    let progress = (dissolve.age_ticks as f32 / super::app::DISSOLVE_TICKS as f32).min(1.0);
    let border = if progress < 0.5 {
        theme::BORDER_UNFOCUSED
    } else {
        theme::SHIM[0]
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let label = grid::card_label(&dissolve.terminal);
    let label_x = inner.x + inner.width.saturating_sub(label.chars().count() as u16) / 2;
    let mut cells: Vec<(u16, u16, char)> =
        vec![(inner.x, inner.y, '>'), (inner.x + 1, inner.y, '_')];
    for (i, ch) in label.chars().enumerate() {
        cells.push((label_x + i as u16, inner.y + 2, ch));
    }
    let buf = frame.buffer_mut();
    for (x, y, ch) in cells {
        if x >= inner.right() || y >= inner.bottom() {
            continue;
        }
        let noise = super::anim::cell_noise(dissolve.seed, x, y);
        // Two decaying thresholds: cells pass through a dim middle dot before
        // going blank, so the card scatters instead of wiping.
        if noise < progress * 0.9 {
            buf[(x, y)].set_symbol(" ");
        } else if noise < progress * 1.4 {
            buf[(x, y)].set_symbol("·").set_fg(theme::SHIM[0]);
        } else {
            buf[(x, y)].set_char(ch);
        }
    }
}

/// Border drawn cell by cell so the marching gradient can color each segment
/// independently, which Block's single border_style cannot do.
fn draw_march_border(frame: &mut Frame, rect: Rect, frame_count: usize) {
    let mut points: Vec<(u16, u16, &str)> = Vec::new();
    for i in 0..rect.width {
        points.push((rect.x + i, rect.y, "─"));
    }
    for j in 1..rect.height.saturating_sub(1) {
        points.push((rect.right() - 1, rect.y + j, "│"));
    }
    for i in (0..rect.width).rev() {
        points.push((rect.x + i, rect.bottom() - 1, "─"));
    }
    for j in (1..rect.height.saturating_sub(1)).rev() {
        points.push((rect.x, rect.y + j, "│"));
    }
    let len = points.len();
    let buf = frame.buffer_mut();
    for (k, (x, y, glyph)) in points.into_iter().enumerate() {
        let glyph = match (
            x == rect.x,
            x == rect.right() - 1,
            y == rect.y,
            y == rect.bottom() - 1,
        ) {
            (true, _, true, _) => "┌",
            (_, true, true, _) => "┐",
            (true, _, _, true) => "└",
            (_, true, _, true) => "┘",
            _ => glyph,
        };
        let level = super::anim::march_level(frame_count, k, len);
        buf[(x, y)]
            .set_symbol(glyph)
            .set_fg(theme::GHOST_RAMP[level]);
    }
}

fn draw_ghost_card(frame: &mut Frame, ghost: &super::app::Ghost, frame_count: usize, rect: Rect) {
    use super::app::GhostPhase;
    match &ghost.phase {
        GhostPhase::Creating => {
            draw_march_border(frame, rect, frame_count);
            let inner = Rect::new(
                rect.x + 1,
                rect.y + 1,
                rect.width.saturating_sub(2),
                rect.height.saturating_sub(2),
            );
            let dots = ".".repeat(1 + frame_count / 3 % 3);
            let shimmer: Line = Line::from(
                (0..inner.width)
                    .map(|i| {
                        let level = super::anim::shimmer_level(frame_count, i, 0);
                        Span::styled("░", Style::default().fg(theme::SHIM[level]))
                    })
                    .collect::<Vec<_>>(),
            );
            let lines = vec![
                Line::from(Span::styled(
                    format!("creating{dots}"),
                    Style::default().fg(theme::STATE_PENDING),
                )),
                Line::from(""),
                Line::from(""),
                shimmer,
            ];
            frame.render_widget(Paragraph::new(lines), inner);
        }
        GhostPhase::Confirmed { .. } => {
            draw_flash_card(frame, rect, "created", theme::BORDER_FOCUSED)
        }
        GhostPhase::Failed { .. } => draw_flash_card(frame, rect, "failed", theme::STATUS_ERROR_BG),
    }
}

fn draw_flash_card(frame: &mut Frame, rect: Rect, label: &str, color: ratatui::style::Color) {
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(label.to_string(), Style::default().fg(color))).centered(),
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
    let connecting = app
        .peek
        .as_ref()
        .map(|p| !p.connected && p.error.is_none())
        .unwrap_or(true);
    let border = if connecting && app.peek_visible() {
        Style::default().fg(super::anim::pulse_color(app.spinner_frame))
    } else {
        border_style(false)
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(peek) = &app.peek else {
        render_peek_connecting(frame, app, inner);
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
        render_peek_connecting(frame, app, inner);
        return;
    }
    // Tail-anchor the emulated screen: show the window ending at the last
    // painted row so a shell prompt (bottom of the screen) stays visible while
    // a full-screen app's most recent region shows through.
    let rows: Vec<String> = peek.parser.screen().rows(0, peek.cols).collect();
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

fn render_peek_connecting(frame: &mut Frame, app: &App, inner: Rect) {
    let shimmer: Line = Line::from(
        (0..inner.width)
            .map(|i| {
                let level = super::anim::shimmer_level(app.spinner_frame, i, 0);
                Span::styled("░", Style::default().fg(theme::SHIM[level]))
            })
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "connecting...",
                Style::default().fg(theme::DIMMED),
            )),
            shimmer,
        ]),
        inner,
    );
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
    fn dissolving_card_decays_its_label() {
        let (mut app, _) = committed(&["1", "2"]);
        app.dissolve = Some(crate::tui::app::Dissolve {
            op: 9,
            terminal: "2".to_string(),
            age_ticks: crate::tui::app::DISSOLVE_TICKS,
            confirmed: false,
            seed: 12345,
        });
        let text = rendered(&app);
        assert!(
            text.contains("Terminal 001"),
            "untouched card intact:\n{text}"
        );
        assert!(
            !text.contains("Terminal 002"),
            "fully decayed label must be gone:\n{text}"
        );
    }

    #[test]
    fn dissolving_card_skips_rects_beyond_a_tiny_frame() {
        let (mut app, _) = committed(&["1", "2"]);
        app.dissolve = Some(crate::tui::app::Dissolve {
            op: 9,
            terminal: "2".to_string(),
            age_ticks: 3,
            confirmed: false,
            seed: 12345,
        });
        // Must not panic: the card rect overflows a 12-column frame.
        let _ = rendered_sized(&app, 12, 30);
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
        assert!(text.contains("Enter: attach  n: new  x: kill"));
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

        app.tick(now + crate::tui::app::HOVER_DEBOUNCE);
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
                crossterm::event::KeyCode::Tab,
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

    #[test]
    fn servers_pane_shows_the_synthetic_new_row() {
        // A wide frame keeps the servers pane (20% of frame width) wider
        // than the label, so it isn't clipped before the assertion below.
        let (app, _) = app_with_servers();
        let text = rendered_sized(&app, 150, 30);
        assert!(text.contains("+ new named server"), "buffer:\n{text}");
    }

    #[test]
    fn ghost_card_occupies_the_next_slot() {
        let (mut app, _) = committed(&["1"]);
        app.ghost = Some(crate::tui::app::Ghost {
            op: 9,
            phase: crate::tui::app::GhostPhase::Creating,
        });
        let text = rendered(&app);
        assert!(text.contains("creating"), "buffer:\n{text}");

        app.ghost.as_mut().unwrap().phase =
            crate::tui::app::GhostPhase::Confirmed { ticks_left: 3 };
        assert!(rendered(&app).contains("created"));

        app.ghost.as_mut().unwrap().phase = crate::tui::app::GhostPhase::Failed { ticks_left: 3 };
        assert!(rendered(&app).contains("failed"));
    }

    #[test]
    fn loud_fetch_shows_skeleton_cards() {
        let (mut app, _) = committed(&["1", "2", "3"]);
        app.grid_fetch = Some(crate::tui::app::GridFetch {
            op: 9,
            age_ticks: crate::tui::app::SKELETON_SHOW_TICKS,
            pending: None,
        });
        // Isolate the grid pane's skeleton from the peek pane, which keeps
        // previewing the last-hovered terminal independent of the grid fetch.
        app.hover = None;
        let text = rendered(&app);
        assert!(text.contains('░'), "skeleton shimmer glyphs:\n{text}");
        assert!(
            !text.contains("Terminal 001"),
            "stale cards replaced by skeletons:\n{text}"
        );
    }

    #[test]
    fn peek_connecting_shows_shimmer() {
        let (app, _) = committed(&["1"]);
        // committed() leaves the hover pending; the peek pane shows the connect state.
        let text = rendered(&app);
        assert!(text.contains("connecting"), "buffer:\n{text}");
        assert!(text.contains('░'), "connect shimmer:\n{text}");
    }

    #[test]
    fn spawn_takeover_replaces_the_grid() {
        let (mut app, _) = app_with_servers();
        app.spawn = Some(crate::tui::app::SpawnView {
            op: 5,
            server: "gpu-a100".to_string(),
            reported: 35,
            shown: 38.0,
            elapsed_ticks: 42,
            log: vec![
                crate::tui::app::SpawnLogLine {
                    at_ticks: 0,
                    message: "Server requested".to_string(),
                },
                crate::tui::app::SpawnLogLine {
                    at_ticks: 30,
                    message: "Pod scheduled on node gpu-07".to_string(),
                },
            ],
            outcome: None,
        });
        let text = rendered(&app);
        assert!(text.contains("Starting gpu-a100"), "buffer:\n{text}");
        assert!(text.contains("38%"), "buffer:\n{text}");
        assert!(text.contains("elapsed 4.2s"), "buffer:\n{text}");
        assert!(
            text.contains("Pod scheduled on node gpu-07"),
            "buffer:\n{text}"
        );
        assert!(text.contains('█'), "bar fill glyph:\n{text}");
        assert!(
            !text.contains("Enter: attach"),
            "hints hidden during takeover"
        );

        app.spawn.as_mut().unwrap().outcome =
            Some(crate::tui::app::SpawnOutcome::Ready { ticks_left: 5 });
        app.spawn.as_mut().unwrap().shown = 100.0;
        let text = rendered(&app);
        assert!(text.contains("gpu-a100 is ready"), "buffer:\n{text}");

        app.spawn.as_mut().unwrap().outcome = Some(crate::tui::app::SpawnOutcome::Failed {
            message: "quota exceeded".to_string(),
            ticks_left: 5,
        });
        let text = rendered(&app);
        assert!(text.contains("spawn failed"), "buffer:\n{text}");
        assert!(text.contains("quota exceeded"), "buffer:\n{text}");
    }

    #[test]
    fn create_named_starting_dialog_does_not_cover_spawn_progress() {
        let (mut app, _) = app_with_servers();
        app.spawn = Some(crate::tui::app::SpawnView {
            op: 7,
            server: "gpu".to_string(),
            reported: 40,
            shown: 42.0,
            elapsed_ticks: 20,
            log: vec![crate::tui::app::SpawnLogLine {
                at_ticks: 10,
                message: "Pod scheduled".to_string(),
            }],
            outcome: None,
        });
        app.dialog = Some(crate::tui::dialogs::Dialog::CreateNamed({
            let mut create =
                crate::tui::dialogs::CreateNamedDialog::new(&std::collections::BTreeMap::new());
            for c in "gpu".chars() {
                create.input.on_key(&crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char(c),
                    crossterm::event::KeyModifiers::NONE,
                ));
            }
            create.op = Some(7);
            create.step = crate::tui::dialogs::CreateStep::Starting;
            create
        }));
        let text = rendered(&app);
        assert!(text.contains("Starting gpu"), "progress visible:\n{text}");
        assert!(text.contains("42%"), "progress bar visible:\n{text}");
        assert!(
            !text.contains("Create named server"),
            "starting dialog must not cover the progress takeover:\n{text}"
        );
        assert!(
            !text.contains("starting 'gpu'"),
            "starting spinner dialog must not cover progress:\n{text}"
        );
    }

    #[test]
    fn format_spawn_log_line_left_aligns_stamp_and_centers_short_message() {
        let line = format_spawn_log_line(40, 20, "ready");
        assert!(
            line.starts_with("2.0s"),
            "stamp is left-aligned, not right-padded: {line:?}"
        );
        assert!(
            !line.starts_with(' '),
            "no leading spaces before stamp: {line:?}"
        );
        let msg_at = line.find("ready").expect("message present");
        // "ready" (5) centered in 40: start 17; stamp column is 6, so no clash.
        assert_eq!(msg_at, 17, "short message is centered: {line:?}");
        assert_eq!(line.chars().count(), 22); // stamp..message, no trailing pad required
    }

    #[test]
    fn format_spawn_log_line_left_aligns_and_ellipsizes_when_message_is_long() {
        let message = "This is an ultra long line which will exceed the window width and more";
        let line = format_spawn_log_line(40, 40, message);
        assert!(line.starts_with("4.0s"), "stamp left-aligned: {line:?}");
        assert!(
            line.ends_with("..."),
            "long message ends with ellipsis: {line:?}"
        );
        assert_eq!(line.chars().count(), 40, "fits the width exactly: {line:?}");
        // Message starts at the stamp column (no overlap).
        let after_stamp = &line[SPAWN_LOG_STAMP_WIDTH..];
        assert!(
            after_stamp.starts_with("This is"),
            "message left-aligned after stamp: {line:?}"
        );
        assert!(!line.contains("and more"), "tail is clipped: {line:?}");
    }

    #[test]
    fn format_spawn_log_line_left_aligns_message_when_centering_would_hit_stamp() {
        // Wide message that fits after the stamp but whose centered start would
        // land inside the stamp column.
        let message = "abcdefghij"; // 10 chars
        let width = 20;
        // Centered start would be (20-10)/2 = 5, but stamp column is 6.
        let line = format_spawn_log_line(width, 0, message);
        assert!(line.starts_with("0.0s"), "{line:?}");
        assert_eq!(
            line.find(message),
            Some(SPAWN_LOG_STAMP_WIDTH),
            "falls back to left-align after stamp: {line:?}"
        );
    }

    #[test]
    fn spawn_log_lines_stamp_left_message_centered_in_the_grid_pane() {
        let (mut app, _) = app_with_servers();
        let message = "Pod scheduled on node gpu-07";
        app.spawn = Some(crate::tui::app::SpawnView {
            op: 5,
            server: "gpu-a100".to_string(),
            reported: 35,
            shown: 38.0,
            elapsed_ticks: 42,
            log: vec![crate::tui::app::SpawnLogLine {
                at_ticks: 30,
                message: message.to_string(),
            }],
            outcome: None,
        });
        let text = rendered(&app);
        let line = text
            .lines()
            .find(|l| l.contains(message) && l.contains("3.0s"))
            .unwrap_or_else(|| panic!("log line missing:\n{text}"));
        let pane_start = line
            .find("││")
            .map(|i| i + "││".len())
            .unwrap_or_else(|| panic!("grid pane borders missing:\n{line}"));
        let pane_end = line[pane_start..]
            .rfind('│')
            .map(|i| pane_start + i)
            .unwrap_or(line.len());
        let pane = &line[pane_start..pane_end];
        let stamp_at = pane.find("3.0s").expect("stamp in pane");
        assert_eq!(stamp_at, 0, "stamp is left-aligned in the pane:\n{pane:?}");
        let msg_at = pane.find(message).expect("message in pane");
        let msg_end = msg_at + message.len();
        let left_pad = msg_at;
        let right_pad = pane.len().saturating_sub(msg_end);
        assert!(
            left_pad.abs_diff(right_pad) <= 1,
            "message text should be centered, left_pad={left_pad} right_pad={right_pad}:\n{pane:?}\nfull:\n{text}"
        );
    }
}
