//! VPS DB sync: download `vpsdb.json` + `lastUpdated.json`, cache on
//! disk, and re-fetch only when the remote timestamp moved.
//!
//! Cache layout (under `dirs::cache_dir() / "pinready" / "vpsdb"`):
//! ```text
//! vpsdb.json              # full catalog payload (~6.7 MB)
//! vpsdb.last_updated      # unix-millis timestamp matching lastUpdated.json
//! ```
//!
//! The remote `lastUpdated.json` is a tiny (~13 byte) integer; we GET
//! it before deciding to refresh the big payload. Saves ~6.6 MB on
//! every PinReady startup once the cache is warm.

use anyhow::{Context, Result};
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

/// Process-wide ureq agent — see `mediadb::http_agent` for rationale.
fn http_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent()
    })
}
use std::path::PathBuf;

use super::models::Game;

/// Production endpoints. Static GitHub Pages URLs — no auth required.
const VPSDB_URL: &str = "https://virtualpinballspreadsheet.github.io/vps-db/db/vpsdb.json";
const LAST_UPDATED_URL: &str =
    "https://virtualpinballspreadsheet.github.io/vps-db/lastUpdated.json";

/// What happened during a `sync_if_stale` call.
#[derive(Debug, Clone, Copy)]
pub enum SyncOutcome {
    /// First sync — full catalog downloaded.
    Fresh { games: usize, bytes: usize },
    /// Remote timestamp moved past local; refreshed payload.
    Updated { games: usize, bytes: usize },
    /// Local cache is current; nothing changed on disk.
    Current { games: usize },
    /// Network unreachable; falling back to whatever cache exists. The
    /// caller may still get useful catalog data via [`VpsDbCache::load`].
    Offline,
}

/// On-disk catalog cache + helpers to load it without re-fetching.
pub struct VpsDbCache {
    json_path: PathBuf,
    timestamp_path: PathBuf,
}

impl VpsDbCache {
    /// Default location: `<dirs::cache_dir>/pinready/vpsdb/`. The dir
    /// is created on first write — readers tolerate its absence.
    pub fn default_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("pinready")
            .join("vpsdb")
    }

    pub fn new(dir: PathBuf) -> Self {
        Self {
            json_path: dir.join("vpsdb.json"),
            timestamp_path: dir.join("vpsdb.last_updated"),
        }
    }

    /// Read the cached catalog, if present. Returns `Ok(None)` on
    /// missing-cache (callers should call [`sync_if_stale`] first).
    pub fn load(&self) -> Result<Option<Vec<Game>>> {
        if !self.json_path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&self.json_path)
            .with_context(|| format!("Failed to read {}", self.json_path.display()))?;
        let games: Vec<Game> = serde_json::from_slice(&bytes)
            .context("Cached vpsdb.json failed to parse — corrupt cache?")?;
        Ok(Some(games))
    }

    fn read_local_timestamp(&self) -> Option<i64> {
        std::fs::read_to_string(&self.timestamp_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    fn write_local_timestamp(&self, ts: i64) -> Result<()> {
        if let Some(parent) = self.timestamp_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.timestamp_path, ts.to_string())
            .with_context(|| format!("Failed to write {}", self.timestamp_path.display()))
    }
}

/// Hit `lastUpdated.json` to see if our cached timestamp is current.
/// Returns the remote timestamp (unix-millis) on success.
fn fetch_remote_timestamp() -> Result<i64> {
    let resp = http_agent()
        .get(LAST_UPDATED_URL)
        .header("User-Agent", "PinReady")
        .call()
        .context("Failed to fetch VPS DB lastUpdated.json")?;
    let text = resp
        .into_body()
        .read_to_string()
        .context("Failed to read lastUpdated.json body")?;
    let ts: i64 = text
        .trim()
        .parse()
        .with_context(|| format!("Invalid timestamp from VPS DB: {text:?}"))?;
    Ok(ts)
}

/// GET the full catalog. Buffers into memory because we want the raw
/// bytes for both parsing and disk-caching.
fn fetch_full_payload() -> Result<Vec<u8>> {
    let resp = http_agent()
        .get(VPSDB_URL)
        .header("User-Agent", "PinReady")
        .call()
        .context("Failed to fetch vpsdb.json")?;
    let mut body = resp.into_body();
    let mut buf = Vec::with_capacity(7 * 1024 * 1024); // ~6.7 MB seed
    body.as_reader()
        .read_to_end(&mut buf)
        .context("Failed to read vpsdb.json body")?;
    Ok(buf)
}

/// One-shot: ensure the local cache is current, refresh if not, return
/// the parsed catalog plus a description of what happened.
///
/// Network failures fall through to [`SyncOutcome::Offline`] — the
/// caller can still load whatever stale cache exists.
pub fn sync_if_stale(cache: &VpsDbCache) -> Result<(Vec<Game>, SyncOutcome)> {
    let remote_ts = match fetch_remote_timestamp() {
        Ok(ts) => Some(ts),
        Err(e) => {
            log::warn!("VPS DB remote timestamp unreachable: {e}");
            None
        }
    };

    let local_ts = cache.read_local_timestamp();
    let cache_exists = cache.json_path.exists();

    // Decide: refresh or reuse?
    let needs_refresh = match (remote_ts, local_ts, cache_exists) {
        (None, _, true) => false, // offline + cache → reuse
        (None, _, false) => return Ok((Vec::new(), SyncOutcome::Offline)),
        (Some(_), _, false) => true,           // no cache → fetch
        (Some(rt), Some(lt), true) => rt > lt, // remote newer
        (Some(_), None, true) => true,         // missing timestamp → resync
    };

    if !needs_refresh {
        let games = cache
            .load()?
            .ok_or_else(|| anyhow::anyhow!("Cache exists but failed to load"))?;
        return Ok((games.clone(), SyncOutcome::Current { games: games.len() }));
    }

    // Refresh: download payload, parse, write cache + timestamp.
    let bytes = fetch_full_payload()?;
    let games: Vec<Game> =
        serde_json::from_slice(&bytes).context("Remote vpsdb.json failed to parse")?;
    if let Some(parent) = cache.json_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cache.json_path, &bytes)
        .with_context(|| format!("Failed to write cache to {}", cache.json_path.display()))?;
    if let Some(ts) = remote_ts {
        cache.write_local_timestamp(ts)?;
    }

    let outcome = if local_ts.is_none() && !cache_exists {
        SyncOutcome::Fresh {
            games: games.len(),
            bytes: bytes.len(),
        }
    } else {
        SyncOutcome::Updated {
            games: games.len(),
            bytes: bytes.len(),
        }
    };
    Ok((games, outcome))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_load_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let cache = VpsDbCache::new(tmp.path().to_path_buf());
        assert!(cache.load().unwrap().is_none());
    }

    #[test]
    fn cache_timestamp_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let cache = VpsDbCache::new(tmp.path().to_path_buf());
        cache.write_local_timestamp(1_777_125_168_204).unwrap();
        assert_eq!(cache.read_local_timestamp(), Some(1_777_125_168_204));
    }

    #[test]
    fn cache_timestamp_invalid_yields_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vpsdb.last_updated");
        std::fs::write(&path, "garbage").unwrap();
        let cache = VpsDbCache::new(tmp.path().to_path_buf());
        assert_eq!(cache.read_local_timestamp(), None);
    }
}
