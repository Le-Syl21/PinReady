//! Per-table scan pipeline — sequential within each table, parallel
//! across tables.
//!
//! Each worker pulls a `ScanJob` from the queue and runs **the same
//! table's complete pipeline** in order:
//!
//! 1. **VPSDB match** — pure in-memory lookup against the pre-synced
//!    `Vec<Game>` snapshot. No I/O.
//! 2. **Media install** — if the table has a high-confidence VPS
//!    match and `medias/bg.png` / `medias/audio.mp3` are missing on
//!    disk, fetch them from VPinMediaDB. Skipped when the file is
//!    already present (file-exists short-circuit; force a refresh by
//!    deleting the file before rescan).
//! 3. **Backglass extraction** — priority chain:
//!    a. `medias/launcher.{png,webp,jpg,jpeg}` — user override
//!    b. `medias/bg.png`                       — vpinmediadb cache
//!    c. `<base>.directb2s`                    — embedded backglass
//!    d. `<base>.vpx` images/                  — last resort
//!    The same worker that may have just installed `medias/bg.png`
//!    in step 2 then reads it in step 3b — no cross-thread file
//!    race possible.
//! 4. **DB write** — `set_backglass(rel_path, bytes, source_mtime)`
//!    and `set_vps_link(...)` on the worker's own DB connection
//!    (WAL mode lets multiple writers commit concurrently).
//! 5. **UI signal** — emit `BgExtraction` on `bg_tx` so the launcher
//!    grid reveals the thumbnail immediately, table by table, as each
//!    worker completes.

use crate::app::BgExtraction;
use crate::db::Database;
use crate::mediadb::{self, MediaDb};
use crate::vpsdb;
use anyhow::Result;
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Bump whenever the matcher chain changes (new strategy, cross-check,
/// confidence shift). Forces a one-shot re-match of every existing
/// `vps_link` row on the next worker run, regardless of cached
/// confidence — a wrong-but-High link from an older matcher gets a
/// chance to flip to the correct vps_id. Stamped into
/// `config.matcher_version` after the bootstrap completes.
pub const MATCHER_VERSION: i64 = 2;

/// One table's job ticket. The worker has everything it needs to
/// process the table without touching shared launcher state.
#[derive(Debug, Clone)]
pub struct ScanJob {
    pub idx: usize,
    pub rel_path: String,
    pub table_dir: PathBuf,
    pub vpx_path: PathBuf,
    pub folder_name: String,
    pub source_mtime: i64,
}

/// Spawn a worker pool and feed it `jobs`. Each worker owns its own
/// `Database` connection (WAL mode → no cross-thread serialization on
/// disjoint rows). Workers terminate when the job queue is empty.
///
/// Returns immediately. Progress flows back to the launcher via
/// `bg_tx`; cancellation via `cancel.store(true)` aborts every worker
/// at its next iteration.
#[allow(clippy::too_many_arguments)]
pub fn spawn_pool(
    jobs: Vec<ScanJob>,
    games: Arc<Vec<vpsdb::models::Game>>,
    media_db: Option<Arc<MediaDb>>,
    bg_tx: Sender<BgExtraction>,
    cancel: Arc<AtomicBool>,
    tables_root: PathBuf,
    scan_generation: u64,
    force_full_rematch: bool,
) {
    if jobs.is_empty() {
        return;
    }

    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 16))
        .unwrap_or(4);

    log::info!(
        "Scan worker pool: {} workers × {} jobs (gen={scan_generation}, full_rematch={force_full_rematch})",
        worker_count,
        jobs.len()
    );

    let (job_tx, job_rx) = crossbeam_channel::unbounded::<ScanJob>();
    for j in jobs {
        let _ = job_tx.send(j);
    }
    drop(job_tx); // closes the queue: workers exit on `Disconnected`.

    for w in 0..worker_count {
        let job_rx = job_rx.clone();
        let bg_tx = bg_tx.clone();
        let cancel = cancel.clone();
        let games = games.clone();
        let media_db = media_db.clone();
        let tables_root = tables_root.clone();

        std::thread::Builder::new()
            .name(format!("pinready-scan-{scan_generation}-{w}"))
            .spawn(move || {
                // Per-worker DB connection. WAL mode means we don't
                // serialize against the other workers on small writes
                // to disjoint `backglass.rel_path` / `vps_link.rel_path`
                // rows.
                let db = match Database::open(None) {
                    Ok(db) => db,
                    Err(e) => {
                        log::error!("scan worker {w} failed to open DB: {e}");
                        return;
                    }
                };

                while let Ok(job) = job_rx.recv() {
                    if cancel.load(Ordering::SeqCst) {
                        log::debug!("scan worker {w} cancelled");
                        return;
                    }
                    if let Err(e) = process_table(
                        &job,
                        &games,
                        media_db.as_deref(),
                        &db,
                        &bg_tx,
                        &tables_root,
                        scan_generation,
                        force_full_rematch,
                    ) {
                        log::warn!("scan worker {w}: {} failed: {e}", job.rel_path);
                    }
                }
            })
            .expect("spawn scan worker thread");
    }
    // The cloned `bg_tx` and `job_rx` we still hold above end here;
    // each worker keeps its own clones, so the channels stay open
    // until the last worker exits.
    drop(bg_tx);
    drop(job_rx);
}

