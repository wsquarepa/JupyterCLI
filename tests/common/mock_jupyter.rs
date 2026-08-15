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
    // Set once an exec of `bash -s` arrives: the exit sentinel is then withheld until the
    // Ctrl-D that jhc sends at stdin EOF, so a test can prove the piped bytes were forwarded
    // before the command finished. Other exec lines keep the original immediate replies.
    let mut awaiting_eof_nonce: Option<String> = None;
    while let Some(Ok(msg)) = ws.next().await {
        if let Message::Text(text) = msg {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let arr = value.as_array().unwrap();
            if arr[0] == "stdin" {
                let payload = arr[1].as_str().unwrap();
                if let Some(pos) = payload.find("printf '\\036") {
                    let nonce_start = pos + 12;
                    let nonce = payload[nonce_start..nonce_start + 16].to_string();
                    if payload.contains("'bash' '-s'") {
                        for out in [format!("{payload}\r\n"), format!("\x1e{nonce}:S\x1e")] {
                            let frame = serde_json::json!(["stdout", out]).to_string();
                            ws.send(Message::Text(frame.into())).await.unwrap();
                        }
                        awaiting_eof_nonce = Some(nonce);
                    } else if payload.contains(".jhc/jobs") {
                        let (body, code) = job_reply(payload);
                        for out in [
                            format!("{payload}\r\n"),
                            format!("\x1e{nonce}:S\x1e"),
                            body,
                            format!("\x1e{nonce}:{code}\x1e"),
                        ] {
                            let frame = serde_json::json!(["stdout", out]).to_string();
                            ws.send(Message::Text(frame.into())).await.unwrap();
                        }
                    } else {
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
                } else if let Some(nonce) = awaiting_eof_nonce.as_ref() {
                    let out = if payload == "\x04" {
                        let sentinel = format!("\x1e{nonce}:0\x1e");
                        awaiting_eof_nonce = None;
                        sentinel
                    } else {
                        format!("got:{payload}")
                    };
                    let frame = serde_json::json!(["stdout", out]).to_string();
                    ws.send(Message::Text(frame.into())).await.unwrap();
                }
            }
        }
    }
}

fn probe_record(
    id: &str,
    exit: &str,
    alive: &str,
    name: serde_json::Value,
    command: &str,
) -> String {
    use base64::Engine as _;
    let meta = serde_json::json!({"id": id, "name": name, "command": command});
    let b64 = base64::engine::general_purpose::STANDARD.encode(meta.to_string());
    format!("{id}\x1f{exit}\x1f{alive}\x1f2026-08-14T10:00:00Z\x1f{b64}\r\n")
}

/// Canned remote answers for the job scripts, keyed on distinctive substrings.
/// Match order matters: probes contain `kill -0`, so they must match before the
/// kill arm; the fallthrough 127 makes an unmatched script an obvious test failure.
fn job_reply(payload: &str) -> (String, i32) {
    let running = || {
        probe_record(
            "aaaaaaaa",
            "-",
            "1",
            serde_json::json!("vllm"),
            "'sleep' '999'",
        )
    };
    let exited = || probe_record("bbbbbbbb", "7", "0", serde_json::json!(null), "'false'");
    if payload.contains("for d in") {
        if payload.contains("jobs/aaaaaaaa") {
            (running(), 0)
        } else if payload.contains("jobs/bbbbbbbb") {
            (exited(), 0)
        } else if payload.contains("jobs/eeeeeeee") {
            (String::new(), 0)
        } else {
            (format!("{}{}", running(), exited()), 0)
        }
    } else if payload.contains("mkdir -p") {
        (String::new(), 0)
    } else if payload.contains("tail -c") {
        if payload.contains("jobs/eeeeeeee") {
            (String::new(), 66)
        } else {
            ("job log line\r\n".to_string(), 0)
        }
    } else if payload.contains("kill -TERM")
        || payload.contains("kill -KILL")
        || payload.contains("rm -rf")
    {
        (String::new(), 0)
    } else {
        (String::new(), 127)
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
            r#"{"name":"ww41","servers":{"":{"name":"","ready":true,"url":"/user/ww41/","started":"2026-08-14T09:00:00Z","user_options":{}}}}"#,
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
