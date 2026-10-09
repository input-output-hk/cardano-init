//! The once-a-day update-check cache: a tiny JSON file under the OS cache dir
//! holding when we last asked GitHub and what the latest version was. A fresh
//! entry means zero network and zero latency (and keeps us well under GitHub's
//! unauthenticated rate limit). Read/write failures are ignored.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const TTL: Duration = Duration::from_secs(24 * 60 * 60);

const FILE_NAME: &str = "update-check";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    /// Unix seconds of the last successful lookup.
    pub checked_at: u64,
    /// The latest released version seen then (no `v` prefix).
    pub latest: String,
}

impl Cache {
    pub fn is_fresh(&self) -> bool {
        self.is_fresh_at(now())
    }

    fn is_fresh_at(&self, now: u64) -> bool {
        now.saturating_sub(self.checked_at) < TTL.as_secs()
    }
}

/// The cached lookup, if present, well-formed and checked within the last day.
pub fn read_fresh() -> Option<Cache> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str::<Cache>(&text)
        .ok()
        .filter(Cache::is_fresh)
}

/// Record a successful lookup (best-effort).
pub fn write(latest: &str) {
    let Some(path) = path() else { return };
    let cache = Cache {
        checked_at: now(),
        latest: latest.to_string(),
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(&cache) {
        let _ = std::fs::write(path, text);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `<OS cache dir>/cardano-init/update-check`: `~/Library/Caches` on macOS,
/// `$XDG_CACHE_HOME|~/.cache` on Linux, `%LOCALAPPDATA%` on Windows.
fn path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("cardano-init").join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_window() {
        let c = Cache {
            checked_at: 1_000_000,
            latest: "0.3.0".into(),
        };
        assert!(c.is_fresh_at(1_000_000));
        assert!(c.is_fresh_at(1_000_000 + TTL.as_secs() - 1));
        assert!(!c.is_fresh_at(1_000_000 + TTL.as_secs()));
    }

    #[test]
    fn cache_round_trips() {
        let c = Cache {
            checked_at: 42,
            latest: "0.3.0".into(),
        };
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Cache>(&text).unwrap(), c);
        assert!(serde_json::from_str::<Cache>("garbage").is_err());
    }
}