/// Run the full pipeline for a single table.
#[allow(clippy::too_many_arguments)]
fn process_table(
    job: &ScanJob,
    games: &[vpsdb::models::Game],
    media_db: Option<&MediaDb>,
    db: &Database,
    bg_tx: &Sender<BgExtraction>,
    tables_root: &Path,
    gen: u64,
    force_full_rematch: bool,
) -> Result<()> {
    // 1. VPSDB match — decide whether to re-evaluate or reuse the
    //    existing link. Same heuristic as the old catalog_worker:
    //    matcher_version upgrade or low-confidence prior link → rematch.
    let existing = db.get_vps_link(&job.rel_path);
    let needs_rematch = if force_full_rematch {
        true
    } else {
        match &existing {
            Some((_, _, conf, _, _, _, _, _)) => conf.eq_ignore_ascii_case("low"),
            None => true,
        }
    };

    let matched_vps_id: Option<String> = if needs_rematch {
        match vpsdb::match_table_from_paths(games, &job.vpx_path, &job.folder_name) {
            Some(m) => {
                let prev_id = existing.as_ref().map(|e| e.0.as_str());
                let prev_conf = existing.as_ref().map(|e| e.2.as_str()).unwrap_or("none");
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
                let vps_id_changed = prev_id.is_some() && Some(m.game.id.as_str()) != prev_id;
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
                if vps_id_changed {
                    // The previously cached media on disk are for the
                    // wrong game now — drop them so the install step
                    // below re-fetches under the new id.
                    db.delete_backglass(&job.rel_path)?;
                    for stale in ["bg.png", "audio.mp3"] {
                        let p = job.table_dir.join("medias").join(stale);
                        if p.is_file() {
                            let _ = std::fs::remove_file(&p);
                            log::info!("Removed stale media {}", p.display());
                        }
                    }
                }
                Some(m.game.id.clone())
            }
            None => existing.as_ref().map(|e| e.0.clone()),
        }
    } else {
        existing.as_ref().map(|e| e.0.clone())
    };

    // 1b. Update-available flag — compare the latest published
    // tableFile updated_at against the local .vpx mtime. This drives
    // the "↑" badge in the launcher.
    if let Some(vps_id) = &matched_vps_id {
        if let Some(game) = games.iter().find(|g| &g.id == vps_id) {
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
        }
    }

    // 2. Media install — fetch bg.png and audio.mp3 from VPinMediaDB
    // when missing. The file-exists short-circuit (in install_if_missing)
    // means we never re-download something that already lives on disk.
    if let (Some(vps_id), Some(mdb)) = (&matched_vps_id, media_db) {
        if let Some((url, md5)) = mdb.bg_url_md5(vps_id) {
            install_if_missing(&job.table_dir, "bg.png", url, md5);
        }
        if let Some((url, md5)) = mdb.audio_url_md5(vps_id) {
            install_if_missing(&job.table_dir, "audio.mp3", url, md5);
        }
    }

    // 3. Backglass extraction — priority chain. Same worker that just
    // wrote `medias/bg.png` now reads it (or any other source) — no
    // cross-thread file race.
    let bytes = crate::assets::extract_backglass_from_launcher_override(&job.table_dir)
        .or_else(|| crate::assets::extract_backglass_from_vpinmediadb(&job.table_dir))
        .or_else(|| {
            let b2s = job.vpx_path.with_extension("directb2s");
            if b2s.is_file() {
                crate::assets::extract_backglass_from_b2s(&b2s)
            } else {
                None
            }
        })
        .or_else(|| crate::assets::extract_backglass_from_vpx(&job.vpx_path));

    // 4 + 5. DB write + UI signal.
    if let Some(bytes) = bytes {
        let rel_path = job
            .vpx_path
            .strip_prefix(tables_root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| job.vpx_path.to_string_lossy().into_owned());
        let _ = bg_tx.send((gen, job.idx, rel_path, bytes, job.source_mtime));
    }
    Ok(())
}

/// Download + write the asset only if `medias/<filename>` doesn't
/// already exist. The file-exists short-circuit replaces the previous
/// md5-comparison branch — a flaky upstream md5 (server-side rebuild,
/// schema migration) shouldn't trigger pointless re-downloads. To
/// force a refresh: delete the file before rescan.
fn install_if_missing(table_dir: &Path, filename: &str, url: &str, expected_md5: &str) {
    let target = table_dir.join("medias").join(filename);
    if target.exists() {
        return;
    }
    match mediadb::fetch_asset(url, expected_md5) {
        Ok(bytes) => {
            if let Err(e) = mediadb::install_asset(table_dir, filename, &bytes) {
                log::warn!(
                    "MediaDb install {filename} for {} failed: {e}",
                    table_dir.display()
                );
            } else {
                log::info!("MediaDb installed {filename} in {}", table_dir.display());
            }
        }
        Err(e) => {
            log::warn!(
                "MediaDb fetch {filename} for {} failed: {e}",
                table_dir.display()
            );
        }
    }
}
