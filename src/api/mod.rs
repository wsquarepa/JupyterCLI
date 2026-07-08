pub mod error;
pub mod server;
pub mod sse;
pub mod types;
pub mod ws;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use error::{ApiError, check};
use types::{JsonMap, NewToken, ProgressEvent, TokenInfo, User};

/// One `client init` event naming which hub and which token a client was built
/// with. The env/config distinction plus the fingerprint is what lets an
/// intermittent auth failure be correlated with the credential in use without
/// recording token material.
pub fn log_client_init(hub_name: &str, base_url: &str, token: &str) {
    let source = if std::env::var_os("JUPYTERHUB_API_TOKEN").is_some() {
        "env:JUPYTERHUB_API_TOKEN"
    } else {
        "config"
    };
    tracing::info!(
        target: "jhc::api",
        hub = hub_name,
        url = base_url,
        token_source = source,
        token_fp = %crate::logging::fingerprint(token),
        token_len = token.chars().count(),
        "client init"
    );
}

/// Percent-encode a server name for use as a single URL path segment. Encodes
/// everything outside the unreserved set so a name like `a b` or `a?b` reaches
/// the hub verbatim after it decodes the segment. This is transport encoding,
/// not name normalization.
const NAME_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Clone)]
pub struct HubClient {
    http: reqwest::Client,
    base: reqwest::Url,
    token: String,
}

impl HubClient {
    pub fn new(url: &str, token: &str) -> Result<Self, ApiError> {
        let trimmed = format!("{}/", url.trim_end_matches('/'));
        let base = reqwest::Url::parse(&trimmed).map_err(|e| ApiError::BadUrl {
            url: url.to_string(),
            reason: e.to_string(),
        })?;
        Ok(Self {
            http: reqwest::Client::new(),
            base,
            token: token.to_string(),
        })
    }

