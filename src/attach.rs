use std::time::{Duration, Instant};

use crate::api::error::ApiError;
use crate::api::ws::{TermFrame, TermSocket};

pub const DETACH_BYTE: u8 = 0x1c;
pub const DETACH_WINDOW: Duration = Duration::from_millis(400);
const PING_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum DetachAction {
    Forward(Vec<u8>),
    Detach,
    Hold,
}

pub struct DetachDetector {
    held_since: Option<Instant>,
}

impl DetachDetector {
    pub fn new() -> Self {
        Self { held_since: None }
    }

    pub fn on_input(&mut self, bytes: &[u8], now: Instant) -> DetachAction {
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + 1);
        for &b in bytes {
            if self.held_since.is_some() {
                if b == DETACH_BYTE {
                    self.held_since = None;
                    return DetachAction::Detach;
                }
                out.push(DETACH_BYTE);
                out.push(b);
                self.held_since = None;
            } else if b == DETACH_BYTE {
                self.held_since = Some(now);
            } else {
                out.push(b);
            }
        }
        if out.is_empty() && self.held_since.is_some() {
            DetachAction::Hold
        } else {
            DetachAction::Forward(out)
        }
    }

    pub fn flush_if_expired(&mut self, now: Instant) -> Option<Vec<u8>> {
        match self.held_since {
            Some(since) if now.duration_since(since) > DETACH_WINDOW => {
                self.held_since = None;
                Some(vec![DETACH_BYTE])
            }
            _ => None,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.held_since.map(|since| since + DETACH_WINDOW)
    }
}

impl Default for DetachDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AttachOutcome {
    Detached,
    RemoteClosed,
}

struct RawGuard;

impl RawGuard {
    fn enable() -> Result<Self, ApiError> {
        crossterm::terminal::enable_raw_mode().map_err(|e| ApiError::Protocol {
            url: "local tty".to_string(),
            reason: format!("cannot enable raw mode: {e}"),
        })?;
        Ok(Self)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

pub async fn attach(mut sock: TermSocket) -> Result<AttachOutcome, ApiError> {
    use std::io::Write as _;
    use tokio::io::AsyncReadExt as _;

    let _raw = RawGuard::enable()?;
    let (cols, rows) = crossterm::terminal::size().map_err(|e| ApiError::Protocol {
        url: "local tty".to_string(),
        reason: format!("cannot read terminal size: {e}"),
    })?;
    sock.send_size(rows, cols).await?;

    let mut winch = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
        .map_err(|e| ApiError::Protocol {
        url: "local tty".to_string(),
        reason: format!("cannot install SIGWINCH handler: {e}"),
    })?;
    let mut stdin = tokio::io::stdin();
    let mut stdout = std::io::stdout();
    let mut detector = DetachDetector::new();
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;
    let mut buf = [0u8; 8192];

    loop {
        let hold_deadline = detector.deadline();
        tokio::select! {
            frame = sock.next_frame() => {
                match frame? {
                    None | Some(TermFrame::Disconnect) => return Ok(AttachOutcome::RemoteClosed),
                    Some(TermFrame::Stdout(text)) => {
                        stdout.write_all(text.as_bytes()).map_err(|e| ApiError::Protocol {
                            url: "local tty".to_string(),
                            reason: format!("cannot write to stdout: {e}"),
                        })?;
                        stdout.flush().ok();
                    }
                    Some(TermFrame::Setup) | Some(TermFrame::Unknown(_)) => {}
                }
            }
            read = stdin.read(&mut buf) => {
                let n = read.map_err(|e| ApiError::Protocol {
                    url: "local tty".to_string(),
                    reason: format!("cannot read stdin: {e}"),
                })?;
                if n == 0 {
                    return Ok(AttachOutcome::Detached);
                }
                match detector.on_input(&buf[..n], Instant::now()) {
                    DetachAction::Detach => {
                        sock.finish().await?;
                        return Ok(AttachOutcome::Detached);
                    }
                    DetachAction::Forward(bytes) if !bytes.is_empty() => {
                        sock.send_stdin(&String::from_utf8_lossy(&bytes)).await?;
                    }
                    DetachAction::Forward(_) | DetachAction::Hold => {}
                }
            }
            _ = async {
                match hold_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(bytes) = detector.flush_if_expired(Instant::now()) {
                    sock.send_stdin(&String::from_utf8_lossy(&bytes)).await?;
                }
            }
            _ = winch.recv() => {
                if let Ok((c, r)) = crossterm::terminal::size() {
                    sock.send_size(r, c).await?;
                }
            }
            _ = ping.tick() => {
                sock.ping().await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn plain_bytes_forward_untouched() {
        let mut d = DetachDetector::new();
        match d.on_input(b"hello", t0()) {
            DetachAction::Forward(bytes) => assert_eq!(bytes, b"hello"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lone_detach_byte_is_held_then_flushed_after_window() {
        let mut d = DetachDetector::new();
        let start = t0();
        assert!(matches!(d.on_input(&[0x1c], start), DetachAction::Hold));
        assert!(
            d.flush_if_expired(start + Duration::from_millis(100))
                .is_none()
        );
        let flushed = d
            .flush_if_expired(start + DETACH_WINDOW + Duration::from_millis(1))
            .unwrap();
        assert_eq!(flushed, vec![0x1c]);
    }

    #[test]
    fn double_detach_byte_within_window_detaches() {
        let mut d = DetachDetector::new();
        let start = t0();
        assert!(matches!(d.on_input(&[0x1c], start), DetachAction::Hold));
        assert!(matches!(
            d.on_input(&[0x1c], start + Duration::from_millis(200)),
            DetachAction::Detach
        ));
    }

    #[test]
    fn other_byte_while_held_flushes_both_in_order() {
        let mut d = DetachDetector::new();
        let start = t0();
        assert!(matches!(d.on_input(&[0x1c], start), DetachAction::Hold));
        match d.on_input(b"x", start + Duration::from_millis(50)) {
            DetachAction::Forward(bytes) => assert_eq!(bytes, vec![0x1c, b'x']),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn detach_byte_inside_larger_buffer_splits_correctly() {
        let mut d = DetachDetector::new();
        match d.on_input(&[b'a', 0x1c], t0()) {
            DetachAction::Forward(bytes) => assert_eq!(bytes, b"a"),
            other => panic!("{other:?}"),
        }
        assert!(d.deadline().is_some());
    }

    #[test]
    fn double_detach_within_one_buffer_detaches() {
        let mut d = DetachDetector::new();
        assert!(matches!(
            d.on_input(&[0x1c, 0x1c], t0()),
            DetachAction::Detach
        ));
    }
}
