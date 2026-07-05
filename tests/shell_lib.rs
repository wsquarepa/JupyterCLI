mod common;

use common::mock_terminado::MockTerminado;
use jhc::api::ws::{TermFrame, TermSocket};
use jhc::shellops;

#[tokio::test]
async fn connect_replay_stdin_roundtrip() {
    let mock = MockTerminado::spawn("old output\r\n", |input| vec![format!("echo:{input}")]).await;
    let mut sock = TermSocket::connect(&mock.url(), "tok").await.unwrap();
    assert!(matches!(
        sock.next_frame().await.unwrap(),
        Some(TermFrame::Setup)
    ));
    match sock.next_frame().await.unwrap() {
        Some(TermFrame::Stdout(s)) => assert_eq!(s, "old output\r\n"),
        other => panic!("expected replay, got {other:?}"),
    }
    sock.send_stdin("ls\n").await.unwrap();
    match sock.next_frame().await.unwrap() {
        Some(TermFrame::Stdout(s)) => assert_eq!(s, "echo:ls\n"),
        other => panic!("expected echo, got {other:?}"),
    }
    sock.finish().await.unwrap();
}

#[tokio::test]
async fn finish_guarantees_delivery_of_last_stdin() {
    let mock = MockTerminado::spawn("", |_| Vec::new()).await;
    let mut sock = TermSocket::connect(&mock.url(), "tok").await.unwrap();
    sock.send_stdin("./kobold --model m.gguf\n").await.unwrap();
    sock.finish().await.unwrap();
    let received = mock.received();
    assert_eq!(received, vec!["./kobold --model m.gguf\n".to_string()]);
}

#[tokio::test]
async fn peek_prints_stripped_replay_then_stops_on_idle() {
    let mock = MockTerminado::spawn("\x1b[?2004hprompt$ tail -f log\r\nline1\r\n", |_| {
        Vec::new()
    })
    .await;
    let sock = TermSocket::connect(&mock.url(), "tok").await.unwrap();
    let mut out: Vec<u8> = Vec::new();
    shellops::peek(sock, false, false, &mut out).await.unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "prompt$ tail -f log\nline1\n"
    );
}

#[tokio::test]
async fn peek_raw_preserves_escapes() {
    let mock = MockTerminado::spawn("\x1b[31mred\x1b[0m", |_| Vec::new()).await;
    let sock = TermSocket::connect(&mock.url(), "tok").await.unwrap();
    let mut out: Vec<u8> = Vec::new();
    shellops::peek(sock, true, false, &mut out).await.unwrap();
    assert_eq!(out, b"\x1b[31mred\x1b[0m");
}

#[tokio::test]
async fn send_appends_newline_and_delivers() {
    let mock = MockTerminado::spawn("", |_| Vec::new()).await;
    let sock = TermSocket::connect(&mock.url(), "tok").await.unwrap();
    shellops::send(sock, "nvidia-smi").await.unwrap();
    assert_eq!(mock.received(), vec!["nvidia-smi\n".to_string()]);
}

#[tokio::test]
async fn exec_runs_command_and_propagates_exit_code() {
    let mock = MockTerminado::spawn("", |input| {
        let nonce_start = input.find("printf '\\036").map(|p| p + 12).unwrap();
        let nonce = &input[nonce_start..nonce_start + 16];
        vec![
            format!("{input}\r\n"),
            format!("\x1e{nonce}:S\x1e"),
            "GPU 0: H200\r\n".to_string(),
            format!("\x1e{nonce}:3\x1e"),
        ]
    })
    .await;
    let sock = TermSocket::connect(&mock.url(), "tok").await.unwrap();
    let mut out: Vec<u8> = Vec::new();
    let outcome = shellops::exec(sock, "nvidia-smi", None, &mut out)
        .await
        .unwrap();
    assert_eq!(outcome.exit_code, 3);
    assert_eq!(String::from_utf8(out).unwrap(), "GPU 0: H200\n");
}
