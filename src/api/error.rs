#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(
        "token invalid or expired: run jhc init or check ~/.config/jhc/config.toml ({method} {url} returned 401)"
    )]
    Unauthorized { method: &'static str, url: String },
    #[error(
        "token lacks the required scope for this operation ({method} {url} returned 403: {body})"
    )]
    Forbidden {
        method: &'static str,
        url: String,
        body: String,
    },
    #[error("this server does not expose the terminals API ({url} returned 404)")]
    TerminalsUnsupported { url: String },
    #[error("{method} {url} returned {status}: {body}")]
    Status {
        method: &'static str,
        url: String,
        status: u16,
        body: String,
    },
    #[error("{method} {url} failed: {source}")]
    Transport {
        method: &'static str,
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("invalid hub url '{url}': {reason}")]
    BadUrl { url: String, reason: String },
    #[error("websocket {url} failed: {source}")]
    Ws {
        url: String,
        #[source]
        source: Box<tokio_tungstenite::tungstenite::Error>,
    },
    #[error("unexpected response from {url}: {reason}")]
    Protocol { url: String, reason: String },
}

/// Response headers worth capturing on failures: they distinguish the hub
/// itself (server/x-jupyterhub-version) from a proxy in front of it, and
/// carry auth hints (www-authenticate) and timing (date).
const DIAGNOSTIC_HEADERS: [&str; 5] = [
    "server",
    "x-jupyterhub-version",
    "www-authenticate",
    "content-type",
    "date",
];

pub async fn check(
    method: &'static str,
    url: &str,
    resp: reqwest::Response,
) -> Result<reqwest::Response, ApiError> {
    let status = resp.status();
    if status.is_success() {
        super::debuglog::log(&[
            ("event", "response".to_string()),
            ("method", method.to_string()),
            ("url", url.to_string()),
            ("status", status.as_u16().to_string()),
        ]);
        return Ok(resp);
    }
    let mut fields = vec![
        ("event", "response".to_string()),
        ("method", method.to_string()),
        ("url", url.to_string()),
        ("status", status.as_u16().to_string()),
    ];
    for name in DIAGNOSTIC_HEADERS {
        if let Some(value) = resp.headers().get(name) {
            fields.push((name, String::from_utf8_lossy(value.as_bytes()).to_string()));
        }
    }
    let body = resp.text().await.unwrap_or_default();
    fields.push((
        "body",
        super::debuglog::snippet(&body, super::debuglog::BODY_SNIPPET_CHARS),
    ));
    super::debuglog::log(&fields);
    match status.as_u16() {
        401 => Err(ApiError::Unauthorized {
            method,
            url: url.to_string(),
        }),
        403 => Err(ApiError::Forbidden {
            method,
            url: url.to_string(),
            body,
        }),
        code => Err(ApiError::Status {
            method,
            url: url.to_string(),
            status: code,
            body,
        }),
    }
}
