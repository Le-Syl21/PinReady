//! Background worker that runs the catalog enrichment pipeline:
//!
//!   1. Sync VPSDB (cached per `lastUpdated.json` timestamp).
//!   2. Sync VPinMediaDB.
//!   3. For each scanned table without a `vps_link` row, run the
//!      matcher and persist the link.
//!   4. For each linked table whose media md5 has shifted, fetch the
//!      backglass + audio + wheel from VPinMediaDB into
//!      `<table>/medias/`.
//!
//! Runs in its own `std::thread` so a slow first-time download
//! (~6.7 MB JSON + per-table assets) doesn't stall the UI. The main
//! thread only learns the worker has finished via the `Receiver`'s
//! disconnect — there's no per-step progress for now (we'll add a
//! channel-based progress feed in v0.9 alongside the launcher hover UI).

use crate::db::Database;
use crate::mediadb::{self, MediaDb};
use crate::vpsdb::{self, fetch::SyncOutcome, matcher::MatchConfidence};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One row from the launcher's `tables` Vec, materialised so we can
/// hand it off to the worker thread without holding any borrows on
/// `App`.
#[derive(Debug, Clone)]
pub struct EnrichmentJob {
    pub rel_path: String,
    pub table_dir: PathBuf,
    pub vpx_path: PathBuf,
    pub folder_name: String,
}

/// Kick off enrichment for the given jobs. Returns immediately. The
/// thread does its own DB writes; the launcher doesn't need a
/// receiver — the next scan picks up `vps_link` rows and any media
/// files dropped on disk via the regular file scan.
///
/// `cancel` is a one-shot signal flipped to `true` by the launcher
/// when a fresh `scan_tables()` supersedes this run. The worker
/// checks it between table iterations and bails out gracefully —
/// this avoids 4× concurrent workers spamming the same DLs when the
/// user clicks Rebuild a few times in a row.
pub fn spawn(jobs: Vec<EnrichmentJob>, cancel: Arc<AtomicBool>) {
    if jobs.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = run(jobs, cancel) {
            log::error!("Catalog enrichment worker failed: {e}");
        }
    });
}

