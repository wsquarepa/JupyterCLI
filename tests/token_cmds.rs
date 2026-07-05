mod common;
#[path = "common/write_config.rs"]
mod write_config;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use write_config::write_config;

async fn mock_user(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "ww41", "servers": {}
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn token_list_renders_table() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/hub/api/users/ww41/tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": "a1", "note": "laptop", "created": "2026-07-01T00:00:00Z"},
            {"id": "b2", "note": null, "expires_at": "2026-08-01T00:00:00Z"}
        ])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["token", "list"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("a1") && stdout.contains("laptop") && stdout.contains("b2"));
}

#[tokio::test]
async fn token_create_prints_secret_once() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("POST"))
        .and(path("/hub/api/users/ww41/tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "secret-value", "id": "c3"
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["token", "create", "--note", "ci runner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("secret-value"));
    assert!(stdout.contains("will not be shown again"));
}

#[tokio::test]
async fn token_revoke_deletes() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("DELETE"))
        .and(path("/hub/api/users/ww41/tokens/a1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["token", "revoke", "a1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
