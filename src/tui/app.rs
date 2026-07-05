use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::config::JsonMap;

pub const STATUS_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Servers,
    Shells,
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
pub struct ShellRow {
    pub name: String,
    pub last_activity: Option<String>,
}

#[derive(Debug)]
pub enum AppEvent {
    Refreshed {
        username: String,
        servers: Vec<ServerRow>,
    },
    Shells {
        server: String,
        shells: Vec<ShellRow>,
    },
    Progress {
        message: String,
    },
    OpDone {
        message: String,
    },
    OpFailed {
        message: String,
    },
}

#[derive(Debug)]
pub enum Effect {
    Refresh,
    FetchShells {
        server: String,
        url: String,
    },
    Start {
        server: Option<String>,
        options: JsonMap,
    },
    Stop {
        server: Option<String>,
    },
    NewShell {
        server: String,
        url: String,
    },
    KillShell {
        server: String,
        url: String,
        shell: String,
    },
    Attach {
        target: String,
    },
    Quit,
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
    pub selected_server: usize,
    pub shells: Vec<ShellRow>,
    pub selected_shell: usize,
    pub focus: Focus,
    // Task 6 replaces the unit type with dialogs::Dialog.
    pub dialog: Option<()>,
    pub status: Option<StatusMsg>,
    pub presets: BTreeMap<String, JsonMap>,
    pub loading: bool,
    effects: Vec<Effect>,
}

impl App {
    pub fn new(hub_name: String, presets: BTreeMap<String, JsonMap>) -> Self {
        Self {
            hub_name,
            username: None,
            servers: Vec::new(),
            selected_server: 0,
            shells: Vec::new(),
            selected_shell: 0,
            focus: Focus::Servers,
            dialog: None,
            status: None,
            presets,
            loading: true,
            effects: vec![Effect::Refresh],
        }
    }

    pub fn selected_server(&self) -> Option<&ServerRow> {
        self.servers.get(self.selected_server)
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
    }

    fn attach_target(server: &ServerRow, shell: &ShellRow) -> String {
        format!("{}:{}", server.name, shell.name)
    }

    fn fetch_shells_for_selection(&mut self) {
        self.shells.clear();
        self.selected_shell = 0;
        if let Some(server) = self.selected_server()
            && server.ready
            && let Some(url) = &server.url
        {
            self.effects.push(Effect::FetchShells {
                server: server.display.clone(),
                url: url.clone(),
            });
        }
    }

