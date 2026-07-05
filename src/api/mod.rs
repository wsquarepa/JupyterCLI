pub mod error;
pub mod server;
pub mod sse;
pub mod types;

use error::{ApiError, check};
use types::{JsonMap, NewToken, ProgressEvent, TokenInfo, User};

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
                    eprintln!(
                        "warning: GET {url} attempt {attempt} returned {}; retrying",
                        resp.status()
                    );
                    last = Some(ApiError::Status {
                        method: "GET",
                        url: url.to_string(),
                        status: resp.status().as_u16(),
                        body: resp.text().await.unwrap_or_default(),
                    });
                }
                Ok(resp) => return check("GET", url.as_str(), resp).await,
                Err(e) if attempt < 3 => {
                    eprintln!("warning: GET {url} attempt {attempt} failed: {e}; retrying");
                    last = Some(ApiError::Transport {
                        method: "GET",
                        url: url.to_string(),
                        source: e,
                    });
                }
                Err(e) => {
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

    pub async fn whoami(&self) -> Result<User, ApiError> {
        let url = self.url("hub/api/user")?;
        let resp = self.get("hub/api/user").await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })
    }

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

    fn server_path(user: &str, server: Option<&str>) -> String {
        match server {
            Some(name) => format!("hub/api/users/{user}/servers/{name}"),
            None => format!("hub/api/users/{user}/server"),
        }
    }

    pub async fn spawn(
        &self,
        user: &str,
        server: Option<&str>,
        options: &JsonMap,
    ) -> Result<(), ApiError> {
        let url = self.url(&Self::server_path(user, server))?;
        let resp = self
            .auth(self.http.post(url.clone()))
            .json(options)
            .send()
            .await
            .map_err(|e| ApiError::Transport {
                method: "POST",
                url: url.to_string(),
                source: e,
            })?;
        check("POST", url.as_str(), resp).await.map(|_| ())
    }

    pub async fn stop(&self, user: &str, server: Option<&str>) -> Result<(), ApiError> {
        let url = self.url(&Self::server_path(user, server))?;
        let resp = self
            .auth(self.http.delete(url.clone()))
            .send()
            .await
            .map_err(|e| ApiError::Transport {
                method: "DELETE",
                url: url.to_string(),
                source: e,
            })?;
        check("DELETE", url.as_str(), resp).await.map(|_| ())
    }

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

    pub async fn create_token(&self, user: &str, note: &str) -> Result<NewToken, ApiError> {
        let url = self.url(&format!("hub/api/users/{user}/tokens"))?;
        let resp = self
            .auth(self.http.post(url.clone()))
            .json(&serde_json::json!({ "note": note }))
            .send()
            .await
            .map_err(|e| ApiError::Transport {
                method: "POST",
                url: url.to_string(),
                source: e,
            })?;
        let resp = check("POST", url.as_str(), resp).await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "POST",
            url: url.to_string(),
            source: e,
        })
    }

    pub async fn revoke_token(&self, user: &str, id: &str) -> Result<(), ApiError> {
        let url = self.url(&format!("hub/api/users/{user}/tokens/{id}"))?;
        let resp = self
            .auth(self.http.delete(url.clone()))
            .send()
            .await
            .map_err(|e| ApiError::Transport {
                method: "DELETE",
                url: url.to_string(),
                source: e,
            })?;
        check("DELETE", url.as_str(), resp).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
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
