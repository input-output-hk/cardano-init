//! Version-update check (tech spec §10): when the latest GitHub release is newer
//! than this binary, show a notice before generation with the update command
//! for this install method. GitHub is asked at most once a day (cached).
//!
//! Informational only: it never blocks beyond a 1s deadline, every failure is a
//! silent no-op, and `--json`/non-TTY runs never touch the network.
//!
//! [`start`] fires the lookup early (on a background thread, overlapping
//! interactive selection); [`announce`] joins it right before writing. Without
//! a prior `start` (tests, `list`, `doctor`), `announce` is a no-op.

mod cache;
mod install;

use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::output;
pub use install::InstallMethod;

const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/input-output-hk/cardano-init/releases/latest";

/// Where to download a release by hand (the fallback when the install method
/// is unknown).
pub const RELEASES_URL: &str = "https://github.com/input-output-hk/cardano-init/releases/latest";

/// Opt-out: set (to anything) to disable the check entirely.
const OPT_OUT_ENV: &str = "CARDANO_INIT_NO_UPDATE_CHECK";

const DEADLINE: Duration = Duration::from_secs(1);

const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A newer release is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNotice {
    pub current: String,
    pub latest: String,
    pub install: InstallMethod,
}

impl UpdateNotice {
    /// The command that updates this install, or `None` when the install method
    /// is unknown (show [`RELEASES_URL`] instead).
    pub fn command(&self) -> Option<String> {
        self.install.update_command()
    }
}

/// An in-flight check. The deadline runs from `started`, so interactive
/// think-time counts towards it.
struct Pending {
    rx: Receiver<Option<String>>,
    started: Instant,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

/// Look up the latest version, for human + attended runs only (and unless opted
/// out via `CARDANO_INIT_NO_UPDATE_CHECK` or `CI`): from a fresh cache
/// synchronously, else via GitHub on a background thread.
pub fn start(json: bool) {
    if json
        || !console::user_attended()
        || std::env::var_os(OPT_OUT_ENV).is_some()
        || std::env::var_os("CI").is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel();
    match cache::read_fresh() {
        Some(c) => {
            let _ = tx.send(Some(c.latest));
        }
        None => {
            std::thread::spawn(move || {
                let latest = fetch_latest();
                if let Some(l) = &latest {
                    cache::write(l);
                }
                let _ = tx.send(latest);
            });
        }
    }
    *PENDING.lock().unwrap_or_else(PoisonError::into_inner) = Some(Pending {
        rx,
        started: Instant::now(),
    });
}

/// Print the pre-generation banner if a newer version is available. Joins the
/// pending request within the remaining deadline (behind a spinner only when it
/// isn't already done); a timeout or failure prints nothing.
pub fn announce(json: bool) {
    let Some(pending) = PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    else {
        return;
    };
    let latest = match pending.rx.try_recv() {
        Ok(latest) => latest,
        Err(mpsc::TryRecvError::Disconnected) => None,
        Err(mpsc::TryRecvError::Empty) => {
            let remaining = DEADLINE.saturating_sub(pending.started.elapsed());
            super::with_spinner("Checking for updates…", json, || {
                pending.rx.recv_timeout(remaining).ok().flatten()
            })
        }
    };
    if let Some(notice) = latest.and_then(|l| notice_for(&l)) {
        output::print_update_notice(&notice);
    }
}

fn notice_for(latest: &str) -> Option<UpdateNotice> {
    if !is_newer(latest, CURRENT) {
        return None;
    }
    Some(UpdateNotice {
        current: CURRENT.to_string(),
        latest: latest.trim_start_matches('v').to_string(),
        install: install::detect_current(),
    })
}

fn fetch_latest() -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DEADLINE))
        .build()
        .into();
    let body = agent
        .get(LATEST_RELEASE_API)
        .header(
            "User-Agent",
            concat!("cardano-init/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    parse_release_tag(&body)
}

/// Extract the version from a GitHub release payload (`tag_name`, e.g. `v0.3.0`).
fn parse_release_tag(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let tag = value.get("tag_name")?.as_str()?;
    parse_version(tag).map(|_| tag.trim_start_matches('v').to_string())
}

/// Parse a plain `major.minor.patch` (optional `v` prefix). Pre-releases and
/// anything else are rejected, so they are never offered as an update.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_comparison() {
        assert!(is_newer("v0.3.0", "0.2.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("0.2.0", "0.2.1"));
    }

    #[test]
    fn prerelease_and_garbage_are_never_newer() {
        assert!(!is_newer("0.3.0-rc.1", "0.2.1"));
        assert!(!is_newer("latest", "0.2.1"));
        assert!(!is_newer("1.0", "0.2.1"));
        assert!(!is_newer("1.0.0.0", "0.2.1"));
    }

    #[test]
    fn release_tag_parsed_from_payload() {
        let body = r#"{"tag_name":"v0.3.0","name":"0.3.0"}"#;
        assert_eq!(parse_release_tag(body).as_deref(), Some("0.3.0"));
        assert_eq!(parse_release_tag(r#"{"tag_name":"v0.3.0-rc.1"}"#), None);
        assert_eq!(parse_release_tag(r#"{"message":"rate limited"}"#), None);
        assert_eq!(parse_release_tag("not json"), None);
    }
}
