use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::config::JsonMap;

use super::grid;

pub const STATUS_TTL: Duration = Duration::from_secs(5);
pub const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const PEEK_DEBOUNCE: Duration = Duration::from_millis(300);
pub const REJECT_FLASH_DURATION: Duration = Duration::from_millis(150);

#[derive(Debug)]
pub struct HoverState {
    pub terminal: String,
    pub since: Instant,
    pub started: bool,
}

// vt100::Parser has no Debug impl, so PeekState cannot derive it.
pub struct PeekState {
    pub terminal: String,
    pub connected: bool,
    pub error: Option<String>,
    pub rows: u16,
    pub cols: u16,
    pub parser: vt100::Parser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Servers,
    Grid,
}

#[derive(Debug, Clone)]
pub struct ServerRow {
    pub name: String,
    pub display: String,
    pub ready: bool,
    pub pending: Option<String>,
    pub options: JsonMap,
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TerminalRow {
    pub name: String,
}

#[derive(Debug)]
pub enum AppEvent {
    Refreshed {
        op: u64,
        username: String,
        servers: Vec<ServerRow>,
    },
    Terminals {
        op: u64,
        server: String,
        terminals: Vec<TerminalRow>,
    },
    Progress {
        message: String,
    },
    OpDone {
        op: u64,
        message: String,
    },
    OpFailed {
        op: u64,
        message: String,
    },
    TerminalCreated {
        op: u64,
        server: String,
        terminal: String,
    },
    PeekOpened {
        op: u64,
        terminal: String,
    },
    PeekChunk {
        terminal: String,
        text: String,
    },
    PeekFailed {
        op: u64,
        terminal: String,
        message: String,
    },
}

#[derive(Debug)]
pub enum Effect {
    Refresh {
        op: u64,
    },
    FetchTerminals {
        op: u64,
        server: String,
        url: String,
    },
    Start {
        op: u64,
        server: Option<String>,
        options: JsonMap,
    },
    Stop {
        op: u64,
        server: Option<String>,
    },
    NewTerminal {
        op: u64,
        server: String,
        url: String,
    },
    KillTerminal {
        op: u64,
        server: String,
        url: String,
        terminal: String,
    },
    PeekStart {
        op: u64,
        url: String,
        terminal: String,
        rows: u16,
        cols: u16,
    },
    PeekStop,
    Attach {
        target: String,
    },
    Quit,
}

impl Effect {
    /// Spinner label for network effects; None for loop-handled effects.
    fn label(&self) -> Option<&'static str> {
        match self {
            Effect::Refresh { .. } => Some("refreshing"),
            Effect::FetchTerminals { .. } => Some("loading terminals"),
            Effect::Start { .. } => Some("starting"),
            Effect::Stop { .. } => Some("stopping"),
            Effect::NewTerminal { .. } => Some("creating"),
            Effect::KillTerminal { .. } => Some("killing"),
            Effect::PeekStart { .. } => Some("connecting"),
            Effect::PeekStop | Effect::Attach { .. } | Effect::Quit => None,
        }
    }

    fn set_op(&mut self, id: u64) {
        match self {
            Effect::Refresh { op }
            | Effect::FetchTerminals { op, .. }
            | Effect::Start { op, .. }
            | Effect::Stop { op, .. }
            | Effect::NewTerminal { op, .. }
            | Effect::KillTerminal { op, .. }
            | Effect::PeekStart { op, .. } => *op = id,
            Effect::PeekStop | Effect::Attach { .. } | Effect::Quit => {}
        }
    }
}

#[derive(Debug)]
pub struct StatusMsg {
    pub text: String,
    pub error: bool,
    pub since: Instant,
}

pub struct App {
    pub hub_name: String,
    pub username: Option<String>,
    pub servers: Vec<ServerRow>,
    pub server_cursor: usize,
    pub committed_server: Option<String>,
    pub terminals: Vec<TerminalRow>,
    pub grid_cursor: usize,
    pub grid_scroll: usize,
    pub focus: Focus,
    pub dialog: Option<super::dialogs::Dialog>,
    pub status: Option<StatusMsg>,
    pub presets: BTreeMap<String, JsonMap>,
    pub terminal_limit: usize,
    pub size: (u16, u16),
    pub ops: BTreeMap<u64, &'static str>,
    pub spinner_frame: usize,
    pub hover: Option<HoverState>,
    pub peek: Option<PeekState>,
    peek_op: Option<u64>,
    pending_select: Option<String>,
    next_op: u64,
    effects: Vec<Effect>,
}

impl App {
    pub fn new(
        hub_name: String,
        presets: BTreeMap<String, JsonMap>,
        terminal_limit: usize,
        size: (u16, u16),
    ) -> Self {
        let mut app = Self {
            hub_name,
            username: None,
            servers: Vec::new(),
            server_cursor: 0,
            committed_server: None,
            terminals: Vec::new(),
            grid_cursor: 0,
            grid_scroll: 0,
            focus: Focus::Servers,
            dialog: None,
            status: None,
            presets,
            terminal_limit,
            size,
            ops: BTreeMap::new(),
            spinner_frame: 0,
            hover: None,
            peek: None,
            peek_op: None,
            pending_select: None,
            next_op: 1,
            effects: Vec::new(),
        };
        app.request_refresh();
        app
    }

    pub fn request_refresh(&mut self) {
        self.push_effect(Effect::Refresh { op: 0 });
    }

    /// Op ids are stamped here; construction sites use a 0 placeholder.
    fn push_effect(&mut self, mut effect: Effect) -> Option<u64> {
        let id = effect.label().map(|label| {
            let id = self.next_op;
            self.next_op += 1;
            self.ops.insert(id, label);
            id
        });
        if let Some(id) = id {
            effect.set_op(id);
        }
        self.effects.push(effect);
        id
    }

    fn finish_op(&mut self, op: u64) {
        self.ops.remove(&op);
    }

