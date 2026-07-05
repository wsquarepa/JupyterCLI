use tokio::sync::mpsc::UnboundedSender;

use crate::api::HubClient;
use crate::api::error::ApiError;
use crate::api::server::ServerClient;
use crate::api::types::User;
use crate::api::ws::TermSocket;

use super::app::{AppEvent, Effect, ServerRow, TerminalRow};

pub fn rows_from_user(user: &User) -> Vec<ServerRow> {
    let mut rows: Vec<ServerRow> = Vec::new();
    match user.servers.get("") {
        Some(server) => rows.push(ServerRow {
            name: String::new(),
            display: "default".to_string(),
            ready: server.ready,
            pending: server.pending.clone(),
            options: server.user_options.clone(),
            url: server.url.clone(),
        }),
        None => rows.push(ServerRow {
            name: String::new(),
            display: "default".to_string(),
            ready: false,
            pending: None,
            options: Default::default(),
            url: None,
        }),
    }
    let mut named: Vec<&crate::api::types::Server> = user
        .servers
        .iter()
        .filter(|(k, _)| !k.is_empty())
        .map(|(_, v)| v)
        .collect();
    named.sort_by(|a, b| a.name.cmp(&b.name));
    for server in named {
        rows.push(ServerRow {
            name: server.name.clone(),
            display: server.name.clone(),
            ready: server.ready,
            pending: server.pending.clone(),
            options: server.user_options.clone(),
            url: server.url.clone(),
        });
    }
    rows
}

pub fn dispatch(effect: Effect, client: HubClient, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let op = match &effect {
            Effect::Refresh { op }
            | Effect::FetchTerminals { op, .. }
            | Effect::Start { op, .. }
            | Effect::Stop { op, .. }
            | Effect::NewTerminal { op, .. }
            | Effect::KillTerminal { op, .. } => *op,
            Effect::PeekStart { .. }
            | Effect::PeekStop
            | Effect::Attach { .. }
            | Effect::Quit
            | Effect::SavePreset { .. } => {
                unreachable!("local effects are handled by the event loop, not net::dispatch")
            }
        };
        let event = match effect {
            Effect::Refresh { op } => refresh(&client, op).await,
            Effect::FetchTerminals { op, server, url } => {
                fetch_terminals(&client, op, server, &url).await
            }
            Effect::Start {
                op,
                server,
                options,
            } => start(&client, op, server.as_deref(), options, &tx).await,
            Effect::Stop { op, server } => stop(&client, op, server.as_deref()).await,
            Effect::NewTerminal { op, server, url } => {
                new_terminal(&client, op, &server, &url).await
            }
            Effect::KillTerminal {
                op,
                server,
                url,
                terminal,
            } => kill_terminal(&client, op, &server, &url, &terminal).await,
            Effect::PeekStart { .. }
            | Effect::PeekStop
            | Effect::Attach { .. }
            | Effect::Quit
            | Effect::SavePreset { .. } => {
                unreachable!("local effects are handled by the event loop, not net::dispatch")
            }
        };
        let event = event.unwrap_or_else(|e| AppEvent::OpFailed {
            op,
            message: e.to_string(),
        });
        // A send failure means the UI is shutting down; nothing left to report to.
        let _ = tx.send(event);
    });
}

/// Follower for one terminal: connects the terminado WebSocket, forwards raw
/// stdout chunks (escape sequences intact, for the peek pane's terminal
/// emulator), and pings every 30 s of silence so idle proxies keep the
/// connection alive. Sends exactly one set_size on connect to force a SIGWINCH
/// repaint and reflow the PTY to the peek pane (a deliberate reversal of the
/// earlier read-only rule: attach later restores the user's real size on its
/// own connect, see src/attach.rs:104). It NEVER sends stdin. The caller owns
/// the returned handle and aborts it to stop following.
pub fn spawn_peek(
    op: u64,
    url: String,
    terminal: String,
    rows: u16,
    cols: u16,
    client: HubClient,
    tx: UnboundedSender<AppEvent>,
) -> tokio::task::AbortHandle {
    tokio::spawn(async move {
        let connect = async {
            let sc = ServerClient::from_hub(&client, &url)?;
            let ws_url = sc.ws_terminal_url(&terminal)?;
            TermSocket::connect(&ws_url, client.token()).await
        };
        let mut sock = match connect.await {
            Ok(sock) => sock,
            Err(e) => {
                let _ = tx.send(AppEvent::PeekFailed {
                    op,
                    terminal,
                    message: e.to_string(),
                });
                return;
            }
        };
        if let Err(e) = sock.send_size(rows, cols).await {
            let _ = tx.send(AppEvent::PeekFailed {
                op,
                terminal,
                message: e.to_string(),
            });
            return;
        }
        let _ = tx.send(AppEvent::PeekOpened {
            op,
            terminal: terminal.clone(),
        });
        loop {
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(30), sock.next_frame()).await;
            match frame {
                Err(_) => {
                    if sock.ping().await.is_err() {
                        break;
                    }
                }
                Ok(Ok(Some(crate::api::ws::TermFrame::Stdout(text)))) => {
                    if !text.is_empty() {
                        let _ = tx.send(AppEvent::PeekChunk {
                            terminal: terminal.clone(),
                            text,
                        });
                    }
                }
                Ok(Ok(Some(_))) => continue,
                Ok(Ok(None)) | Ok(Err(_)) => break,
            }
        }
    })
    .abort_handle()
}

