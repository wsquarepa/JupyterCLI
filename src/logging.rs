//! Logging setup and redaction helpers. CLI always installs a stderr
//! subscriber (default `warn`; `--verbose` raises jhc to debug). The TUI only
//! logs when `--verbose` or `RUST_LOG` is set, writing to an auto-named file
//! because stderr would corrupt the alternate screen. Token material and
//! terminal payload bytes are never recorded; a token appears only as a
//! fingerprint.

use std::path::PathBuf;

use time::OffsetDateTime;
use time::macros::format_description;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::time::UtcTime;

pub const BODY_SNIPPET_CHARS: usize = 2000;

/// Default `RUST_LOG`-style directive used only when `RUST_LOG` is unset.
/// `--verbose` raises jhc targets to debug; third-party stays at warn.
pub fn cli_directive(verbose: bool) -> &'static str {
    if verbose { "warn,jhc=debug" } else { "warn" }
}

/// Whether the TUI should install a file log sink. Opt-in only: `--verbose`
/// and/or a set `RUST_LOG` environment variable (presence, not value).
pub fn tui_logging_active(verbose: bool, rust_log_present: bool) -> bool {
    verbose || rust_log_present
}

/// `jhc-YYYYMMDDTHHMMSSZ-<pid>.log`. UTC keeps names sortable and unambiguous
/// across timezones; the pid disambiguates same-second launches.
pub fn log_file_name(now: OffsetDateTime, pid: u32) -> String {
    let fmt = format_description!("[year][month][day]T[hour][minute][second]Z");
    let stamp = now
        .to_offset(time::UtcOffset::UTC)
        .format(&fmt)
        .unwrap_or_else(|_| "unknown".to_string());
    format!("jhc-{stamp}-{pid}.log")
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

/// Install the global stderr subscriber for CLI runs. `RUST_LOG` wins when set;
/// otherwise `cli_directive(verbose)`.
pub fn init_cli(verbose: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cli_directive(verbose)));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(UtcTime::rfc_3339())
        .with_writer(std::io::stderr)
        .init();
}

/// Install the global file subscriber for the TUI when logging is requested.
/// Returns `Ok(None)` when neither `--verbose` nor `RUST_LOG` is set (no file,
/// no subscriber). Any I/O failure returns Err WITHOUT installing a subscriber:
/// the diagnostics channel must never prevent the TUI from launching, and under
/// the alternate screen there is nowhere to surface the error. With no
/// subscriber installed, tracing events become no-ops.
pub fn init_tui(verbose: bool) -> std::io::Result<Option<PathBuf>> {
    if !tui_logging_active(verbose, std::env::var_os("RUST_LOG").is_some()) {
        return Ok(None);
    }
    let dir = dirs::state_dir()
        .or_else(dirs::cache_dir)
        .ok_or_else(|| std::io::Error::other("no state or cache directory"))?
        .join("jhc")
        .join("logs");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(log_file_name(OffsetDateTime::now_utc(), std::process::id()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cli_directive(verbose)));
    // Mutex<File> serializes concurrent task writes so events never interleave.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(UtcTime::rfc_3339())
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .init();
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

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

    #[test]
    fn log_file_name_formats_utc_and_pid() {
        let now = datetime!(2026-07-08 14:30:00 UTC);
        assert_eq!(log_file_name(now, 12345), "jhc-20260708T143000Z-12345.log");
    }

    #[test]
    fn cli_directive_switches_on_verbose() {
        assert_eq!(cli_directive(false), "warn");
        assert_eq!(cli_directive(true), "warn,jhc=debug");
    }

    #[test]
    fn tui_logging_active_only_with_verbose_or_rust_log() {
        assert!(!tui_logging_active(false, false));
        assert!(tui_logging_active(true, false));
        assert!(tui_logging_active(false, true));
        assert!(tui_logging_active(true, true));
    }
}