    /// Spinner glyph and the most recently started operation's label, while
    /// any operation is in flight.
    pub fn spinner(&self) -> Option<(&'static str, &'static str)> {
        self.ops.iter().next_back().map(|(_, label)| {
            (
                SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()],
                *label,
            )
        })
    }

    pub fn set_size(&mut self, cols: u16, rows: u16) {
        self.size = (cols, rows);
        self.ensure_grid_cursor_visible();
    }

    pub fn set_status(&mut self, text: String, error: bool, now: Instant) {
        self.status = Some(StatusMsg {
            text,
            error,
            since: now,
        });
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn tick(&mut self, now: Instant) {
        if let Some(status) = &self.status
            && now.duration_since(status.since) > STATUS_TTL
        {
            self.status = None;
        }
        if !self.ops.is_empty() {
            self.spinner_frame = self.spinner_frame.wrapping_add(1);
        }
        if let Some(super::dialogs::Dialog::CreateNamed(create)) = &mut self.dialog
            && let Some(since) = create.flash
            && now.duration_since(since) > REJECT_FLASH_DURATION
        {
            create.flash = None;
        }
        let due = self
            .hover
            .as_ref()
            .is_some_and(|h| !h.started && now.duration_since(h.since) >= PEEK_DEBOUNCE);
        if due {
            let target = self.committed_row().and_then(|r| r.url.clone());
            if let Some(url) = target {
                let terminal = self
                    .hover
                    .as_ref()
                    .expect("due is only true when hover is Some")
                    .terminal
                    .clone();
                let cols = grid::grid_inner_width(self.size.0).max(1);
                let rows = grid::peek_height(self.size.1.saturating_sub(1))
                    .saturating_sub(2)
                    .max(1);
                self.peek = Some(PeekState {
                    terminal: terminal.clone(),
                    connected: false,
                    error: None,
                    rows,
                    cols,
                    // Peek resizes the PTY to the pane on connect, so the parser
                    // and PTY agree on the size by construction and full-screen
                    // apps repaint into this exact grid. Ceiling: a terminal
                    // resize mid-hover keeps these dimensions until the next
                    // hover, which reconnects at the new size.
                    parser: vt100::Parser::new(rows, cols, 0),
                });
                self.peek_op = self.push_effect(Effect::PeekStart {
                    op: 0,
                    url,
                    terminal,
                    rows,
                    cols,
                });
                if let Some(hover) = &mut self.hover {
                    hover.started = true;
                }
            }
        }
    }

    pub fn committed_row(&self) -> Option<&ServerRow> {
        let name = self.committed_server.as_deref()?;
        self.servers.iter().find(|s| s.display == name)
    }

    pub fn displayed_terminals(&self) -> &[TerminalRow] {
        &self.terminals[..self.terminals.len().min(grid::DISPLAY_CAP)]
    }

    pub fn hovered_terminal(&self) -> Option<&TerminalRow> {
        if self.focus == Focus::Grid {
            self.displayed_terminals().get(self.grid_cursor)
        } else {
            None
        }
    }

    pub fn grid_columns(&self) -> usize {
        grid::columns_for_width(grid::grid_inner_width(self.size.0))
    }

    pub fn peek_visible(&self) -> bool {
        self.hover.is_some()
    }

    /// Reconcile the hover with the current cursor. A dialog, a focus change,
    /// or a cursor move ends the old hover (closing its socket); resting on a
    /// new card starts the debounce clock.
    fn sync_hover(&mut self, now: Instant) {
        let current = if self.dialog.is_none() {
            self.hovered_terminal().map(|t| t.name.clone())
        } else {
            None
        };
        let unchanged = self.hover.as_ref().map(|h| h.terminal.as_str()) == current.as_deref();
        if unchanged {
            return;
        }
        self.teardown_peek();
        if let Some(terminal) = current {
            self.hover = Some(HoverState {
                terminal,
                since: now,
                started: false,
            });
        }
    }

    fn teardown_peek(&mut self) {
        let had_socket = self.hover.as_ref().is_some_and(|h| h.started) || self.peek.is_some();
        if had_socket {
            self.effects.push(Effect::PeekStop);
        }
        if let Some(op) = self.peek_op.take() {
            self.finish_op(op);
        }
        self.hover = None;
        self.peek = None;
    }

    fn grid_inner_height(&self) -> u16 {
        let main = self.size.1.saturating_sub(1);
        let grid_pane = if self.peek_visible() {
            main.saturating_sub(grid::peek_height(main))
        } else {
            main
        };
        grid_pane.saturating_sub(2)
    }

    fn ensure_grid_cursor_visible(&mut self) {
        let cols = self.grid_columns();
        let visible = grid::visible_card_rows(self.grid_inner_height()).max(1);
        let row = self.grid_cursor / cols;
        if row < self.grid_scroll {
            self.grid_scroll = row;
        }
        if row >= self.grid_scroll + visible {
            self.grid_scroll = row + 1 - visible;
        }
    }

    pub fn apply(&mut self, event: AppEvent, now: Instant) {
        match event {
            AppEvent::Refreshed {
                op,
                username,
                servers,
            } => {
                self.finish_op(op);
                let cursor_name = self
                    .servers
                    .get(self.server_cursor)
                    .map(|s| s.display.clone());
                self.username = Some(username);
                self.servers = servers;
                self.server_cursor = cursor_name
                    .and_then(|d| self.servers.iter().position(|s| s.display == d))
                    .unwrap_or(0);
                self.revalidate_commitment();
            }
            AppEvent::Terminals {
                op,
                server,
                terminals,
            } => {
                self.finish_op(op);
                let valid = self.committed_server.as_deref() == Some(server.as_str())
                    && self.committed_row().is_some_and(|s| s.ready);
                if valid {
                    self.terminals = sorted_terminals(terminals);
                    let count = self.displayed_terminals().len();
                    self.grid_cursor = self.grid_cursor.min(count.saturating_sub(1));
                    self.ensure_grid_cursor_visible();
                    // A pending create points the cursor at its new terminal
                    // once the list refresh shows it; a missing name clears the
                    // request so it cannot hijack a later refresh of the same
                    // server (switching servers clears it in commit_cursor_server).
                    if let Some(name) = self.pending_select.take()
                        && let Some(index) = self
                            .displayed_terminals()
                            .iter()
                            .position(|t| t.name == name)
                    {
                        self.grid_cursor = index;
                        self.ensure_grid_cursor_visible();
                    }
                }
            }
            AppEvent::Progress { message } => self.set_status(message, false, now),
            AppEvent::OpDone { op, message } => {
                self.finish_op(op);
                self.set_status(message, false, now);
                self.request_refresh();
            }
            AppEvent::OpFailed { op, message } => {
                self.finish_op(op);
                self.set_status(message, true, now);
            }
            AppEvent::TerminalCreated {
                op,
                server,
                terminal,
            } => {
                self.finish_op(op);
                self.set_status(
                    format!("created terminal {terminal} on {server}"),
                    false,
                    now,
                );
                self.pending_select = Some(terminal);
                let target = self
                    .committed_row()
                    .and_then(|r| r.url.clone().map(|u| (r.display.clone(), u)));
                if let Some((server, url)) = target {
                    self.push_effect(Effect::FetchTerminals { op: 0, server, url });
                }
            }
            AppEvent::PeekOpened { op, terminal } => {
                self.finish_op(op);
                if self.peek_op == Some(op) {
                    self.peek_op = None;
                }
                if let Some(peek) = &mut self.peek
                    && peek.terminal == terminal
                {
                    peek.connected = true;
                }
            }
            AppEvent::PeekChunk { terminal, text } => {
                if let Some(peek) = &mut self.peek
                    && peek.terminal == terminal
                {
                    peek.parser.process(text.as_bytes());
                }
            }
            AppEvent::PeekFailed {
                op,
                terminal,
                message,
            } => {
                self.finish_op(op);
                if self.peek_op == Some(op) {
                    self.peek_op = None;
                }
                if let Some(peek) = &mut self.peek
                    && peek.terminal == terminal
                {
                    peek.error = Some(message);
                }
            }
        }
        self.sync_hover(now);
    }

    /// After a server refresh: keep a still-ready committed server (and
    /// refetch its terminals so the grid stays fresh), or drop a commitment
    /// whose server vanished or stopped.
    fn revalidate_commitment(&mut self) {
        let target = self
            .committed_row()
            .map(|r| (r.display.clone(), r.ready, r.url.clone()));
        match target {
            Some((server, true, Some(url))) => {
                self.push_effect(Effect::FetchTerminals { op: 0, server, url });
            }
            None => {
                if self.committed_server.is_some() {
                    self.drop_commitment();
                }
            }
            Some(_) => self.drop_commitment(),
        }
    }

    fn drop_commitment(&mut self) {
        self.committed_server = None;
        self.terminals.clear();
        self.grid_cursor = 0;
        self.grid_scroll = 0;
        if self.focus == Focus::Grid {
            self.focus = Focus::Servers;
        }
    }

    pub fn after_attach(&mut self, message: String, now: Instant) {
        self.set_status(message, false, now);
        let target = self
            .committed_row()
            .map(|r| (r.display.clone(), r.url.clone()));
        if let Some((server, Some(url))) = target {
            self.push_effect(Effect::FetchTerminals { op: 0, server, url });
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        self.handle_key(key, now);
        self.sync_hover(now);
    }

    fn handle_key(&mut self, key: &KeyEvent, now: Instant) {
        if let Some(dialog) = &mut self.dialog {
            match super::dialogs::handle_key(dialog, key, now) {
                super::dialogs::Outcome::Stay => {}
                super::dialogs::Outcome::Close => self.dialog = None,
                super::dialogs::Outcome::Commit(effect) => {
                    self.dialog = None;
                    self.push_effect(effect);
                }
                super::dialogs::Outcome::Spawn(effect) => {
                    let op = self.push_effect(effect);
                    if let Some(super::dialogs::Dialog::CreateNamed(create)) = &mut self.dialog {
                        create.op = op;
                        create.step = super::dialogs::CreateStep::Starting;
                    }
                }
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.effects.push(Effect::Quit),
            KeyCode::Char('r') => self.request_refresh(),
            KeyCode::Tab => self.toggle_focus(),
            _ => match self.focus {
                Focus::Servers => self.on_key_servers(key.code, now),
                Focus::Grid => self.on_key_grid(key.code, now),
            },
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Servers if self.committed_server.is_some() => Focus::Grid,
            Focus::Servers => Focus::Servers,
            Focus::Grid => Focus::Servers,
        };
    }

    fn on_key_servers(&mut self, code: KeyCode, now: Instant) {
        let synthetic = self.server_cursor == self.servers.len();
        match code {
            KeyCode::Up => self.move_server_cursor(false),
            KeyCode::Down => self.move_server_cursor(true),
            KeyCode::Enter if synthetic => self.open_create_named(),
            KeyCode::Char('n') if synthetic => self.open_create_named(),
            KeyCode::Enter => self.enter_server(now),
            KeyCode::Char('n') => self.start_server_dialog(now),
            KeyCode::Char('x') => self.confirm_stop_server(now),
            _ => {}
        }
    }

    fn open_create_named(&mut self) {
        self.dialog = Some(super::dialogs::Dialog::CreateNamed(
            super::dialogs::CreateNamedDialog::new(&self.presets),
        ));
    }

    fn on_key_grid(&mut self, code: KeyCode, now: Instant) {
        match code {
            KeyCode::Esc => self.focus = Focus::Servers,
            KeyCode::Left => self.move_grid_cursor(-1, 0),
            KeyCode::Right => self.move_grid_cursor(1, 0),
            KeyCode::Up => self.move_grid_cursor(0, -1),
            KeyCode::Down => self.move_grid_cursor(0, 1),
            KeyCode::Enter => self.attach_hovered(),
            KeyCode::Char('n') => self.new_terminal_on_committed(now),
            KeyCode::Char('x') => self.confirm_kill_terminal(),
            _ => {}
        }
    }

    fn move_server_cursor(&mut self, down: bool) {
        // The synthetic "+ new named server" row sits at index servers.len(),
        // so the cursor range is 0..=servers.len().
        let max = self.servers.len();
        self.server_cursor = if down {
            (self.server_cursor + 1).min(max)
        } else {
            self.server_cursor.saturating_sub(1)
        };
    }

    fn move_grid_cursor(&mut self, dx: i32, dy: i32) {
        let count = self.displayed_terminals().len();
        if count == 0 {
            return;
        }
        let cols = self.grid_columns();
        let row = self.grid_cursor / cols;
        let col = self.grid_cursor % cols;
        let last_row = (count - 1) / cols;
        let next = match (dx, dy) {
            (-1, 0) if col > 0 => self.grid_cursor - 1,
            (1, 0) if col + 1 < cols => (self.grid_cursor + 1).min(count - 1),
            (0, -1) if row > 0 => self.grid_cursor - cols,
            (0, 1) if row < last_row => (self.grid_cursor + cols).min(count - 1),
            _ => self.grid_cursor,
        };
        self.grid_cursor = next;
        self.ensure_grid_cursor_visible();
    }

    fn enter_server(&mut self, now: Instant) {
        let Some(server) = self.servers.get(self.server_cursor) else {
            return;
        };
        if server.ready && server.url.is_some() {
            self.commit_cursor_server();
            self.focus = Focus::Grid;
        } else if server.ready {
            self.set_status("the server reports no URL; refresh".to_string(), true, now);
        } else {
            self.spawn_stopped_server(now);
        }
    }

    /// Dialog/status paths for a not-yet-ready cursor server, shared by Enter
    /// and by `n` in the Servers pane: a pending spawn reports progress, and any
    /// stopped server (default or named) opens the Start preset picker. A named
    /// server carries its name into the spawn so the picker restarts it directly.
    fn spawn_stopped_server(&mut self, now: Instant) {
        let Some(server) = self.servers.get(self.server_cursor) else {
            return;
        };
        if server.pending.is_some() {
            self.set_status("a spawn is already in progress".to_string(), false, now);
        } else {
            let target = if server.name.is_empty() {
                None
            } else {
                Some(server.name.clone())
            };
            self.dialog = Some(super::dialogs::Dialog::Start(
                super::dialogs::StartDialog::new(target, &self.presets),
            ));
        }
    }

    /// `n` in the Servers pane starts the cursor server: a ready server is
    /// already running (info status), and anything else follows the shared
    /// stopped-server paths.
    fn start_server_dialog(&mut self, now: Instant) {
        let Some(server) = self.servers.get(self.server_cursor) else {
            return;
        };
        if server.ready {
            let display = server.display.clone();
            self.set_status(format!("{display} is already running"), false, now);
        } else {
            self.spawn_stopped_server(now);
        }
    }

    fn commit_cursor_server(&mut self) {
        let Some(server) = self.servers.get(self.server_cursor) else {
            return;
        };
        let display = server.display.clone();
        let url = server.url.clone();
        if self.committed_server.as_deref() != Some(display.as_str()) {
            self.terminals.clear();
            self.grid_cursor = 0;
            self.grid_scroll = 0;
            // A create pending on the previous server must not select a
            // same-named terminal on this one.
            self.pending_select = None;
        }
        self.committed_server = Some(display.clone());
        if let Some(url) = url {
            self.push_effect(Effect::FetchTerminals {
                op: 0,
                server: display,
                url,
            });
        }
    }

    fn tui_terminal_cap(&self) -> usize {
        self.terminal_limit.min(grid::DISPLAY_CAP)
    }

    /// True (and an error status is set) when the held terminal list is at the
    /// interactive cap. Callers only invoke this when the held list belongs to
    /// the server being created on.
    fn reject_at_cap(&mut self, now: Instant) -> bool {
        if self.terminals.len() < self.tui_terminal_cap() {
            return false;
        }
        let message = if self.terminal_limit > grid::DISPLAY_CAP {
            "the interactive interface caps at 999 terminals; use the CLI to create more"
                .to_string()
        } else {
            format!("terminal limit reached ({})", self.tui_terminal_cap())
        };
        self.set_status(message, true, now);
        true
    }

    fn new_terminal_on_committed(&mut self, now: Instant) {
        if self.reject_at_cap(now) {
            return;
        }
        let Some((server, url)) = self
            .committed_row()
            .and_then(|r| r.url.clone().map(|u| (r.display.clone(), u)))
        else {
            return;
        };
        self.push_effect(Effect::NewTerminal { op: 0, server, url });
    }

    fn confirm_stop_server(&mut self, now: Instant) {
        let Some(server) = self.servers.get(self.server_cursor) else {
            return;
        };
        if !(server.ready || server.pending.is_some()) {
            self.set_status("the selected server is not running".to_string(), true, now);
            return;
        }
        let target = (!server.name.is_empty()).then(|| server.name.clone());
        self.dialog = Some(super::dialogs::Dialog::Confirm(
            super::dialogs::ConfirmDialog {
                message: format!("Stop {}? Running work will be lost.", server.display),
                effect: Effect::Stop {
                    op: 0,
                    server: target,
                },
            },
        ));
    }

    fn confirm_kill_terminal(&mut self) {
        let Some(terminal) = self.hovered_terminal().map(|t| t.name.clone()) else {
            return;
        };
        let Some((server, url)) = self
            .committed_row()
            .and_then(|r| r.url.clone().map(|u| (r.display.clone(), u)))
        else {
            return;
        };
        self.dialog = Some(super::dialogs::Dialog::Confirm(
            super::dialogs::ConfirmDialog {
                message: format!("Kill {} on {}?", grid::card_label(&terminal), server),
                effect: Effect::KillTerminal {
                    op: 0,
                    server,
                    url,
                    terminal,
                },
            },
        ));
    }

    fn attach_hovered(&mut self) {
        let Some(terminal) = self.hovered_terminal().map(|t| t.name.clone()) else {
            return;
        };
        let Some(server_name) = self.committed_row().map(|r| r.name.clone()) else {
            return;
        };
        self.teardown_peek();
        self.effects.push(Effect::Attach {
            target: format!("{server_name}:{terminal}"),
        });
    }
}

/// Ascending numeric order; non-numeric names sort after numeric ones,
/// lexicographically among themselves.
fn sorted_terminals(mut terminals: Vec<TerminalRow>) -> Vec<TerminalRow> {
    terminals.sort_by(
        |a, b| match (a.name.parse::<u64>(), b.name.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            (Err(_), Err(_)) => a.name.cmp(&b.name),
        },
    );
    terminals
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};

    pub(crate) fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    pub(crate) fn row(name: &str, ready: bool) -> ServerRow {
        ServerRow {
            name: name.to_string(),
            display: if name.is_empty() {
                "default".to_string()
            } else {
                name.to_string()
            },
            ready,
            pending: None,
            options: JsonMap::new(),
            url: ready.then(|| format!("/user/ww41/{name}/")),
        }
    }

    fn terminals(names: &[&str]) -> Vec<TerminalRow> {
        names
            .iter()
            .map(|n| TerminalRow {
                name: (*n).to_string(),
            })
            .collect()
    }

    /// 100x30 frame: grid inner width 78, so 4 cards per row; inner height
    /// 27, so 4 visible card rows.
    pub(crate) fn fresh_app() -> (App, Instant) {
        let now = Instant::now();
        let mut app = App::new("icrn".to_string(), Default::default(), 999, (100, 30));
        let effects = app.take_effects();
        assert!(matches!(effects.as_slice(), [Effect::Refresh { op: 1 }]));
        app.apply(
            AppEvent::Refreshed {
                op: 1,
                username: "ww41".to_string(),
                servers: vec![row("", true), row("backup", true), row("lab", false)],
            },
            now,
        );
        (app, now)
    }

    /// fresh_app with the default server committed and terminals loaded.
    pub(crate) fn committed_app(names: &[&str]) -> (App, Instant) {
        let (mut app, now) = fresh_app();
        app.on_key(&press(KeyCode::Enter), now);
        let effects = app.take_effects();
        let op = match effects.as_slice() {
            [Effect::FetchTerminals { op, server, .. }] if server == "default" => *op,
            other => panic!("unexpected effects: {other:?}"),
        };
        app.apply(
            AppEvent::Terminals {
                op,
                server: "default".to_string(),
                terminals: terminals(names),
            },
            now,
        );
        (app, now)
    }

    #[test]
    fn new_registers_a_refresh_op() {
        let mut app = App::new("icrn".to_string(), Default::default(), 999, (100, 30));
        assert_eq!(app.ops.get(&1), Some(&"refreshing"));
        assert!(app.spinner().is_some());
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 1,
                username: "ww41".to_string(),
                servers: vec![],
            },
            Instant::now(),
        );
        assert!(app.ops.is_empty());
        assert!(app.spinner().is_none());
    }

    #[test]
    fn cursor_movement_alone_fetches_nothing() {
        let (mut app, now) = fresh_app();
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Down), now);
        app.on_key(&press(KeyCode::Up), now);
        assert!(app.take_effects().is_empty());
        assert!(app.committed_server.is_none());
    }

    #[test]
    fn enter_commits_fetches_and_focuses_grid() {
        let (mut app, now) = fresh_app();
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Enter), now);
        assert_eq!(app.committed_server.as_deref(), Some("default"));
        assert_eq!(app.focus, Focus::Grid);
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::FetchTerminals { server, .. }] if server == "default"
        ));
    }

    #[test]
    fn stopped_named_row_opens_start_dialog_with_its_name() {
        let (mut app, now) = fresh_app();
        // fresh_app's servers are [default(ready), backup(ready), lab(stopped)];
        // move the cursor to the stopped named row "lab".
        app.on_key(&press(KeyCode::Down), now);
        app.on_key(&press(KeyCode::Down), now);
        assert_eq!(app.servers[app.server_cursor].display, "lab");
        app.on_key(&press(KeyCode::Enter), now);
        match &app.dialog {
            Some(crate::tui::dialogs::Dialog::Start(start)) => {
                assert_eq!(start.server.as_deref(), Some("lab"));
            }
            other => panic!("expected a Start dialog, got {other:?}"),
        }
    }

    #[test]
    fn enter_on_stopped_default_and_named_rows_open_start_dialog() {
        let (mut app, now) = fresh_app();
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 99,
                username: "ww41".to_string(),
                servers: vec![row("", false), row("backup", false)],
            },
            now,
        );
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Enter), now);
        assert!(matches!(
            app.dialog,
            Some(crate::tui::dialogs::Dialog::Start(_))
        ));
        app.on_key(&press(KeyCode::Esc), now);
        app.on_key(&press(KeyCode::Down), now);
        app.on_key(&press(KeyCode::Enter), now);
        match &app.dialog {
            Some(crate::tui::dialogs::Dialog::Start(start)) => {
                assert_eq!(start.server.as_deref(), Some("backup"));
            }
            other => panic!("expected a Start dialog, got {other:?}"),
        }
    }

    #[test]
    fn tab_needs_a_commitment_and_esc_returns() {
        let (mut app, now) = fresh_app();
        app.on_key(&press(KeyCode::Tab), now);
        assert_eq!(app.focus, Focus::Servers);
        app.on_key(&press(KeyCode::Enter), now);
        assert_eq!(app.focus, Focus::Grid);
        app.on_key(&press(KeyCode::Esc), now);
        assert_eq!(app.focus, Focus::Servers);
        app.on_key(&press(KeyCode::Tab), now);
        assert_eq!(app.focus, Focus::Grid);
    }

    #[test]
    fn refresh_keeps_valid_commitment_and_refetches() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 50,
                username: "ww41".to_string(),
                servers: vec![row("", true)],
            },
            now,
        );
        assert_eq!(app.committed_server.as_deref(), Some("default"));
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::FetchTerminals { server, .. }] if server == "default"
        ));
    }

    #[test]
    fn refresh_drops_stopped_commitment_and_leaves_grid() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 50,
                username: "ww41".to_string(),
                servers: vec![row("", false)],
            },
            now,
        );
        assert!(app.committed_server.is_none());
        assert!(app.terminals.is_empty());
        assert_eq!(app.focus, Focus::Servers);
    }

    #[test]
    fn terminals_sort_numerically_and_stale_events_drop() {
        let (mut app, now) = committed_app(&["10", "2", "zsh", "1"]);
        let names: Vec<&str> = app.terminals.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["1", "2", "10", "zsh"]);
        app.apply(
            AppEvent::Terminals {
                op: 77,
                server: "backup".to_string(),
                terminals: vec![TerminalRow {
                    name: "9".to_string(),
                }],
            },
            now,
        );
        assert_eq!(app.terminals.len(), 4, "stale server terminals must drop");
    }

    #[test]
    fn grid_cursor_moves_in_two_dimensions() {
        // 100 wide -> 4 columns. 6 terminals: rows [0,1,2,3], [4,5].
        let (mut app, now) = committed_app(&["1", "2", "3", "4", "5", "6"]);
        assert_eq!(app.grid_columns(), 4);
        app.on_key(&press(KeyCode::Right), now);
        assert_eq!(app.grid_cursor, 1);
        app.on_key(&press(KeyCode::Down), now);
        assert_eq!(app.grid_cursor, 5);
        app.on_key(&press(KeyCode::Down), now);
        assert_eq!(app.grid_cursor, 5, "no row below");
        app.on_key(&press(KeyCode::Up), now);
        assert_eq!(app.grid_cursor, 1);
        app.on_key(&press(KeyCode::Right), now);
        app.on_key(&press(KeyCode::Right), now);
        assert_eq!(app.grid_cursor, 3);
        app.on_key(&press(KeyCode::Right), now);
        assert_eq!(app.grid_cursor, 3, "no wrap at row end");
        app.on_key(&press(KeyCode::Down), now);
        assert_eq!(app.grid_cursor, 5, "shorter last row clamps");
    }

    #[test]
    fn grid_scroll_follows_the_cursor() {
        // 4 columns, 40 terminals = 10 rows; 30 rows tall -> inner height 27
        // -> 4 visible card rows.
        let names: Vec<String> = (1..=40).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (mut app, now) = committed_app(&refs);
        assert_eq!(app.grid_scroll, 0);
        for _ in 0..9 {
            app.on_key(&press(KeyCode::Down), now);
        }
        assert_eq!(app.grid_cursor, 36);
        assert_eq!(
            app.grid_scroll, 7,
            "row 9 visible with 3 rows needs scroll 7"
        );
        for _ in 0..9 {
            app.on_key(&press(KeyCode::Up), now);
        }
        assert_eq!(app.grid_scroll, 0);
    }

    #[test]
    fn enter_on_terminal_attaches_with_server_name_addressing() {
        let (mut app, now) = committed_app(&["2"]);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Enter), now);
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::Attach { target }] if target == ":2"
        ));
    }

    #[test]
    fn n_in_grid_creates_and_n_at_cap_rejects() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::NewTerminal { server, .. }] if server == "default"
        ));

        let names: Vec<String> = (1..=999).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (mut app, now) = committed_app(&refs);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(app.take_effects().is_empty());
        let status = app.status.as_ref().expect("cap must set a status");
        assert_eq!(status.text, "terminal limit reached (999)");
    }

    #[test]
    fn n_at_cap_with_raised_config_points_at_the_cli() {
        let now = Instant::now();
        let mut app = App::new("icrn".to_string(), Default::default(), 1500, (100, 30));
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 1,
                username: "ww41".to_string(),
                servers: vec![row("", true)],
            },
            now,
        );
        app.on_key(&press(KeyCode::Enter), now);
        let effects = app.take_effects();
        let op = match effects.as_slice() {
            [Effect::FetchTerminals { op, .. }] => *op,
            other => panic!("unexpected effects: {other:?}"),
        };
        let many: Vec<TerminalRow> = (1..=999)
            .map(|i| TerminalRow {
                name: i.to_string(),
            })
            .collect();
        app.apply(
            AppEvent::Terminals {
                op,
                server: "default".to_string(),
                terminals: many,
            },
            now,
        );
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(app.take_effects().is_empty());
        let status = app.status.as_ref().expect("cap must set a status");
        assert!(status.text.contains("caps at 999"));
        assert!(status.text.contains("CLI"));
    }

    #[test]
    fn n_in_servers_pane_opens_the_start_flow() {
        let (mut app, now) = fresh_app();
        let _ = app.take_effects();
        let pending = ServerRow {
            name: "run".to_string(),
            display: "run".to_string(),
            ready: false,
            pending: Some("starting".to_string()),
            options: JsonMap::new(),
            url: None,
        };
        app.apply(
            AppEvent::Refreshed {
                op: 5,
                username: "ww41".to_string(),
                servers: vec![
                    row("", false),
                    row("backup", true),
                    pending,
                    row("lab", false),
                ],
            },
            now,
        );
        let _ = app.take_effects();

        // Stopped default: opens the Start dialog, no effects, no focus change.
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(matches!(
            app.dialog,
            Some(crate::tui::dialogs::Dialog::Start(_))
        ));
        assert_eq!(app.focus, Focus::Servers);
        assert!(app.committed_server.is_none());
        assert!(app.take_effects().is_empty());
        app.on_key(&press(KeyCode::Esc), now);
        assert!(app.dialog.is_none());

        // Ready: an info status, no dialog, no effects, no focus change.
        app.on_key(&press(KeyCode::Down), now);
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(app.dialog.is_none());
        assert!(app.take_effects().is_empty());
        assert_eq!(app.focus, Focus::Servers);
        let status = app.status.as_ref().expect("ready sets a status");
        assert!(!status.error);
        assert_eq!(status.text, "backup is already running");

        // Pending: an info status about a spawn in progress.
        app.on_key(&press(KeyCode::Down), now);
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(app.take_effects().is_empty());
        let status = app.status.as_ref().expect("pending sets a status");
        assert!(!status.error);
        assert_eq!(status.text, "a spawn is already in progress");

        // Stopped named: opens the Start dialog with its name.
        app.on_key(&press(KeyCode::Down), now);
        app.on_key(&press(KeyCode::Char('n')), now);
        match &app.dialog {
            Some(crate::tui::dialogs::Dialog::Start(start)) => {
                assert_eq!(start.server.as_deref(), Some("lab"));
            }
            other => panic!("expected a Start dialog, got {other:?}"),
        }
        assert!(app.take_effects().is_empty());
    }

    #[test]
    fn creating_a_terminal_selects_it_in_the_grid() {
        let (mut app, now) = committed_app(&["1", "2"]);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('n')), now);
        let op = match app.take_effects().as_slice() {
            [Effect::NewTerminal { op, server, .. }] if server == "default" => *op,
            other => panic!("unexpected effects: {other:?}"),
        };
        app.apply(
            AppEvent::TerminalCreated {
                op,
                server: "default".to_string(),
                terminal: "5".to_string(),
            },
            now,
        );
        let status = app.status.as_ref().expect("create sets a status");
        assert_eq!(status.text, "created terminal 5 on default");
        assert!(!status.error);
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::FetchTerminals { server, .. }] if server == "default"
        ));

        // The refreshed list carries the new terminal; the cursor lands on it
        // and the hover follows.
        app.apply(
            AppEvent::Terminals {
                op: 900,
                server: "default".to_string(),
                terminals: terminals(&["1", "2", "5"]),
            },
            now,
        );
        assert_eq!(app.displayed_terminals()[app.grid_cursor].name, "5");
        assert_eq!(app.hover.as_ref().map(|h| h.terminal.as_str()), Some("5"));

        // A second list refresh must not re-select the same terminal.
        app.on_key(&press(KeyCode::Left), now);
        let moved = app.grid_cursor;
        app.apply(
            AppEvent::Terminals {
                op: 901,
                server: "default".to_string(),
                terminals: terminals(&["1", "2", "5"]),
            },
            now,
        );
        assert_eq!(
            app.grid_cursor, moved,
            "a second Terminals must not re-select"
        );
    }

    #[test]
    fn x_flows_open_confirm_and_commit() {
        let (mut app, now) = fresh_app();
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('x')), now);
        assert!(app.dialog.is_some());
        app.on_key(&press(KeyCode::Char('q')), now);
        assert!(app.dialog.is_some(), "q inside a dialog must not quit");
        assert!(app.take_effects().is_empty());
        app.on_key(&press(KeyCode::Enter), now);
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::Stop { server: None, .. }]
        ));

        let (mut app, now) = committed_app(&["2"]);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('x')), now);
        app.on_key(&press(KeyCode::Char('y')), now);
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::KillTerminal { terminal, .. }] if terminal == "2"
        ));
    }

    #[test]
    fn op_done_refreshes_and_status_expires() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.apply(
            AppEvent::OpDone {
                op: 40,
                message: "created shell 2 on default".to_string(),
            },
            now,
        );
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::Refresh { .. }]
        ));
        assert!(app.status.is_some());
        app.tick(now + STATUS_TTL + Duration::from_millis(1));
        assert!(app.status.is_none());
    }

    #[test]
    fn after_attach_sets_status_and_refetches() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.after_attach("attach ended".to_string(), now);
        assert!(app.status.is_some());
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::FetchTerminals { .. }]
        ));
    }

    #[test]
    fn q_quits_and_spinner_advances_only_while_busy() {
        let (mut app, now) = fresh_app();
        app.tick(now);
        assert_eq!(app.spinner_frame, 0, "no ops in flight");
        app.request_refresh();
        app.tick(now);
        assert_eq!(app.spinner_frame, 1);
        app.on_key(&press(KeyCode::Char('q')), now);
        assert!(matches!(app.take_effects().last(), Some(Effect::Quit)));
    }

    #[test]
    fn hover_starts_peek_only_after_the_debounce() {
        let (mut app, now) = committed_app(&["1", "2"]);
        let _ = app.take_effects();
        assert!(app.peek_visible(), "grid focus + cursor on a card hovers");
        app.tick(now + Duration::from_millis(100));
        assert!(app.take_effects().is_empty(), "before the debounce");
        app.tick(now + PEEK_DEBOUNCE);
        let effects = app.take_effects();
        // 100x30 frame: grid inner width 78, peek pane inner height 5.
        assert!(matches!(
            effects.as_slice(),
            [Effect::PeekStart {
                terminal,
                rows: 5,
                cols: 78,
                ..
            }] if terminal == "1"
        ));
        assert_eq!(app.ops.values().next_back(), Some(&"connecting"));
        app.tick(now + PEEK_DEBOUNCE + Duration::from_millis(200));
        assert!(app.take_effects().is_empty(), "never starts twice");
    }

    #[test]
    fn skimming_across_cards_never_connects() {
        let (mut app, now) = committed_app(&["1", "2", "3"]);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Right), now + Duration::from_millis(100));
        app.on_key(&press(KeyCode::Right), now + Duration::from_millis(200));
        app.tick(now + Duration::from_millis(400));
        // Hover on "3" began at t=200ms; 400ms is before its 300ms debounce.
        assert!(app.take_effects().is_empty());
        app.tick(now + Duration::from_millis(501));
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::PeekStart { terminal, .. }] if terminal == "3"
        ));
    }

    #[test]
    fn moving_off_a_live_peek_stops_it_and_closes_the_op() {
        let (mut app, now) = committed_app(&["1", "2"]);
        let _ = app.take_effects();
        app.tick(now + PEEK_DEBOUNCE);
        let _ = app.take_effects();
        assert!(!app.ops.is_empty(), "connecting op open");
        app.on_key(&press(KeyCode::Right), now + Duration::from_millis(400));
        let effects = app.take_effects();
        assert!(matches!(effects.as_slice(), [Effect::PeekStop]));
        assert!(app.ops.is_empty(), "abort must close the connecting op");
        assert!(app.peek.is_none());
        assert!(app.hover.is_some(), "the new card is now hovered");
    }

    #[test]
    fn focus_change_and_dialogs_tear_the_hover_down() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Esc), now);
        assert!(app.hover.is_none());
        assert!(!app.peek_visible());

        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.tick(now + PEEK_DEBOUNCE);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('x')), now); // opens the kill confirm
        let effects = app.take_effects();
        assert!(matches!(effects.as_slice(), [Effect::PeekStop]));
        assert!(app.hover.is_none());
    }

    #[test]
    fn peek_events_paint_the_screen_and_stale_chunks_drop() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.tick(now + PEEK_DEBOUNCE);
        let _ = app.take_effects();
        let op = *app.ops.keys().next_back().expect("connecting op");
        app.apply(
            AppEvent::PeekOpened {
                op,
                terminal: "1".to_string(),
            },
            now,
        );
        assert!(app.ops.is_empty());
        assert!(app.peek.as_ref().is_some_and(|p| p.connected));
        // Cursor addressing paints row 1 before row 0; a faithful screen shows
        // them in screen order, not arrival order.
        app.apply(
            AppEvent::PeekChunk {
                terminal: "1".to_string(),
                text: "\u{1b}[2;1Hsecond line\u{1b}[1;1Hfirst line".to_string(),
            },
            now,
        );
        app.apply(
            AppEvent::PeekChunk {
                terminal: "9".to_string(),
                text: "other terminal".to_string(),
            },
            now,
        );
        let peek = app.peek.as_ref().expect("peek state");
        let rows: Vec<String> = peek.parser.screen().rows(0, 80).collect();
        assert_eq!(rows[0].trim_end(), "first line");
        assert_eq!(rows[1].trim_end(), "second line");
    }

    #[test]
    fn peek_screen_does_not_wrap_below_its_pane_width() {
        // A 177-column frame gives a 140-column peek parser, so a 120-char line
        // fits on one row without the parser wrapping it.
        let now = Instant::now();
        let mut app = App::new("icrn".to_string(), Default::default(), 999, (177, 30));
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                op: 1,
                username: "ww41".to_string(),
                servers: vec![row("", true)],
            },
            now,
        );
        app.on_key(&press(KeyCode::Enter), now);
        let op = match app.take_effects().as_slice() {
            [Effect::FetchTerminals { op, .. }] => *op,
            other => panic!("unexpected effects: {other:?}"),
        };
        app.apply(
            AppEvent::Terminals {
                op,
                server: "default".to_string(),
                terminals: terminals(&["1"]),
            },
            now,
        );
        let _ = app.take_effects();
        app.tick(now + PEEK_DEBOUNCE);
        let cols = match app.take_effects().as_slice() {
            [Effect::PeekStart { cols, .. }] => *cols,
            other => panic!("unexpected effects: {other:?}"),
        };
        assert_eq!(cols, 140);
        let op = *app.ops.keys().next_back().expect("connecting op");
        app.apply(
            AppEvent::PeekOpened {
                op,
                terminal: "1".to_string(),
            },
            now,
        );
        let line = "x".repeat(120);
        app.apply(
            AppEvent::PeekChunk {
                terminal: "1".to_string(),
                text: line.clone(),
            },
            now,
        );
        let peek = app.peek.as_ref().expect("peek state");
        let rows: Vec<String> = peek.parser.screen().rows(0, cols).collect();
        assert_eq!(rows[0].trim_end(), line, "a 120-char line stays on one row");
        assert_eq!(rows[1].trim_end(), "", "nothing wraps onto the next row");
    }

    #[test]
    fn peek_failure_sets_the_error_and_closes_the_op() {
        let (mut app, now) = committed_app(&["1"]);
        let _ = app.take_effects();
        app.tick(now + PEEK_DEBOUNCE);
        let _ = app.take_effects();
        let op = *app.ops.keys().next_back().expect("connecting op");
        app.apply(
            AppEvent::PeekFailed {
                op,
                terminal: "1".to_string(),
                message: "connection refused".to_string(),
            },
            now,
        );
        assert!(app.ops.is_empty());
        assert_eq!(
            app.peek.as_ref().and_then(|p| p.error.as_deref()),
            Some("connection refused")
        );
    }

    #[test]
    fn reject_flash_clears_after_its_duration() {
        let (mut app, now) = fresh_app();
        app.dialog = Some(crate::tui::dialogs::Dialog::CreateNamed(
            crate::tui::dialogs::CreateNamedDialog::new(&app.presets),
        ));
        app.on_key(&press(KeyCode::Enter), now); // empty name -> flash set
        assert!(matches!(
            &app.dialog,
            Some(crate::tui::dialogs::Dialog::CreateNamed(d)) if d.flash.is_some()
        ));
        app.tick(now + REJECT_FLASH_DURATION + Duration::from_millis(1));
        assert!(matches!(
            &app.dialog,
            Some(crate::tui::dialogs::Dialog::CreateNamed(d)) if d.flash.is_none()
        ));
    }

    #[test]
    fn attach_emits_peekstop_before_attach() {
        let (mut app, now) = committed_app(&["2"]);
        let _ = app.take_effects();
        app.tick(now + PEEK_DEBOUNCE);
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Enter), now + Duration::from_millis(400));
        let effects = app.take_effects();
        assert!(matches!(
            effects.as_slice(),
            [Effect::PeekStop, Effect::Attach { target }] if target == ":2"
        ));
    }

    #[test]
    fn cursor_reaches_the_synthetic_row_and_n_opens_create_named() {
        let (mut app, now) = fresh_app();
        // servers = [default, backup, lab]; the synthetic row is index 3.
        for _ in 0..5 {
            app.on_key(&press(KeyCode::Down), now);
        }
        assert_eq!(app.server_cursor, app.servers.len());
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(matches!(
            app.dialog,
            Some(crate::tui::dialogs::Dialog::CreateNamed(_))
        ));
    }

    #[test]
    fn x_on_the_synthetic_row_does_nothing() {
        let (mut app, now) = fresh_app();
        for _ in 0..5 {
            app.on_key(&press(KeyCode::Down), now);
        }
        assert_eq!(app.server_cursor, app.servers.len());
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('x')), now);
        assert!(app.dialog.is_none());
        assert!(app.take_effects().is_empty());
    }
}
