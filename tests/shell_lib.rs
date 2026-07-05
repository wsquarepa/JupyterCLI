mod common;

use common::mock_terminado::MockTerminado;
use jhc::api::ws::{TermFrame, TermSocket};

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
