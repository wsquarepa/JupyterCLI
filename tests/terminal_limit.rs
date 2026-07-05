mod common;
#[path = "common/write_config.rs"]
mod write_config;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use write_config::{write_config, write_config_with};

fn user_body() -> serde_json::Value {
    serde_json::json!({
        "name": "ww41",
        "servers": {"": {"name": "", "ready": true, "url": "/user/ww41/", "user_options": {}}}
    })
}

fn terminals_body(count: usize) -> serde_json::Value {
    let items: Vec<serde_json::Value> = (1..=count)
        .map(|i| serde_json::json!({"name": i.to_string()}))
        .collect();
    serde_json::Value::Array(items)
}

#[tokio::test]
async fn shell_new_refuses_at_the_default_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_body()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/ww41/api/terminals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(terminals_body(999)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/user/ww41/api/terminals"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .args(["shell", "new"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("terminal limit reached (999 of 999)"),
        "stderr was: {stderr}"
    );
    assert!(stderr.contains("terminal_limit"), "stderr was: {stderr}");
}

#[tokio::test]
async fn shell_new_allows_more_when_config_raises_the_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_body()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/ww41/api/terminals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(terminals_body(999)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/user/ww41/api/terminals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"name": "1000"})))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config_with(dir.path(), &server.uri(), "terminal_limit = 1500\n");
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .args(["shell", "new"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
