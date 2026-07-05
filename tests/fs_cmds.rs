mod common;
#[path = "common/write_config.rs"]
mod write_config;

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use write_config::write_config;

async fn mock_user(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "ww41",
            "servers": {"": {"name": "", "ready": true, "url": "/user/ww41/", "user_options": {}}}
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn ls_renders_directory_listing() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/user/ww41/api/contents/work"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "work", "path": "work", "type": "directory",
            "content": [
                {"name": "data.csv", "path": "work/data.csv", "type": "file", "size": 1234,
                 "last_modified": "2026-07-04T00:00:00Z"},
                {"name": "notebooks", "path": "work/notebooks", "type": "directory"}
            ]
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["ls", ":work"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("data.csv") && stdout.contains("notebooks"));
}

#[tokio::test]
async fn cp_download_writes_local_file() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(b"csv,data\n");
    Mock::given(method("GET"))
        .and(path("/user/ww41/api/contents/work/data.csv"))
        .and(query_param("format", "base64"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "data.csv", "path": "work/data.csv", "type": "file",
            "format": "base64", "content": b64
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let dest = dir.path().join("local.csv");
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["cp", ":work/data.csv", dest.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"csv,data\n");
}

#[tokio::test]
async fn cp_rejects_local_to_local_and_remote_to_remote() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "https://example.invalid");
    for pair in [["a.txt", "b.txt"], [":a.txt", ":b.txt"]] {
        let output = common::client_bin()
            .env("JHC_CONFIG_DIR", dir.path())
            .args(["cp", pair[0], pair[1]])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("exactly one side"), "stderr: {stderr}");
    }
}

#[tokio::test]
async fn rm_refuses_directory_without_recursive() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/user/ww41/api/contents/work"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "work", "path": "work", "type": "directory", "content": []
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["rm", ":work"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("-r"));
}
