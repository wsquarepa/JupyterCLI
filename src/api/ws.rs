use super::error::ApiError;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

#[derive(Debug)]
pub enum TermFrame {
    Setup,
    Stdout(String),
    Disconnect,
    Unknown(String),
}

pub fn decode_frame(text: &str) -> Result<TermFrame, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("frame is not JSON: {e}"))?;
    let arr = value.as_array().ok_or("frame is not a JSON array")?;
    let kind = arr
        .first()
        .and_then(|k| k.as_str())
        .ok_or("frame has no string kind")?;
    match kind {
        "setup" => Ok(TermFrame::Setup),
        "stdout" => {
            let text = arr
                .get(1)
                .and_then(|t| t.as_str())
                .ok_or("stdout frame has no payload")?;
            Ok(TermFrame::Stdout(text.to_string()))
        }
        "disconnect" => Ok(TermFrame::Disconnect),
        other => Ok(TermFrame::Unknown(other.to_string())),
    }
}

pub fn encode_stdin(text: &str) -> String {
    serde_json::json!(["stdin", text]).to_string()
}

pub fn encode_set_size(rows: u16, cols: u16) -> String {
    serde_json::json!(["set_size", rows, cols]).to_string()
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub struct TermSocket {
    ws: WsStream,
    url: String,
}

impl TermSocket {
    pub async fn connect(url: &str, token: &str) -> Result<Self, ApiError> {
        let mut request = url.into_client_request().map_err(|e| ApiError::Ws {
            url: url.to_string(),
            source: Box::new(e),
        })?;
        let auth = format!("token {token}")
            .parse()
            .map_err(|_| ApiError::BadUrl {
                url: url.to_string(),
                reason: "token contains characters invalid in a header".to_string(),
            })?;
        request.headers_mut().insert("Authorization", auth);
        // A 404 on the upgrade is how a stale terminal id surfaces: terminado keeps no
        // record of terminals from before a server restart, so the handshake rejection
        // is the only signal that the shell is gone.
        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| match e {
                tokio_tungstenite::tungstenite::Error::Http(ref resp)
                    if resp.status().as_u16() == 404 =>
                {
                    ApiError::ShellNotFound {
                        url: url.to_string(),
                    }
                }
                e => ApiError::Ws {
                    url: url.to_string(),
                    source: Box::new(e),
                },
            })?;
        tracing::debug!(target: "jhc::ws", url = %url, "connect");
        Ok(Self {
            ws,
            url: url.to_string(),
        })
    }

    fn ws_err(&self, e: tokio_tungstenite::tungstenite::Error) -> ApiError {
        ApiError::Ws {
            url: self.url.clone(),
            source: Box::new(e),
        }
    }

    pub async fn send_stdin(&mut self, text: &str) -> Result<(), ApiError> {
        let frame = encode_stdin(text);
        tracing::trace!(target: "jhc::ws", direction = "send", bytes = frame.len(), "stdin");
        self.ws
            .send(Message::Text(frame.into()))
            .await
            .map_err(|e| self.ws_err(e))
    }

    pub async fn send_size(&mut self, rows: u16, cols: u16) -> Result<(), ApiError> {
        let frame = encode_set_size(rows, cols);
        tracing::trace!(target: "jhc::ws", direction = "send", bytes = frame.len(), "set_size");
        self.ws
            .send(Message::Text(frame.into()))
            .await
            .map_err(|e| self.ws_err(e))
    }

    pub async fn ping(&mut self) -> Result<(), ApiError> {
        tracing::trace!(target: "jhc::ws", direction = "send", bytes = 0, "ping");
        self.ws
            .send(Message::Ping(Vec::new().into()))
            .await
            .map_err(|e| self.ws_err(e))
    }

    pub async fn next_frame(&mut self) -> Result<Option<TermFrame>, ApiError> {
        loop {
            match self.ws.next().await {
                None => return Ok(None),
                Some(Err(e)) => return Err(self.ws_err(e)),
                Some(Ok(Message::Text(text))) => {
                    tracing::trace!(target: "jhc::ws", direction = "recv", bytes = text.len(), "frame");
                    let frame = decode_frame(&text).map_err(|reason| ApiError::Protocol {
                        url: self.url.clone(),
                        reason,
                    })?;
                    return Ok(Some(frame));
                }
                Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Ok(_)) => continue,
            }
        }
    }

    pub async fn finish(mut self) -> Result<(), ApiError> {
        self.ws.flush().await.map_err(|e| ApiError::Ws {
            url: self.url.clone(),
            source: Box::new(e),
        })?;
        tracing::debug!(target: "jhc::ws", url = %self.url, "close");
        self.ws.close(None).await.map_err(|e| ApiError::Ws {
            url: self.url.clone(),
            source: Box::new(e),
        })?;
        while let Some(msg) = self.ws.next().await {
            match msg {
                Ok(_) => continue,
                Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed) => break,
                Err(e) => {
                    return Err(ApiError::Ws {
                        url: self.url,
                        source: Box::new(e),
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_known_frames() {
        assert!(matches!(
            decode_frame("[\"setup\", {}]").unwrap(),
            TermFrame::Setup
        ));
        match decode_frame("[\"stdout\", \"hi\\r\\n\"]").unwrap() {
            TermFrame::Stdout(s) => assert_eq!(s, "hi\r\n"),
            other => panic!("wrong frame: {other:?}"),
        }
        assert!(matches!(
            decode_frame("[\"disconnect\", 1]").unwrap(),
            TermFrame::Disconnect
        ));
        assert!(matches!(
            decode_frame("[\"mystery\", 1]").unwrap(),
            TermFrame::Unknown(_)
        ));
    }

    #[test]
    fn rejects_malformed_frames() {
        assert!(decode_frame("not json").is_err());
        assert!(decode_frame("{}").is_err());
        assert!(decode_frame("[]").is_err());
    }

    #[tokio::test]
    async fn connect_upgrade_404_maps_to_stale_shell_error() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
            stream.flush().await.unwrap();
        });
        let url = format!("ws://{addr}/user/ww41/terminals/websocket/7");
        let err = match TermSocket::connect(&url, "tok").await {
            Ok(_) => panic!("connect against a 404 upgrade must fail"),
            Err(e) => e,
        };
        assert!(matches!(err, ApiError::ShellNotFound { .. }), "got: {err}");
        let msg = err.to_string();
        assert!(msg.contains("server restart"), "got: {msg}");
        assert!(msg.contains("jhc shell list"), "got: {msg}");
    }

    #[test]
    fn encodes_stdin_and_size() {
        assert_eq!(encode_stdin("ls\n"), "[\"stdin\",\"ls\\n\"]");
        assert_eq!(encode_set_size(24, 80), "[\"set_size\",24,80]");
    }
}
