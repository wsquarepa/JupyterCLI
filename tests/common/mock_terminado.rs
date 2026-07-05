use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

type StdinHandler = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

pub struct MockTerminado {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<String>>>,
}

impl MockTerminado {
    pub async fn spawn(
        replay: &str,
        on_stdin: impl Fn(&str) -> Vec<String> + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let handler: StdinHandler = Arc::new(on_stdin);
        let replay = replay.to_string();
        let received_clone = received.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let replay = replay.clone();
                let handler = handler.clone();
                let received = received_clone.clone();
                tokio::spawn(async move {
                    serve(stream, replay, handler, received).await;
                });
            }
        });
        Self { addr, received }
    }

    pub fn url(&self) -> String {
        format!("ws://{}/", self.addr)
    }

    pub fn received(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }
}

async fn serve(
    stream: TcpStream,
    replay: String,
    handler: StdinHandler,
    received: Arc<Mutex<Vec<String>>>,
) {
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    ws.send(Message::Text("[\"setup\", {}]".to_string().into()))
        .await
        .unwrap();
    if !replay.is_empty() {
        let frame = serde_json::json!(["stdout", replay]).to_string();
        ws.send(Message::Text(frame.into())).await.unwrap();
    }
    // On the peer's Close, tungstenite auto-queues the reply; keep polling so it flushes
    // and the close handshake completes before this task drops the socket. Dropping early
    // resets the TCP connection, which the client observes as ResetWithoutClosingHandshake.
    while let Some(Ok(msg)) = ws.next().await {
        if let Message::Text(text) = msg {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let arr = value.as_array().unwrap();
            if arr[0] == "stdin" {
                let payload = arr[1].as_str().unwrap().to_string();
                received.lock().unwrap().push(payload.clone());
                for out in handler(&payload) {
                    let frame = serde_json::json!(["stdout", out]).to_string();
                    ws.send(Message::Text(frame.into())).await.unwrap();
                }
            }
        }
    }
}