async fn refresh(client: &HubClient, op: u64) -> Result<AppEvent, ApiError> {
    let me = client.whoami().await?;
    let full = client.user_including_stopped(&me.name).await?;
    Ok(AppEvent::Refreshed {
        op,
        username: me.name,
        servers: rows_from_user(&full),
    })
}

async fn fetch_terminals(
    client: &HubClient,
    op: u64,
    server: String,
    url: &str,
) -> Result<AppEvent, ApiError> {
    let sc = ServerClient::from_hub(client, url)?;
    let terminals = sc
        .terminals()
        .await?
        .into_iter()
        .map(|t| TerminalRow { name: t.name })
        .collect();
    Ok(AppEvent::Terminals {
        op,
        server,
        terminals,
    })
}

async fn start(
    client: &HubClient,
    op: u64,
    server: Option<&str>,
    options: crate::config::JsonMap,
    tx: &UnboundedSender<AppEvent>,
) -> Result<AppEvent, ApiError> {
    let user = client.whoami().await?;
    client.spawn(&user.name, server, &options).await?;
    let progress_tx = tx.clone();
    client
        .wait_ready(&user.name, server, |event| {
            if let Some(message) = &event.message {
                let _ = progress_tx.send(AppEvent::Progress {
                    message: message.clone(),
                });
            }
        })
        .await?;
    Ok(AppEvent::OpDone {
        op,
        message: "server ready".to_string(),
    })
}

async fn stop(client: &HubClient, op: u64, server: Option<&str>) -> Result<AppEvent, ApiError> {
    let user = client.whoami().await?;
    client.stop(&user.name, server).await?;
    Ok(AppEvent::OpDone {
        op,
        message: format!(
            "stop requested for {}",
            server.unwrap_or("the default server")
        ),
    })
}

async fn new_terminal(
    client: &HubClient,
    op: u64,
    server: &str,
    url: &str,
) -> Result<AppEvent, ApiError> {
    let sc = ServerClient::from_hub(client, url)?;
    let terminal = sc.create_terminal().await?;
    Ok(AppEvent::TerminalCreated {
        op,
        server: server.to_string(),
        terminal: terminal.name,
    })
}

