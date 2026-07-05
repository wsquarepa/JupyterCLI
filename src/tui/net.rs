use tokio::sync::mpsc::UnboundedSender;

use crate::api::HubClient;
use crate::api::error::ApiError;
use crate::api::server::ServerClient;
use crate::api::types::User;

use super::app::{AppEvent, Effect, ServerRow, ShellRow};

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
        let event = match effect {
            Effect::Refresh => refresh(&client).await,
            Effect::FetchShells { server, url } => fetch_shells(&client, server, &url).await,
            Effect::Start { server, options } => {
                start(&client, server.as_deref(), options, &tx).await
            }
            Effect::Stop { server } => stop(&client, server.as_deref()).await,
            Effect::NewShell { server, url } => new_shell(&client, &server, &url).await,
            Effect::KillShell { server, url, shell } => {
                kill_shell(&client, &server, &url, &shell).await
            }
            Effect::Attach { .. } | Effect::Quit => {
                unreachable!("attach and quit are handled by the event loop, not net::dispatch")
            }
        };
        let event = event.unwrap_or_else(|e| AppEvent::OpFailed {
            message: e.to_string(),
        });
        // A send failure means the UI is shutting down; nothing left to report to.
        let _ = tx.send(event);
    });
}

async fn refresh(client: &HubClient) -> Result<AppEvent, ApiError> {
    let user = client.whoami().await?;
    Ok(AppEvent::Refreshed {
        username: user.name.clone(),
        servers: rows_from_user(&user),
    })
}

async fn fetch_shells(client: &HubClient, server: String, url: &str) -> Result<AppEvent, ApiError> {
    let sc = ServerClient::from_hub(client, url)?;
    let shells = sc
        .terminals()
        .await?
        .into_iter()
        .map(|t| ShellRow {
            name: t.name,
            last_activity: t.last_activity,
        })
        .collect();
    Ok(AppEvent::Shells { server, shells })
}

async fn start(
    client: &HubClient,
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
        message: "server ready".to_string(),
    })
}

async fn stop(client: &HubClient, server: Option<&str>) -> Result<AppEvent, ApiError> {
    let user = client.whoami().await?;
    client.stop(&user.name, server).await?;
    Ok(AppEvent::OpDone {
        message: format!(
            "stop requested for {}",
            server.unwrap_or("the default server")
        ),
    })
}

async fn new_shell(client: &HubClient, server: &str, url: &str) -> Result<AppEvent, ApiError> {
    let sc = ServerClient::from_hub(client, url)?;
    let terminal = sc.create_terminal().await?;
    Ok(AppEvent::OpDone {
        message: format!("created shell {} on {server}", terminal.name),
    })
}

async fn kill_shell(
    client: &HubClient,
    server: &str,
    url: &str,
    shell: &str,
) -> Result<AppEvent, ApiError> {
    let sc = ServerClient::from_hub(client, url)?;
    sc.delete_terminal(shell).await?;
    Ok(AppEvent::OpDone {
        message: format!("killed shell {shell} on {server}"),
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
        let client = HubClient::new(&server.uri(), "tok").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        dispatch(Effect::Refresh, client, tx);
        match rx.recv().await.unwrap() {
            AppEvent::Refreshed { username, servers } => {
                assert_eq!(username, "ww41");
                assert_eq!(servers.len(), 2);
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
            AppEvent::OpDone { message } => assert!(message.contains("ready")),
            other => panic!("unexpected event: {other:?}"),
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
        dispatch(Effect::Refresh, client, tx);
        match rx.recv().await.unwrap() {
            AppEvent::OpFailed { message } => assert!(message.contains("scope")),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