    pub fn apply(&mut self, event: AppEvent, now: Instant) {
        match event {
            AppEvent::Refreshed { username, servers } => {
                let keep = self.selected_server().map(|s| s.display.clone());
                self.username = Some(username);
                self.servers = servers;
                self.loading = false;
                self.selected_server = keep
                    .and_then(|d| self.servers.iter().position(|s| s.display == d))
                    .unwrap_or(0);
                self.fetch_shells_for_selection();
            }
            AppEvent::Shells { server, shells } => {
                let current = self.selected_server().map(|s| s.display.clone());
                if current.as_deref() == Some(server.as_str()) {
                    self.shells = shells;
                    self.selected_shell =
                        self.selected_shell.min(self.shells.len().saturating_sub(1));
                }
            }
            AppEvent::Progress { message } => self.set_status(message, false, now),
            AppEvent::OpDone { message } => {
                self.set_status(message, false, now);
                self.effects.push(Effect::Refresh);
            }
            AppEvent::OpFailed { message } => self.set_status(message, true, now),
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        // Task 6 routes keys to an open dialog here, before dashboard handling.
        match key.code {
            KeyCode::Char('q') => self.effects.push(Effect::Quit),
            KeyCode::Char('r') => {
                self.loading = true;
                self.effects.push(Effect::Refresh);
            }
            KeyCode::Left => self.focus = Focus::Servers,
            KeyCode::Right if !self.shells.is_empty() => self.focus = Focus::Shells,
            KeyCode::Up | KeyCode::Down => self.move_selection(key.code == KeyCode::Down),
            KeyCode::Char('n') => match self.selected_server() {
                Some(server) if server.ready && server.url.is_some() => {
                    let effect = Effect::NewShell {
                        server: server.display.clone(),
                        url: server.url.clone().expect("checked is_some above"),
                    };
                    self.effects.push(effect);
                }
                _ => self.set_status(
                    "the selected server is not ready; start it first".to_string(),
                    true,
                    now,
                ),
            },
            KeyCode::Enter => self.on_enter(now),
            _ => {}
        }
    }

    fn move_selection(&mut self, down: bool) {
        match self.focus {
            Focus::Servers => {
                let len = self.servers.len();
                if len == 0 {
                    return;
                }
                let before = self.selected_server;
                self.selected_server = if down {
                    (self.selected_server + 1).min(len - 1)
                } else {
                    self.selected_server.saturating_sub(1)
                };
                if self.selected_server != before {
                    self.fetch_shells_for_selection();
                }
            }
            Focus::Shells => {
                let len = self.shells.len();
                if len == 0 {
                    return;
                }
                self.selected_shell = if down {
                    (self.selected_shell + 1).min(len - 1)
                } else {
                    self.selected_shell.saturating_sub(1)
                };
            }
        }
    }

    fn on_enter(&mut self, _now: Instant) {
        match self.focus {
            Focus::Servers => {
                if self.selected_server().is_some_and(|s| s.ready) && !self.shells.is_empty() {
                    self.focus = Focus::Shells;
                }
                // Task 6: Enter on a stopped default server opens the start dialog.
            }
            Focus::Shells => {
                if let (Some(server), Some(shell)) = (
                    self.servers.get(self.selected_server),
                    self.shells.get(self.selected_shell),
                ) {
                    self.effects.push(Effect::Attach {
                        target: Self::attach_target(server, shell),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn row(name: &str, ready: bool) -> ServerRow {
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

    fn refreshed_app() -> (App, Instant) {
        let now = Instant::now();
        let mut app = App::new("icrn".to_string(), Default::default());
        let _ = app.take_effects(); // discard the initial Refresh
        app.apply(
            AppEvent::Refreshed {
                username: "ww41".to_string(),
                servers: vec![row("", true), row("backup", true)],
            },
            now,
        );
        (app, now)
    }

    #[test]
    fn new_queues_initial_refresh() {
        let mut app = App::new("icrn".to_string(), Default::default());
        assert!(matches!(app.take_effects().as_slice(), [Effect::Refresh]));
        assert!(app.take_effects().is_empty());
    }

    #[test]
    fn refresh_apply_fetches_shells_for_selected_ready_server() {
        let (mut app, _) = refreshed_app();
        let effects = app.take_effects();
        assert!(matches!(
            effects.as_slice(),
            [Effect::FetchShells { server, .. }] if server == "default"
        ));
    }

    #[test]
    fn selection_move_refetches_shells() {
        let (mut app, now) = refreshed_app();
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Down), now);
        let effects = app.take_effects();
        assert!(matches!(
            effects.as_slice(),
            [Effect::FetchShells { server, .. }] if server == "backup"
        ));
    }

    #[test]
    fn stale_shells_event_is_ignored() {
        let (mut app, now) = refreshed_app();
        app.apply(
            AppEvent::Shells {
                server: "backup".to_string(),
                shells: vec![ShellRow {
                    name: "1".to_string(),
                    last_activity: None,
                }],
            },
            now,
        );
        assert!(
            app.shells.is_empty(),
            "shells for an unselected server must be dropped"
        );
    }

    #[test]
    fn enter_on_shell_queues_attach_with_addressing() {
        let (mut app, now) = refreshed_app();
        let _ = app.take_effects();
        app.apply(
            AppEvent::Shells {
                server: "default".to_string(),
                shells: vec![ShellRow {
                    name: "2".to_string(),
                    last_activity: None,
                }],
            },
            now,
        );
        app.on_key(&press(KeyCode::Right), now); // focus shells
        app.on_key(&press(KeyCode::Enter), now);
        let effects = app.take_effects();
        assert!(matches!(
            effects.as_slice(),
            [Effect::Attach { target }] if target == ":2"
        ));
    }

    #[test]
    fn q_quits_and_refresh_preserves_selection_by_name() {
        let (mut app, now) = refreshed_app();
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Down), now); // select backup
        let _ = app.take_effects();
        app.apply(
            AppEvent::Refreshed {
                username: "ww41".to_string(),
                servers: vec![row("", true), row("backup", true)],
            },
            now,
        );
        assert_eq!(app.selected_server().unwrap().display, "backup");
        app.on_key(&press(KeyCode::Char('q')), now);
        assert!(matches!(app.take_effects().last(), Some(Effect::Quit)));
    }

    #[test]
    fn status_expires_after_ttl() {
        let (mut app, now) = refreshed_app();
        app.apply(
            AppEvent::OpFailed {
                message: "boom".to_string(),
            },
            now,
        );
        assert!(app.status.as_ref().is_some_and(|s| s.error));
        app.tick(now + Duration::from_secs(4));
        assert!(app.status.is_some());
        app.tick(now + STATUS_TTL + Duration::from_millis(1));
        assert!(app.status.is_none());
    }

    #[test]
    fn n_on_ready_server_queues_new_shell_and_errors_when_not_ready() {
        let (mut app, now) = refreshed_app();
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(matches!(
            app.take_effects().as_slice(),
            [Effect::NewShell { server, .. }] if server == "default"
        ));
        app.apply(
            AppEvent::Refreshed {
                username: "ww41".to_string(),
                servers: vec![row("", false)],
            },
            now,
        );
        let _ = app.take_effects();
        app.on_key(&press(KeyCode::Char('n')), now);
        assert!(app.take_effects().is_empty());
        assert!(app.status.as_ref().is_some_and(|s| s.error));
    }
}
