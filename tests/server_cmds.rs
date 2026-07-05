mod common;
#[path = "common/write_config.rs"]
mod write_config;

use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use write_config::write_config;

fn user_body(ready: bool) -> serde_json::Value {
    serde_json::json!({
        "name": "ww41",
        "servers": {"": {"name": "", "ready": ready, "url": "/user/ww41/",
                          "user_options": {"profile": "environments", "resource": "2_a100"}}}
    })
}

#[tokio::test]
async fn start_with_preset_posts_options_and_waits() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_body(false)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/hub/api/users/ww41/server"))
        .and(body_json(
            serde_json::json!({"profile": "environments", "resource": "2_a100"}),
        ))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/hub/api/users/ww41/server/progress"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string("data: {\"progress\": 100, \"ready\": true}\n\n"),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["start", "--preset", "gpu"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn start_rejects_preset_plus_options() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "https://example.invalid");
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .args(["start", "--preset", "gpu", "-o", "resource=3_h200"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("either --preset or -o")
    );
}

#[tokio::test]
async fn start_with_unknown_preset_lists_available() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "https://example.invalid");
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .args(["start", "--preset", "nope"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("nope") && stderr.contains("gpu"),
        "stderr: {stderr}"
    );
}

#[tokio::test]
async fn stop_deletes_named_server() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/hub/api/users/ww41/servers/backup"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_body(true)))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["stop", "backup"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn preset_import_saves_running_options() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_body(true)))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["preset", "import", "--as", "captured"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(
        saved.contains("[hubs.test.presets.captured]"),
        "config was: {saved}"
    );
    assert!(saved.contains("resource = \"2_a100\""));
}