async fn kill_terminal(
    client: &HubClient,
    op: u64,
    server: &str,
    url: &str,
    terminal: &str,
) -> Result<AppEvent, ApiError> {
    let sc = ServerClient::from_hub(client, url)?;
    sc.delete_terminal(terminal).await?;
    Ok(AppEvent::OpDone {
        op,
        message: format!("killed terminal {terminal} on {server}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{AppEvent, Effect};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn user_json(ready: bool) -> serde_json::Value {
        serde_json::json!({
            "name": "ww41",
            "servers": if ready {
                serde_json::json!({
                    "": {"name": "", "ready": true, "url": "/user/ww41/", "user_options": {}},
                    "backup": {"name": "backup", "ready": true, "url": "/user/ww41/backup/", "user_options": {}}
                })
            } else {
                serde_json::json!({})
            }
        })
    }

    #[test]
    fn rows_include_synthetic_default_and_sort_default_first() {
        let user: crate::api::types::User = serde_json::from_value(user_json(false)).unwrap();
        let rows = rows_from_user(&user);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display, "default");
        assert!(!rows[0].ready);

        let user: crate::api::types::User = serde_json::from_value(user_json(true)).unwrap();
        let rows = rows_from_user(&user);
        assert_eq!(rows[0].display, "default");
        assert!(rows[0].ready);
        assert_eq!(rows[1].display, "backup");
    }

    #[tokio::test]
    async fn refresh_sends_refreshed_event() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(user_json(true)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hub/api/users/ww41"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "ww41",
                "servers": {
                    "": {"name": "", "ready": true, "url": "/user/ww41/", "user_options": {}},
                    "backup": {"name": "backup", "ready": true, "url": "/user/ww41/backup/", "user_options": {}},
                    "old": {"name": "old", "ready": false, "pending": null, "url": null, "user_options": {}}
                }
            })))
            .mount(&server)
            .await;
        let client = HubClient::new(&server.uri(), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        dispatch(Effect::Refresh { op: 0 }, client, tx);
        match rx.recv().await.unwrap() {
            AppEvent::Refreshed {
                username, servers, ..
            } => {
                assert_eq!(username, "ww41");
                assert!(servers.iter().any(|s| s.display == "old" && !s.ready));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_streams_progress_then_done() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(user_json(false)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/hub/api/users/ww41/server"))
            .and(body_json(serde_json::json!({"resource": "2_a100"})))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hub/api/users/ww41/server/progress"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/event-stream")
                    .set_body_string(
                        "data: {\"progress\": 50, \"message\": \"pod pending\"}\n\ndata: {\"progress\": 100, \"ready\": true}\n\n",
                    ),
            )
            .mount(&server)
            .await;

        let client = HubClient::new(&server.uri(), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let options: crate::config::JsonMap =
            serde_json::from_str(r#"{"resource": "2_a100"}"#).unwrap();
        dispatch(
            Effect::Start {
                op: 0,
                server: None,
                options,
            },
            client,
            tx,
        );

        match rx.recv().await.unwrap() {
            AppEvent::Progress { message } => assert!(message.contains("pod pending")),
            other => panic!("unexpected event: {other:?}"),
        }
        match rx.recv().await.unwrap() {
            AppEvent::OpDone { message, .. } => assert!(message.contains("ready")),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn named_spawn_streams_progress_then_done() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(user_json(false)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/hub/api/users/ww41/servers/gpu"))
            .and(body_json(serde_json::json!({"resource": "2_a100"})))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hub/api/users/ww41/servers/gpu/progress"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/event-stream")
                    .set_body_string(
                        "data: {\"progress\": 50, \"message\": \"pod pending\"}\n\ndata: {\"progress\": 100, \"ready\": true}\n\n",
                    ),
            )
            .mount(&server)
            .await;

        let client = HubClient::new(&server.uri(), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let options: crate::config::JsonMap =
            serde_json::from_str(r#"{"resource": "2_a100"}"#).unwrap();
        dispatch(
            Effect::Start {
                op: 5,
                server: Some("gpu".to_string()),
                options,
            },
            client,
            tx,
        );
        loop {
            match rx.recv().await.unwrap() {
                AppEvent::OpDone { op, message } => {
                    assert_eq!(op, 5);
                    assert!(message.contains("ready"));
                    break;
                }
                AppEvent::Progress { .. } => continue,
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn failures_surface_as_op_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;
        let client = HubClient::new(&server.uri(), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        dispatch(Effect::Refresh { op: 0 }, client, tx);
        match rx.recv().await.unwrap() {
            AppEvent::OpFailed { message, .. } => assert!(message.contains("scope")),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn named_spawn_failure_reports_op_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(user_json(false)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/hub/api/users/ww41/servers/gpu"))
            .respond_with(ResponseTemplate::new(400).set_body_string("already running"))
            .mount(&server)
            .await;
        let client = HubClient::new(&server.uri(), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        dispatch(
            Effect::Start {
                op: 7,
                server: Some("gpu".to_string()),
                options: Default::default(),
            },
            client,
            tx,
        );
        loop {
            match rx.recv().await.unwrap() {
                AppEvent::OpFailed { op, .. } => {
                    assert_eq!(op, 7);
                    break;
                }
                AppEvent::Progress { .. } => continue,
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message;

    /// Minimal terminado endpoint: accepts one WebSocket, sends setup plus the
    /// given stdout frames, then forwards every text frame the client sends
    /// over the returned channel while holding the socket open.
    async fn ws_terminal(
        frames: Vec<String>,
    ) -> (
        std::net::SocketAddr,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (received_tx, received_rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::Text("[\"setup\", {}]".to_string().into()))
                .await
                .unwrap();
            for frame in frames {
                ws.send(Message::Text(frame.into())).await.unwrap();
            }
            while let Some(Ok(message)) = ws.next().await {
                if let Message::Text(text) = message {
                    let _ = received_tx.send(text.to_string());
                }
            }
        });
        (addr, received_rx)
    }

    #[tokio::test]
    async fn spawn_peek_sizes_then_streams_raw_chunks() {
        let (addr, mut received) = ws_terminal(vec![
            serde_json::json!(["stdout", "\u{1b}[32mhello\u{1b}[0m\r\nworld"]).to_string(),
        ])
        .await;
        let client = HubClient::new(&format!("http://{addr}"), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = spawn_peek(
            7,
            "/user/ww41/".to_string(),
            "1".to_string(),
            12,
            140,
            client,
            tx,
        );
        match rx.recv().await.unwrap() {
            AppEvent::PeekOpened { op, terminal } => {
                assert_eq!(op, 7);
                assert_eq!(terminal, "1");
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match rx.recv().await.unwrap() {
            AppEvent::PeekChunk { text, .. } => {
                assert_eq!(text, "\u{1b}[32mhello\u{1b}[0m\r\nworld")
            }
            other => panic!("unexpected event: {other:?}"),
        }
        let frame = received.recv().await.expect("peek sends a set_size frame");
        let decoded: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(decoded, serde_json::json!(["set_size", 12, 140]));
        handle.abort();
    }

    #[tokio::test]
    async fn spawn_peek_reports_connect_failure() {
        // Bind and drop a listener to get a port that refuses connections.
        let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = dead.local_addr().unwrap();
        drop(dead);
        let client = HubClient::new(&format!("http://{addr}"), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _handle = spawn_peek(
            9,
            "/user/ww41/".to_string(),
            "1".to_string(),
            12,
            140,
            client,
            tx,
        );
        match rx.recv().await.unwrap() {
            AppEvent::PeekFailed { op, terminal, .. } => {
                assert_eq!(op, 9);
                assert_eq!(terminal, "1");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
