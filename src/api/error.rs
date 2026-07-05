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

pub async fn check(
    method: &'static str,
    url: &str,
    resp: reqwest::Response,
) -> Result<reqwest::Response, ApiError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
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
