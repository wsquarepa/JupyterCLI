use super::HubClient;
use super::error::{ApiError, check};
use super::types::{ContentEntry, ContentModel, Terminal};
use base64::Engine as _;

pub struct ServerClient {
    http: reqwest::Client,
    base: reqwest::Url,
    token: String,
}

impl ServerClient {
    pub fn from_hub(hub: &HubClient, server_url_path: &str) -> Result<Self, ApiError> {
        // JupyterHub's `server.url` is host-absolute and already carries the hub's
        // base_url prefix, so it must replace the hub base's path wholesale rather than
        // extend it. A leading-slash join does exactly that; the trailing slash keeps
        // later relative joins (`api/terminals`, ...) inside the server path.
        let abs = format!("/{}/", server_url_path.trim_matches('/'));
        let base = hub.base().join(&abs).map_err(|e| ApiError::BadUrl {
            url: format!("{}{server_url_path}", hub.base()),
            reason: e.to_string(),
        })?;
        Ok(Self {
            http: reqwest::Client::new(),
            base,
            token: hub.token().to_string(),
        })
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

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn terminals(&self) -> Result<Vec<Terminal>, ApiError> {
        let url = self.url("api/terminals")?;
        let result = self.auth(self.http.get(url.clone())).send().await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "GET",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        if resp.status().as_u16() == 404 {
            return Err(ApiError::TerminalsUnsupported {
                url: url.to_string(),
            });
        }
        let resp = check("GET", url.as_str(), resp).await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn create_terminal(&self) -> Result<Terminal, ApiError> {
        let url = self.url("api/terminals")?;
        let result = self.auth(self.http.post(url.clone())).send().await;
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
        if resp.status().as_u16() == 404 {
            return Err(ApiError::TerminalsUnsupported {
                url: url.to_string(),
            });
        }
        let resp = check("POST", url.as_str(), resp).await?;
        resp.json().await.map_err(|e| ApiError::Transport {
            method: "POST",
            url: url.to_string(),
            source: e,
        })
    }

    #[tracing::instrument(level = "debug", skip_all, fields(name = name))]
    pub async fn delete_terminal(&self, name: &str) -> Result<(), ApiError> {
        let url = self.url(&format!("api/terminals/{name}"))?;
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
        if resp.status().as_u16() == 404 {
            return Ok(());
        }
        check("DELETE", url.as_str(), resp).await.map(|_| ())
    }

    #[tracing::instrument(level = "debug", skip_all, fields(path = path))]
    pub async fn list_dir(&self, path: &str) -> Result<Vec<ContentEntry>, ApiError> {
        let url = self.url(&format!("api/contents/{}", path.trim_start_matches('/')))?;
        let result = self
            .auth(self.http.get(url.clone()))
            .query(&[("content", "1")])
            .send()
            .await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "GET",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        let resp = check("GET", url.as_str(), resp).await?;
        let model: serde_json::Value = resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })?;
        match model["type"].as_str() {
            Some("directory") => {
                let entries = model["content"].clone();
                serde_json::from_value(entries).map_err(|e| ApiError::Protocol {
                    url: url.to_string(),
                    reason: format!("bad directory listing: {e}"),
                })
            }
            Some(_) => {
                let entry: ContentEntry =
                    serde_json::from_value(model).map_err(|e| ApiError::Protocol {
                        url: url.to_string(),
                        reason: format!("bad content model: {e}"),
                    })?;
                Ok(vec![entry])
            }
            None => Err(ApiError::Protocol {
                url: url.to_string(),
                reason: "content model missing type".to_string(),
            }),
        }
    }

    #[tracing::instrument(level = "debug", skip_all, fields(path = path))]
    pub async fn download(&self, path: &str) -> Result<Vec<u8>, ApiError> {
        let url = self.url(&format!("api/contents/{}", path.trim_start_matches('/')))?;
        let result = self
            .auth(self.http.get(url.clone()))
            .query(&[("format", "base64"), ("content", "1")])
            .send()
            .await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "GET", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "GET",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        let resp = check("GET", url.as_str(), resp).await?;
        let model: ContentModel = resp.json().await.map_err(|e| ApiError::Transport {
            method: "GET",
            url: url.to_string(),
            source: e,
        })?;
        let content = model
            .content
            .as_ref()
            .and_then(|c| c.as_str())
            .ok_or_else(|| ApiError::Protocol {
                url: url.to_string(),
                reason: "content model has no string content".to_string(),
            })?;
        match model.format.as_deref() {
            Some("base64") => base64::engine::general_purpose::STANDARD
                .decode(content.replace('\n', ""))
                .map_err(|e| ApiError::Protocol {
                    url: url.to_string(),
                    reason: format!("invalid base64 content: {e}"),
                }),
            Some("text") => Ok(content.as_bytes().to_vec()),
            other => Err(ApiError::Protocol {
                url: url.to_string(),
                reason: format!("unexpected content format {other:?}"),
            }),
        }
    }

