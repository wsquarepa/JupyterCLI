use std::net::SocketAddr;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

/// A single listener that answers both the JupyterHub REST calls an `exec` makes and the
/// terminado WebSocket, so the compiled `jhc` binary can run a full exec end to end.
pub struct MockJupyter {
    addr: SocketAddr,
}

impl MockJupyter {
    pub async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream));
            }
        });
        Self { addr }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

async fn serve(stream: TcpStream) {
    let head = peek_head(&stream).await;
    let is_ws = head.contains("GET /user/ww41/terminals/websocket/")
        && head.to_ascii_lowercase().contains("upgrade:");
    if is_ws {
        serve_terminado(stream).await;
    } else {
        serve_http(stream, &head).await;
    }
}

/// Read the request head without consuming it, so the WebSocket path can hand the raw stream
/// to `accept_async` (which performs the handshake itself). Small localhost requests arrive in
/// one segment, so a single completed peek carries the whole head.
async fn peek_head(stream: &TcpStream) -> String {
    let mut buf = vec![0u8; 65536];
    loop {
        let n = stream.peek(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf[..n]).to_string();
        if n == 0 || text.contains("\r\n\r\n") {
            return text;
        }
    }
}

async fn serve_terminado(stream: TcpStream) {
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    ws.send(Message::Text("[\"setup\", {}]".to_string().into()))
        .await
        .unwrap();
    while let Some(Ok(msg)) = ws.next().await {
        if let Message::Text(text) = msg {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let arr = value.as_array().unwrap();
            if arr[0] == "stdin" {
                let payload = arr[1].as_str().unwrap();
                if let Some(pos) = payload.find("printf '\\036") {
                    let nonce_start = pos + 12;
                    let nonce = &payload[nonce_start..nonce_start + 16];
                    for out in [
                        format!("{payload}\r\n"),
                        format!("\x1e{nonce}:S\x1e"),
                        "hi\r\n".to_string(),
                        format!("\x1e{nonce}:0\x1e"),
                    ] {
                        let frame = serde_json::json!(["stdout", out]).to_string();
                        ws.send(Message::Text(frame.into())).await.unwrap();
                    }
                }
            }
        }
    }
}

async fn serve_http(mut stream: TcpStream, head: &str) {
    // Drain the peeked request. Closing a socket with unread bytes sends RST instead of FIN,
    // which can truncate the response before the client reads it. The head is ASCII and these
    // requests carry no body, so its byte length is exactly what is buffered.
    let mut scratch = vec![0u8; head.len()];
    stream.read_exact(&mut scratch).await.unwrap();

    let (status, body): (&str, &str) = if head.starts_with("GET /hub/api/user ") {
        (
            "200 OK",
            r#"{"name":"ww41","servers":{"":{"name":"","ready":true,"url":"/user/ww41/","user_options":{}}}}"#,
        )
    } else if head.starts_with("POST /user/ww41/api/terminals ") {
        ("200 OK", r#"{"name":"1"}"#)
    } else if head.starts_with("DELETE /user/ww41/api/terminals/1 ") {
        ("204 No Content", "")
    } else {
        ("404 Not Found", "")
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
}