fn run(jobs: Vec<EnrichmentJob>, cancel: Arc<AtomicBool>) -> anyhow::Result<()> {
    let db = Database::open(None)?;

    // 1. VPSDB
    let vps_cache = vpsdb::fetch::VpsDbCache::new(vpsdb::fetch::VpsDbCache::default_dir());
    let (games, vps_outcome) = vpsdb::fetch::sync_if_stale(&vps_cache)?;
    log_vps_outcome(vps_outcome);
    if games.is_empty() {
        log::warn!("Catalog enrichment: VPSDB returned 0 games — aborting");
        return Ok(());
    }

    // 2. MediaDB
    let media_db = match MediaDb::sync(MediaDb::default_cache_dir()) {
        Ok(m) => Some(m),
        Err(e) => {
            log::warn!("Catalog enrichment: MediaDb sync failed ({e}) — match-only run");
            None
        }
    };

    log::info!(
        "Catalog enrichment running on {} tables (mediadb {})",
        jobs.len(),
        if media_db.is_some() { "ON" } else { "OFF" }
    );

    let mut matched_high = 0usize;
    let mut matched_low = 0usize;
    let mut media_dl = 0usize;
    let mut unmatched = 0usize;

    for job in &jobs {
        // Bail out if a fresh scan superseded us.
        if cancel.load(Ordering::SeqCst) {
            log::info!(
                "Catalog enrichment canceled mid-run after {} matched ({} unmatched)",
                matched_high + matched_low,
                unmatched
            );
            return Ok(());
        }
        // Re-use existing link if present.
        let existing = db.get_vps_link(&job.rel_path);

        let (vps_id, table_id, confidence, strategy, vps_updated_at) = match existing {
            Some((id, tid, conf, strat, ts, _, _, _)) => (id, tid, conf, strat, ts),
            None => match vpsdb::match_table_from_paths(&games, &job.vpx_path, &job.folder_name) {
                Some(m) => {
                    let game_ts = m.game.updated_at.unwrap_or(0);
                    db.set_vps_link(
                        &job.rel_path,
                        &m.game.id,
                        None,
                        &m.confidence.to_string(),
                        m.strategy,
                        game_ts,
                        None,
                        None,
                        None,
                    )?;
                    if m.confidence >= MatchConfidence::Medium {
                        matched_high += 1;
                    } else {
                        matched_low += 1;
                    }
                    (
                        m.game.id.clone(),
                        None,
                        m.confidence.to_string(),
                        m.strategy.to_string(),
                        game_ts,
                    )
                }
                None => {
                    unmatched += 1;
                    continue;
                }
            },
        };

        // 3. Media fetch (only if MediaDb available + match present).
        let Some(ref mdb) = media_db else { continue };
        // Fetch the two assets we care about for hover preview —
        // `bg.png` and `audio.mp3`. Other types in vpinmediadb (wheel,
        // dmd, table, fss, video, …) are deliberately skipped. We
        // re-DL only when (a) URL exists, (b) md5 differs from what we
        // last stored, (c) target file isn't already on disk under the
        // user's hand. Filenames mirror the catalog's factory names —
        // no parenthesized labels.
        let cur_link = db.get_vps_link(&job.rel_path);
        let cur_bg_md5 = cur_link.as_ref().and_then(|l| l.5.clone());
        let cur_audio_md5 = cur_link.as_ref().and_then(|l| l.6.clone());
        let cur_wheel_md5 = cur_link.as_ref().and_then(|l| l.7.clone());

        let mut new_bg_md5 = cur_bg_md5.clone();
        let mut new_audio_md5 = cur_audio_md5.clone();

        if let Some((url, md5)) = mdb.bg_url_md5(&vps_id) {
            if try_install_asset(cur_bg_md5.as_deref(), md5, url, &job.table_dir, "bg.png")
                .unwrap_or(false)
            {
                new_bg_md5 = Some(md5.to_string());
                media_dl += 1;
            }
        }
        if let Some((url, md5)) = mdb.audio_url_md5(&vps_id) {
            if try_install_asset(
                cur_audio_md5.as_deref(),
                md5,
                url,
                &job.table_dir,
                "audio.mp3",
            )
            .unwrap_or(false)
            {
                new_audio_md5 = Some(md5.to_string());
                media_dl += 1;
            }
        }

        // Update md5s if any changed.
        if new_bg_md5 != cur_bg_md5 || new_audio_md5 != cur_audio_md5 {
            db.set_vps_link(
                &job.rel_path,
                &vps_id,
                table_id.as_deref(),
                &confidence,
                &strategy,
                vps_updated_at,
                new_bg_md5.as_deref(),
                new_audio_md5.as_deref(),
                cur_wheel_md5.as_deref(),
            )?;
        }
    }

    log::info!(
        "Catalog enrichment done: matched={} (low={}) media={} unmatched={}",
        matched_high + matched_low,
        matched_low,
        media_dl,
        unmatched
    );
    Ok(())
}

/// Returns Ok(true) if the asset was downloaded + installed; Ok(false)
/// if skipped (md5 matches or already on disk); Err on hard failure.
fn try_install_asset(
    cached_md5: Option<&str>,
    remote_md5: &str,
    url: &str,
    table_dir: &Path,
    filename: &str,
) -> anyhow::Result<bool> {
    if cached_md5 == Some(remote_md5) {
        return Ok(false);
    }
    // Also skip if the file exists on disk but our DB lost track —
    // user may have hand-placed something they want to keep.
    let target = table_dir.join("medias").join(filename);
    if target.exists() && cached_md5.is_none() {
        log::debug!(
            "MediaDb skip {filename} in {}: file exists, no md5 in DB → assume user-managed",
            table_dir.display()
        );
        return Ok(false);
    }
    let bytes = mediadb::fetch_asset(url, remote_md5)?;
    mediadb::install_asset(table_dir, filename, &bytes)?;
    log::info!("MediaDb installed {filename} in {}", table_dir.display());
    Ok(true)
}

fn log_vps_outcome(o: SyncOutcome) {
    match o {
        SyncOutcome::Fresh { games, bytes } => {
            log::info!("VPSDB first sync: {games} games, {} KB", bytes / 1024)
        }
        SyncOutcome::Updated { games, bytes } => {
            log::info!("VPSDB refreshed: {games} games, {} KB", bytes / 1024)
        }
        SyncOutcome::Current { games } => log::info!("VPSDB cache current ({games} games)"),
        SyncOutcome::Offline => log::warn!("VPSDB offline + no cache → enrichment is a no-op"),
    }
}