    #[tracing::instrument(level = "debug", skip_all, fields(path = path))]
    pub async fn upload(&self, path: &str, bytes: &[u8]) -> Result<(), ApiError> {
        let clean = path.trim_start_matches('/');
        let url = self.url(&format!("api/contents/{clean}"))?;
        let body = serde_json::json!({
            "type": "file",
            "format": "base64",
            "content": base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        let result = self
            .auth(self.http.put(url.clone()))
            .json(&body)
            .send()
            .await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "PUT", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "PUT", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "PUT",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        check("PUT", url.as_str(), resp).await.map(|_| ())
    }

    #[tracing::instrument(level = "debug", skip_all, fields(path = path))]
    pub async fn mkdir(&self, path: &str) -> Result<(), ApiError> {
        let clean = path.trim_start_matches('/');
        let url = self.url(&format!("api/contents/{clean}"))?;
        let result = self
            .auth(self.http.put(url.clone()))
            .json(&serde_json::json!({"type": "directory"}))
            .send()
            .await;
        let resp = match result {
            Ok(resp) => {
                tracing::debug!(target: "jhc::api", method = "PUT", url = %url, outcome = %resp.status(), "request");
                resp
            }
            Err(e) => {
                tracing::debug!(target: "jhc::api", method = "PUT", url = %url, outcome = %format!("transport error: {e}"), "request");
                return Err(ApiError::Transport {
                    method: "PUT",
                    url: url.to_string(),
                    source: e,
                });
            }
        };
        check("PUT", url.as_str(), resp).await.map(|_| ())
    }

    #[tracing::instrument(level = "debug", skip_all, fields(path = path))]
    pub async fn delete_path(&self, path: &str) -> Result<(), ApiError> {
        let clean = path.trim_start_matches('/');
        let url = self.url(&format!("api/contents/{clean}"))?;
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

    pub fn ws_terminal_url(&self, name: &str) -> Result<String, ApiError> {
        let mut url = self.url(&format!("terminals/websocket/{name}"))?;
        let scheme = match url.scheme() {
            "https" => "wss",
            "http" => "ws",
            other => {
                return Err(ApiError::BadUrl {
                    url: url.to_string(),
                    reason: format!("unsupported scheme {other}"),
                });
            }
        };
        url.set_scheme(scheme).map_err(|()| ApiError::BadUrl {
            url: url.to_string(),
            reason: "cannot set websocket scheme".to_string(),
        })?;
        Ok(url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::HubClient;
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn sc(server: &MockServer) -> ServerClient {
        let hub = HubClient::new(&server.uri(), "tok").unwrap();
        ServerClient::from_hub(&hub, "/user/ww41/backup/").unwrap()
    }

    #[tokio::test]
    async fn terminals_404_maps_to_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/ww41/backup/api/terminals"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let err = sc(&server).await.terminals().await.unwrap_err();
        assert!(err.to_string().contains("terminals API"));
    }

    #[tokio::test]
    async fn create_and_delete_terminal() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/user/ww41/backup/api/terminals"))
            .and(header("Authorization", "token tok"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"name": "1"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/user/ww41/backup/api/terminals/1"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = sc(&server).await;
        let term = client.create_terminal().await.unwrap();
        assert_eq!(term.name, "1");
        client.delete_terminal("1").await.unwrap();
    }

    #[tokio::test]
    async fn delete_terminal_tolerates_404() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/user/ww41/backup/api/terminals/9"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        sc(&server).await.delete_terminal("9").await.unwrap();
    }

    #[tokio::test]
    async fn download_decodes_base64() {
        let server = MockServer::start().await;
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello\x00world");
        Mock::given(method("GET"))
            .and(path("/user/ww41/backup/api/contents/data/blob.bin"))
            .and(query_param("format", "base64"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "blob.bin", "path": "data/blob.bin", "type": "file",
                "format": "base64", "content": b64
            })))
            .mount(&server)
            .await;
        let bytes = sc(&server).await.download("data/blob.bin").await.unwrap();
        assert_eq!(bytes, b"hello\x00world");
    }

    #[tokio::test]
    async fn upload_puts_base64_model() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/user/ww41/backup/api/contents/out.txt"))
            .and(body_partial_json(
                serde_json::json!({"type": "file", "format": "base64"}),
            ))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        sc(&server).await.upload("out.txt", b"hi").await.unwrap();
    }

    #[test]
    fn ws_url_swaps_scheme() {
        let hub = HubClient::new("https://jupyter.example.edu", "tok").unwrap();
        let client = ServerClient::from_hub(&hub, "/user/ww41/").unwrap();
        assert_eq!(
            client.ws_terminal_url("2").unwrap(),
            "wss://jupyter.example.edu/user/ww41/terminals/websocket/2"
        );
    }

    #[test]
    fn base_url_hub_does_not_double_prefix() {
        let hub = HubClient::new("https://host/jupyter", "tok").unwrap();
        let client = ServerClient::from_hub(&hub, "/jupyter/user/ww41/").unwrap();
        assert_eq!(
            client.ws_terminal_url("1").unwrap(),
            "wss://host/jupyter/user/ww41/terminals/websocket/1"
        );
    }
}
