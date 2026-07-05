use std::collections::BTreeMap;

pub type JsonMap = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct User {
    pub name: String,
    #[serde(default)]
    pub servers: BTreeMap<String, Server>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Server {
    pub name: String,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub pending: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub user_options: JsonMap,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Terminal {
    pub name: String,
    #[serde(default)]
    pub last_activity: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenInfo {
    pub id: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub last_activity: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewToken {
    pub token: String,
    pub id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProgressEvent {
    #[serde(default)]
    pub progress: Option<u64>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContentEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContentModel {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_model_parses_icrn_shape() {
        let raw = r#"{
            "name": "ww41",
            "servers": {
                "backup": {
                    "name": "backup",
                    "ready": true,
                    "pending": null,
                    "url": "/user/ww41/backup/",
                    "user_options": {"profile": "environments", "image": "vscode", "resource": "2_a100"}
                }
            }
        }"#;
        let user: User = serde_json::from_str(raw).unwrap();
        let server = &user.servers["backup"];
        assert!(server.ready);
        assert_eq!(server.url.as_deref(), Some("/user/ww41/backup/"));
        assert_eq!(server.user_options["resource"], serde_json::json!("2_a100"));
    }

    #[test]
    fn progress_event_tolerates_partial_fields() {
        let ev: ProgressEvent =
            serde_json::from_str(r#"{"progress": 50, "message": "spawning"}"#).unwrap();
        assert_eq!(ev.progress, Some(50));
        assert!(!ev.ready && !ev.failed);
        let done: ProgressEvent =
            serde_json::from_str(r#"{"progress": 100, "ready": true, "url": "/user/ww41/"}"#)
                .unwrap();
        assert!(done.ready);
    }
}
