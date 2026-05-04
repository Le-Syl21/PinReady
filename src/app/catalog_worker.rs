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

/// Bump whenever the matcher chain changes (new strategy, cross-check,
/// confidence shifts). Forces a one-shot re-match of every existing
/// `vps_link` row on the next worker run, regardless of cached
/// confidence — a wrong-but-High link from an older matcher (e.g.
/// "Batman '66" matched to "Flash" via a fake `cGameName`) gets a
/// chance to flip to the correct vps_id. After re-matching, the worker
/// stamps the new version into `config.matcher_version`.
const MATCHER_VERSION: i64 = 2;

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
/// Run the enrichment synchronously on the calling thread. Called from
/// the unified rescan worker (`launcher::scan_tables`) so the backglass
/// extractor can rely on `medias/bg.png` being on disk before it walks
/// the priority chain. Returns when every job has been processed (or
/// the cancel flag was tripped by a fresh rescan).
pub fn run(jobs: Vec<EnrichmentJob>, cancel: Arc<AtomicBool>) -> anyhow::Result<()> {
    let db = Database::open(None)?;
    let mirror = db.mirror_base_url();

    // 1. VPSDB
    let vps_cache = vpsdb::fetch::VpsDbCache::new(vpsdb::fetch::VpsDbCache::default_dir());
    let (games, vps_outcome) = vpsdb::fetch::sync_if_stale(&vps_cache)?;
    log_vps_outcome(vps_outcome);
    if games.is_empty() {
        log::warn!("Catalog enrichment: VPSDB returned 0 games — aborting");
        return Ok(());
    }

    // 2. MediaDB
    let media_db = match MediaDb::sync(MediaDb::default_cache_dir(), mirror.as_deref()) {
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

    // One-shot re-match: triggered when the persisted matcher version
    // is older than this binary's. Pays a per-table OLE open on the
    // upgrade run, then settles back to "low-only re-match" on
    // subsequent runs.
    let stored_matcher_version: i64 = db
        .get_config("matcher_version")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let force_full_rematch = stored_matcher_version < MATCHER_VERSION;
    if force_full_rematch {
        log::info!(
            "Matcher upgraded ({stored_matcher_version} → {MATCHER_VERSION}); re-evaluating every vps_link this run"
        );
    }

    let mut matched_high = 0usize;
    let mut matched_low = 0usize;
    let mut media_dl = 0usize;
    let mut unmatched = 0usize;
    let mut update_available_count = 0usize;

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
        // Re-use existing link if present and Medium+ confidence. Low
        // links get re-matched on every run because the matcher chain
        // keeps growing (new strategies were added: TableName-based) —
        // a previously "low" folder-fuzzy match may now upgrade to
        // High via tablename_exact.
        let existing = db.get_vps_link(&job.rel_path);
        let needs_rematch = if force_full_rematch {
            true
        } else {
            match &existing {
                Some((_, _, conf, _, _, _, _, _)) => conf.eq_ignore_ascii_case("low"),
                None => true,
            }
        };

        let (vps_id, table_id, confidence, strategy, vps_updated_at) = if needs_rematch {
            match vpsdb::match_table_from_paths(&games, &job.vpx_path, &job.folder_name) {
                Some(m) => {
                    let game_ts = m.game.updated_at.unwrap_or(0);
                    let prev_id = existing.as_ref().map(|e| e.0.as_str());
                    let prev_conf = existing.as_ref().map(|e| e.2.as_str()).unwrap_or("none");
                    let vps_id_changed = prev_id.is_some() && Some(m.game.id.as_str()) != prev_id;
                    if Some(m.game.id.as_str()) != prev_id
                        || !prev_conf.eq_ignore_ascii_case(&m.confidence.to_string())
                    {
                        log::info!(
                            "Re-match {}: {} ({}) → {} ({}, strategy={})",
                            job.rel_path,
                            prev_id.unwrap_or("-"),
                            prev_conf,
                            m.game.id,
                            m.confidence,
                            m.strategy,
                        );
                    }
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
                    // vps_id flipped → the previously cached media on
                    // disk are for the wrong game (Batman 66 carrying a
                    // Williams Flash bg.png is the canonical example).
                    // Drop the cached md5s + the on-disk files so the
                    // next mediadb pass re-downloads under the new id.
                    if vps_id_changed {
                        db.clear_link_media_md5s(&job.rel_path)?;
                        db.delete_backglass(&job.rel_path)?;
                        for stale in ["bg.png", "audio.mp3"] {
                            let p = job.table_dir.join("medias").join(stale);
                            if p.is_file() {
                                if let Err(e) = std::fs::remove_file(&p) {
                                    log::warn!("Failed to remove stale {}: {e}", p.display());
                                } else {
                                    log::info!("Removed stale media {}", p.display());
                                }
                            }
                        }
                    }
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
                None => match existing {
                    // No new match — keep the existing low link rather
                    // than orphaning the row.
                    Some((id, tid, conf, strat, ts, _, _, _)) => (id, tid, conf, strat, ts),
                    None => {
                        unmatched += 1;
                        continue;
                    }
                },
            }
        } else {
            let (id, tid, conf, strat, ts, _, _, _) = existing.unwrap();
            (id, tid, conf, strat, ts)
        };

        // 3. Update detection — compare the local .vpx mtime against
        // the most recent `tableFiles[*].updatedAt` published in the
        // catalog. This catches the real case the user cares about:
        // "the catalog has a newer version than what's on my disk".
        //
        // Earlier iteration compared `Game.updated_at` (catalog-edit
        // ts) vs the snapshot stored at link time; that's a "did the
        // catalog entry move?" signal which misses two locally-stored
        // versions of the same game (both link to the same `vps_id`,
        // both see the same `Game.updated_at`, neither flagged).
        //
        // Stale tolerance: 24h. Author-clock skew + zip-extraction
        // mtimes that match the publish date by accident shouldn't
        // trigger phantom updates. Confidence filter dropped — a
        // "low" match that picked the right `vps_id` is still useful;
        // a wrong-vps_id match would simply compare against unrelated
        // `tableFiles` and rarely flag, which is acceptable.
        // _vps_updated_at is unused by the new logic but kept in DB
        // for backwards compat with `vps_link` row shape.
        let _ = vps_updated_at;
        if let Some(game) = games.iter().find(|g| g.id == vps_id) {
            let latest_tf_ts: i64 = game
                .table_files
                .iter()
                .filter_map(|tf| tf.updated_at)
                .max()
                .unwrap_or(0);
            let local_mtime_ms: i64 = std::fs::metadata(&job.vpx_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let tolerance_ms = 24 * 3600 * 1000;
            let outdated = latest_tf_ts > 0
                && local_mtime_ms > 0
                && latest_tf_ts > local_mtime_ms + tolerance_ms;
            db.set_update_available(&job.rel_path, outdated)?;
            if outdated {
                update_available_count += 1;
            }
        }

        // 4. Media fetch (only if MediaDb available + match present).
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
        "Catalog enrichment done: matched={} (low={}) media={} unmatched={} updates={}",
        matched_high + matched_low,
        matched_low,
        media_dl,
        unmatched,
        update_available_count
    );

    // Persist the matcher version we just settled the cache against
    // so the next run skips the full re-match unless we bump the
    // constant again.
    if force_full_rematch {
        if let Err(e) = db.set_config("matcher_version", &MATCHER_VERSION.to_string()) {
            log::warn!("Failed to persist matcher_version: {e}");
        }
    }

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
