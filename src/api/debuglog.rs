//! Best-effort diagnostic logging for intermittent API failures, gated by the
//! `JHC_DEBUG_LOG` environment variable (a file path). Writes structured
//! `key=value` lines that are safe in both CLI and TUI contexts (a file, not
//! stderr, so the alternate screen is never corrupted). Never records token
//! material; tokens appear only as a fingerprint.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_PATH: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| std::env::var_os("JHC_DEBUG_LOG").map(PathBuf::from));

pub const BODY_SNIPPET_CHARS: usize = 2000;

/// Append one structured line to the debug log, or do nothing when
/// `JHC_DEBUG_LOG` is unset. Write failures are deliberately swallowed: a
/// diagnostics channel must never fail or alter the client's real work.
pub fn log(fields: &[(&str, String)]) {
    let Some(path) = LOG_PATH.as_ref() else {
        return;
    };
    let line = render_line(now_millis(), fields);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// One `ts=... key=value ...` line. Values with spaces or quotes are wrapped
/// in double quotes (inner quotes become apostrophes so the line stays
/// splittable); newlines are escaped so one event is always one line.
fn render_line(ts_millis: u128, fields: &[(&str, String)]) -> String {
    let mut line = format!("ts={ts_millis}");
    for (key, value) in fields {
        let flat = value.replace('\r', "").replace('\n', "\\n");
        if flat.contains(' ') || flat.contains('"') || flat.is_empty() {
            line.push_str(&format!(" {key}=\"{}\"", flat.replace('"', "'")));
        } else {
            line.push_str(&format!(" {key}={flat}"));
        }
    }
    line
}

/// FNV-1a fingerprint so log lines can tell which token was in use without
/// recording token material. Non-cryptographic, but a 64-bit preimage of a
/// high-entropy secret is not recoverable from a local debug artifact.
pub fn fingerprint(secret: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in secret.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Truncate on a char boundary so multibyte responses cannot panic the logger.
pub fn snippet(body: &str, max_chars: usize) -> String {
    body.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_line_quotes_and_escapes() {
        let line = render_line(
            1720464000123,
            &[
                ("method", "GET".to_string()),
                (
                    "body",
                    "{\"status\": 403, \"message\": \"Forbidden\"}".to_string(),
                ),
                ("note", "line one\nline two".to_string()),
                ("empty", String::new()),
            ],
        );
        assert_eq!(
            line,
            "ts=1720464000123 method=GET \
             body=\"{'status': 403, 'message': 'Forbidden'}\" \
             note=\"line one\\nline two\" empty=\"\""
        );
    }

    #[test]
    fn fingerprint_is_stable_and_discriminating() {
        assert_eq!(fingerprint("abc"), fingerprint("abc"));
        assert_ne!(fingerprint("abc"), fingerprint("abd"));
        assert_eq!(fingerprint("abc").len(), 16);
    }

    #[test]
    fn snippet_respects_char_boundaries() {
        assert_eq!(snippet("héllo wörld", 6), "héllo ");
        assert_eq!(snippet("short", 2000), "short");
    }
}
