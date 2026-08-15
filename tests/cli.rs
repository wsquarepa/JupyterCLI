mod common;

#[path = "common/write_config.rs"]
mod write_config;

use write_config::write_config;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn no_config_prints_guidance_and_fails() {
    let dir = tempfile::tempdir().unwrap();
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .arg("status")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("jhc init"), "stderr was: {stderr}");
    assert!(stderr.contains("hub/token"));
}

#[tokio::test]
async fn init_noninteractive_writes_config_and_status_reads_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "ww41",
            "servers": {"": {"name": "", "ready": true, "url": "/user/ww41/", "user_options": {"resource": "2_a100"}}}
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let init = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args([
            "init",
            "--url",
            &server.uri(),
            "--token",
            "tok",
            "--name",
            "test",
        ])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dir.path().join("config.toml"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    let status = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .arg("status")
        .output()
        .unwrap();
    assert!(status.status.success());
    let stdout = String::from_utf8(status.stdout).unwrap();
    assert!(stdout.contains("ww41"));
    assert!(stdout.contains("default"));
    assert!(stdout.contains("resource=\"2_a100\""));
}

#[test]
fn exec_usage_error_exits_125_not_2() {
    let dir = tempfile::tempdir().unwrap();
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["exec", "--definitely-bogus-flag"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("--definitely-bogus-flag") || stderr.contains("Usage"),
        "expected a usage error, got: {stderr}"
    );
}

#[test]
fn exec_help_still_exits_zero() {
    let output = common::client_bin()
        .args(["exec", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Usage"),
        "expected help text, got: {stdout}"
    );
}

#[tokio::test]
async fn init_refresh_preserves_existing_presets() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "ww41", "servers": {}
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());

    let init = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args([
            "init",
            "--url",
            &server.uri(),
            "--token",
            "new",
            "--name",
            "test",
        ])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(
        saved.contains("[hubs.test.presets.gpu]"),
        "preset table was wiped:\n{saved}"
    );
    assert!(saved.contains("resource = \"2_a100\""), "config:\n{saved}");
    assert!(
        saved.contains("token = \"new\""),
        "token not updated:\n{saved}"
    );
}

#[test]
fn help_copy_says_jupytercli_and_never_uses_em_dashes() {
    fn audit(args: &[&str]) {
        let output = common::client_bin()
            .args(args)
            .arg("--help")
            .output()
            .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            !text.contains('\u{2014}'),
            "em-dash in help for {args:?}:\n{text}"
        );
    }
    let root = common::client_bin().arg("--help").output().unwrap();
    let root_text = String::from_utf8(root.stdout).unwrap();
    assert!(root_text.contains("JupyterCLI"));
    assert!(!root_text.contains('\u{2014}'));
    for group in [
        vec![],
        vec!["init"],
        vec!["status"],
        vec!["start"],
        vec!["stop"],
        vec!["preset"],
        vec!["preset", "import"],
        vec!["shell"],
        vec!["shell", "send"],
        vec!["shell", "peek"],
        vec!["shell", "attach"],
        vec!["exec"],
        vec!["ls"],
        vec!["cp"],
        vec!["rm"],
        vec!["token"],
        vec!["token", "create"],
    ] {
        audit(&group);
    }

    let peek = common::client_bin()
        .args(["shell", "peek", "--help"])
        .output()
        .unwrap();
    let peek_text = String::from_utf8(peek.stdout).unwrap();
    assert!(
        peek_text.contains("tee") && peek_text.contains(":job.log"),
        "peek help must document the tee-to-file long-job pattern:\n{peek_text}"
    );
}

#[tokio::test]
async fn status_json_emits_parseable_hub_user_and_servers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "ww41",
            "servers": {"": {"name": "", "ready": true, "url": "/user/ww41/", "user_options": {"resource": "2_a100"}}}
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["hub"]["name"], "test");
    assert_eq!(value["user"], "ww41");
    assert_eq!(value["servers"][""]["ready"], true);
    assert_eq!(value["servers"][""]["user_options"]["resource"], "2_a100");
}

#[tokio::test]
async fn shell_list_json_emits_terminals_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "ww41",
            "servers": {"": {"name": "", "ready": true, "url": "/user/ww41/", "user_options": {}}}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/ww41/api/terminals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"name": "1", "last_activity": "2026-08-14T00:00:00Z"},
            {"name": "2"}
        ])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &server.uri());
    let output = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["shell", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["terminals"][0]["name"], "1");
    assert_eq!(
        value["terminals"][0]["last_activity"],
        "2026-08-14T00:00:00Z"
    );
    assert_eq!(value["terminals"][1]["name"], "2");
}

#[tokio::test]
async fn verbose_prints_request_summaries_without_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hub/api/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "ww41", "servers": {}
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let init = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args([
            "init",
            "--url",
            &server.uri(),
            "--token",
            "supersecrettok",
            "--name",
            "test",
        ])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let status = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["--verbose", "status"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let stderr = String::from_utf8(status.stderr).unwrap();
    assert!(stderr.contains("jhc::api"), "stderr was: {stderr}");
    assert!(stderr.contains("request"), "stderr was: {stderr}");
    assert!(stderr.contains("GET"), "stderr was: {stderr}");
    assert!(
        !stderr.contains("supersecrettok"),
        "token must never appear in verbose output"
    );

    let quiet = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .arg("status")
        .output()
        .unwrap();
    assert!(
        !String::from_utf8(quiet.stderr)
            .unwrap()
            .contains("jhc::api")
    );
}
