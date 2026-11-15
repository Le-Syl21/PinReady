//! Keeping the tiles' community ratings and "update available" badges
//! current while the launcher stays open.
//!
//! The scan jobs link a table to its VPS entry and set its update flag in the
//! database, but nothing told the screen: the tiles only learned it at the next
//! index build, which meant closing a table and coming back. And the VPS
//! database itself was only synced when a table needed scanning. Two fixes:
//! the tiles re-read the links while a scan runs and when it ends, and the
//! catalog is refreshed on its own, at start and every few hours.

use super::{App, launcher::rel_to};
use std::time::{Duration, Instant};

/// How often the background housekeeping looks at what is due.
const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(60);
/// How often the tiles re-read the links while a scan is running.
const LINKS_WHILE_SCANNING: Duration = Duration::from_secs(2);
/// Config key holding when the catalog was last refreshed (unix seconds).
const KEY_CATALOG_AT: &str = "catalog_refreshed_at";
/// The VPS database changes a few times a day; once every six hours is plenty.
const CATALOG_MAX_AGE_SECS: i64 = 6 * 3600;

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether the catalog is due for a refresh: never done, or older than the limit.
fn catalog_due(refreshed_at: Option<i64>, now: i64) -> bool {
    refreshed_at.is_none_or(|t| now - t >= CATALOG_MAX_AGE_SECS)
}

impl App {
    /// Copy the link, update flag and reliability of every table from the
    /// database to the tiles. One query, a few thousand rows at most.
    pub(super) fn refresh_vps_links(&mut self) {
        let links = self.db.get_vps_overview();
        let dir = std::path::PathBuf::from(&self.tables_dir);
        for table in &mut self.tables {
            if let Some((id, update, reliable)) = links.get(&rel_to(&dir, &table.path)) {
                table.vps_id = Some(id.clone());
                table.update_available = *update;
                table.vps_shareable = *reliable;
            }
        }
        self.vps_links_read_at = Some(Instant::now());
    }

    /// While a scan runs, show what it has linked as it goes.
    pub(super) fn refresh_vps_links_while_scanning(&mut self) {
        if self.bg_rx.is_some()
            && self
                .vps_links_read_at
                .is_none_or(|t| t.elapsed() >= LINKS_WHILE_SCANNING)
        {
            self.refresh_vps_links();
        }
    }

    /// Once a minute, start whatever is due: the community ratings (once a
    /// day, they decide for themselves) and the catalog (every six hours).
    pub(super) fn run_housekeeping(&mut self) {
        if self
            .housekeeping_at
            .is_some_and(|t| t.elapsed() < HOUSEKEEPING_EVERY)
        {
            return;
        }
        self.housekeeping_at = Some(Instant::now());
        self.start_ratings_refresh_if_stale();
        self.start_catalog_refresh_if_due();
    }

    fn start_catalog_refresh_if_due(&mut self) {
        if self.catalog_refresh_rx.is_some() || !self.db.catalog_enrichment_enabled() {
            return;
        }
        let refreshed_at: Option<i64> = self
            .db
            .get_config(KEY_CATALOG_AT)
            .and_then(|s| s.parse().ok());
        if !catalog_due(refreshed_at, unix_now()) {
            return;
        }
        let dir = std::path::PathBuf::from(&self.tables_dir);
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.catalog_refresh_rx = Some(rx);
        let spawned = std::thread::Builder::new()
            .name("pinready-catalog-refresh".into())
            .spawn(move || {
                let _ = tx.send(refresh_catalog(&dir));
            });
        if spawned.is_err() {
            self.catalog_refresh_rx = None;
        }
    }

    /// Pick up the result of the catalog refresh, and show it.
    pub(super) fn process_catalog_refresh(&mut self) {
        let Some(rx) = &self.catalog_refresh_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.catalog_refresh_rx = None;
                match result {
                    Ok(changed) => {
                        log::info!("Catalog refreshed: {changed} update flag(s) changed");
                        // Only after a success: a failed one (offline) is tried again
                        let _ = self.db.set_config(KEY_CATALOG_AT, &unix_now().to_string());
                        self.refresh_vps_links();
                    }
                    Err(e) => log::warn!("Catalog refresh failed: {e}"),
                }
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.catalog_refresh_rx = None;
            }
        }
    }
}

/// Sync the VPS database, then redo the update flag of every linked table.
/// Returns how many flags changed.
fn refresh_catalog(tables_dir: &std::path::Path) -> Result<usize, String> {
    let cache =
        crate::vpsdb::fetch::VpsDbCache::new(crate::vpsdb::fetch::VpsDbCache::default_dir());
    let (games, _) = crate::vpsdb::fetch::sync_if_stale(&cache).map_err(|e| format!("{e:#}"))?;
    if games.is_empty() {
        return Err("no VPS database (offline, no copy)".into());
    }
    let db = crate::db::Database::open(None).map_err(|e| format!("{e:#}"))?;
    let mut changed = 0;
    for (rel_path, (vps_id, was_flagged, _)) in db.get_vps_overview() {
        let Some(game) = games.iter().find(|g| g.id == vps_id) else {
            continue;
        };
        let latest = game
            .table_files
            .iter()
            .filter_map(|tf| tf.updated_at)
            .max()
            .unwrap_or(0);
        let local_ms = std::fs::metadata(tables_dir.join(&rel_path))
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let outdated = crate::scan_worker::update_is_available(latest, local_ms);
        if outdated != was_flagged {
            db.set_update_available(&rel_path, outdated)
                .map_err(|e| format!("{e:#}"))?;
            changed += 1;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_due_when_never_done_or_old() {
        let now = 1_800_000_000;
        assert!(catalog_due(None, now));
        assert!(catalog_due(Some(now - CATALOG_MAX_AGE_SECS), now));
        assert!(!catalog_due(Some(now - CATALOG_MAX_AGE_SECS + 1), now));
        assert!(!catalog_due(Some(now), now));
    }
}