    pub fn base(&self) -> &reqwest::Url {
        &self.base
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    fn url(&self, path: &str) -> Result<reqwest::Url, ApiError> {
        self.base.join(path).map_err(|e| ApiError::BadUrl {
            url: format!("{}{path}", self.base),
            reason: e.to_string(),
        })
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.header("Authorization", format!("token {}", self.token))
    }

    async fn get(&self, path: &str) -> Result<reqwest::Response, ApiError> {
        let url = self.url(path)?;
        let mut last: Option<ApiError> = None;
        for attempt in 1..=3u32 {
            let result = self.auth(self.http.get(url.clone())).send().await;
            match result {
                Ok(resp) if resp.status().is_server_error() && attempt < 3 => {
                    let status = resp.status();
                    tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %status, "request");
                    tracing::warn!(target: "jhc::api", method = "GET", url = %url, attempt, status = %status, "retrying");
                    last = Some(ApiError::Status {
                        method: "GET",
                        url: url.to_string(),
                        status: status.as_u16(),
                        body: resp.text().await.unwrap_or_default(),
                    });
                }
                Ok(resp) => {
                    tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %resp.status(), "request");
                    return check("GET", url.as_str(), resp).await;
                }
                Err(e) if attempt < 3 => {
                    tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %format!("transport error: {e}"), "request");
                    tracing::warn!(target: "jhc::api", method = "GET", url = %url, attempt, error = %e, "retrying");
                    last = Some(ApiError::Transport {
                        method: "GET",
                        url: url.to_string(),
                        source: e,
                    });
                }
                Err(e) => {
                    tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %format!("transport error: {e}"), "request");
                    return Err(ApiError::Transport {
                        method: "GET",
                        url: url.to_string(),
                        source: e,
                    });
                }
            }
        }
        Err(last.expect("retry loop records an error before exhausting attempts"))
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn whoami(&self) -> Result<User, ApiError> {
        let url = self.url("hub/api/user")?;
        let resp = self.get("hub/api/user").await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = name))]
    pub async fn user(&self, name: &str) -> Result<User, ApiError> {
        let path = format!("hub/api/users/{name}");
        let url = self.url(&path)?;
        let resp = self.get(&path).await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = name))]
    pub async fn user_including_stopped(&self, name: &str) -> Result<User, ApiError> {
        let path = format!("hub/api/users/{name}?include_stopped_servers=true");
        let url = self.url(&path)?;
        let resp = self.get(&path).await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })
    }

    fn server_path(user: &str, server: Option<&str>) -> String {
        match server {
            Some(name) => {
                let encoded = utf8_percent_encode(name, NAME_SEGMENT);
                format!("hub/api/users/{user}/servers/{encoded}")
            }
            None => format!("hub/api/users/{user}/server"),
        }
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = user, server = server))]
    pub async fn spawn(
        &self,
        user: &str,
        server: Option<&str>,
        options: &JsonMap,
    ) -> Result<(), ApiError> {
        let url = self.url(&Self::server_path(user, server))?;
        let result = self
            .auth(self.http.post(url.clone()))
            .json(options)
            .send()
            .await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "POST", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "POST", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "POST",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        check("POST", url.as_str(), resp).await.map(|_| ())
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = user, server = server))]
    pub async fn stop(&self, user: &str, server: Option<&str>) -> Result<(), ApiError> {
        let url = self.url(&Self::server_path(user, server))?;
        let result = self.auth(self.http.delete(url.clone())).send().await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "DELETE", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "DELETE", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "DELETE",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        check("DELETE", url.as_str(), resp).await.map(|_| ())
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = user, server = server))]
    pub async fn wait_ready(
        &self,
        user: &str,
        server: Option<&str>,
        mut on_event: impl FnMut(&ProgressEvent),
    ) -> Result<(), ApiError> {
        use futures_util::StreamExt as _;
        let path = format!("{}/progress", Self::server_path(user, server));
        let url = self.url(&path)?;
        let resp = self.get(&path).await?;
        let mut stream = resp.bytes_stream();
        let mut parser = sse::SseParser::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| ApiError::Transport {
                method: "GET",
                url: url.to_string(),
                source: e,
            })?;
            for payload in parser.push(&String::from_utf8_lossy(&bytes)) {
                let event: ProgressEvent =
                    serde_json::from_str(&payload).map_err(|e| ApiError::Protocol {
                        url: url.to_string(),
                        reason: format!("bad progress event {payload:?}: {e}"),
                    })?;
                on_event(&event);
                tracing::debug!(target: "jhc::api", message = ?event.message, progress = ?event.progress, ready = event.ready, failed = event.failed, "spawn progress");
                if event.failed {
                    return Err(ApiError::Protocol {
                        url: url.to_string(),
                        reason: format!(
                            "spawn failed: {}",
                            event.message.unwrap_or_else(|| "no message".to_string())
                        ),
                    });
                }
                if event.ready {
                    return Ok(());
                }
            }
        }
        Err(ApiError::Protocol {
            url: url.to_string(),
            reason: "progress stream ended before the server became ready".to_string(),
        })
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = user))]
    pub async fn tokens(&self, user: &str) -> Result<Vec<TokenInfo>, ApiError> {
        let path = format!("hub/api/users/{user}/tokens");
        let url = self.url(&path)?;
        let resp = self.get(&path).await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = user))]
    pub async fn create_token(&self, user: &str, note: &str) -> Result<NewToken, ApiError> {
        let url = self.url(&format!("hub/api/users/{user}/tokens"))?;
        let result = self
            .auth(self.http.post(url.clone()))
            .json(&serde_json::json!({ "note": note }))
            .send()
            .await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "POST", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "POST", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "POST",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        let resp = check("POST", url.as_str(), resp).await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "POST",
            url: url.to_string(),
            source: e,
        })
    }

    #[tracing::instrument(level = "debug", skip_all, fields(user = user, token_id = id))]
    pub async fn revoke_token(&self, user: &str, id: &str) -> Result<(), ApiError> {
        let url = self.url(&format!("hub/api/users/{user}/tokens/{id}"))?;
        let result = self.auth(self.http.delete(url.clone())).send().await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "DELETE", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "DELETE", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "DELETE",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        check("DELETE", url.as_str(), resp).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client(server: &MockServer) -> HubClient {
        HubClient::new(&server.uri(), "tok").unwrap()
    }

    #[tokio::test]
    async fn whoami_hits_hub_api_user_with_token_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .and(header("Authorization", "token tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "ww41", "servers": {}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let user = client(&server).await.whoami().await.unwrap();
        assert_eq!(user.name, "ww41");
    }

    #[tokio::test]
    async fn spawn_posts_bare_options_object() {
        let server = MockServer::start().await;
        let opts: types::JsonMap =
            serde_json::from_str(r#"{"profile": "environments", "resource": "2_a100"}"#).unwrap();
        Mock::given(method("POST"))
            .and(path("/hub/api/users/ww41/servers/backup"))
            .and(body_json(
                serde_json::json!({"profile": "environments", "resource": "2_a100"}),
            ))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .await
            .spawn("ww41", Some("backup"), &opts)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn spawn_percent_encodes_the_server_name_segment() {
        // A `?` would otherwise be parsed by Url::join as the query separator,
        // truncating the path; encoding keeps it inside the name segment.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hub/api/users/ww41/servers/a%3Fb"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .await
            .spawn("ww41", Some("a?b"), &JsonMap::new())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unauthorized_maps_to_actionable_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = client(&server).await.whoami().await.unwrap_err();
        assert!(err.to_string().contains("jhc init"));
    }

    #[tokio::test]
    async fn forbidden_with_scope_body_keeps_scope_copy() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"status": 403, "message": "Action is not authorized with current scopes; requires any of [admin:users]"}"#,
            ))
            .mount(&server)
            .await;
        let err = client(&server).await.whoami().await.unwrap_err();
        assert!(err.to_string().contains("scope"));
    }

    #[tokio::test]
    async fn forbidden_with_bare_body_maps_to_auth_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_string(r#"{"status": 403, "message": "Forbidden"}"#),
            )
            .mount(&server)
            .await;
        let err = client(&server).await.whoami().await.unwrap_err();
        assert!(err.to_string().contains("browser login"));
    }

    #[tokio::test]
    async fn get_retries_on_5xx_then_raises() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(502))
            .expect(3)
            .mount(&server)
            .await;
        let err = client(&server).await.whoami().await.unwrap_err();
        assert!(err.to_string().contains("502"));
    }

    #[tokio::test]
    async fn user_including_stopped_sends_the_flag_and_parses_a_stopped_server() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/users/ww41"))
            .and(query_param("include_stopped_servers", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "ww41",
                "servers": {
                    "backup": {
                        "name": "backup", "ready": false, "pending": null,
                        "stopped": true, "url": null, "user_options": {}
                    }
                }
            })))
            .mount(&server)
            .await;
        let user = client(&server)
            .await
            .user_including_stopped("ww41")
            .await
            .unwrap();
        let backup = &user.servers["backup"];
        assert!(!backup.ready);
        assert_eq!(backup.name, "backup");
    }

    #[tokio::test]
    async fn wait_ready_streams_sse_until_ready() {
        let server = MockServer::start().await;
        let body = "data: {\"progress\": 50, \"message\": \"pod pending\"}\n\ndata: {\"progress\": 100, \"ready\": true}\n\n";
        Mock::given(method("GET"))
            .and(path("/hub/api/users/ww41/server/progress"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let mut messages = Vec::new();
        client(&server)
            .await
            .wait_ready("ww41", None, |ev| {
                messages.push(ev.message.clone().unwrap_or_default());
            })
            .await
            .unwrap();
        assert_eq!(messages[0], "pod pending");
    }

    #[tokio::test]
    async fn wait_ready_surfaces_failure_message() {
        let server = MockServer::start().await;
        let body = "data: {\"failed\": true, \"message\": \"KeyError: 'bogus'\"}\n\n";
        Mock::given(method("GET"))
            .and(path("/hub/api/users/ww41/server/progress"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .await
            .wait_ready("ww41", None, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("KeyError"));
    }
}
