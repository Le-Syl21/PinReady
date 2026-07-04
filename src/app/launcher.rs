use super::*;

/// Return the most-recent Unix-seconds mtime across every candidate
/// backglass source for a given table folder: `medias/launcher.*`,
/// `medias/bg.png` (vpinmediadb-installed cache), `.directb2s`, and the
/// `.vpx` itself. Missing files don't participate.
/// Used at scan time to invalidate the SQLite cache when any source
/// changes — especially a `launcher.*` override added after the
/// initial scan, or a fresh `medias/bg.png` from a catalog enrichment
/// run. Silent on any fs error — a 0 mtime just means "don't consider
/// this file newer than the cache".
fn max_source_mtime(table_dir: &std::path::Path, vpx_path: &std::path::Path) -> i64 {
    let b2s = vpx_path.with_extension("directb2s");
    let medias = table_dir.join("medias");
    let candidates = [
        medias.join("launcher.png"),
        medias.join("launcher.webp"),
        medias.join("launcher.jpg"),
        medias.join("launcher.jpeg"),
        medias.join("bg.png"),
        b2s,
        vpx_path.to_path_buf(),
    ];
    let mut max_mtime = 0i64;
    for candidate in &candidates {
        if let Ok(meta) = std::fs::metadata(candidate) {
            if let Ok(m) = meta.modified() {
                if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                    max_mtime = max_mtime.max(d.as_secs() as i64);
                }
            }
        }
    }
    max_mtime
}

/// mtime helper used by the VBS-patch scanner: only the `.vpx` and its
/// `.vbs` sidecar matter (the launcher.* override is irrelevant to VBS
/// classification). Same semantics and failure mode as
/// `max_source_mtime`.
fn max_vbs_mtime(vpx_path: &std::path::Path) -> i64 {
    let sidecar = vpx_path.with_extension("vbs");
    let candidates = [vpx_path.to_path_buf(), sidecar];
    let mut max_mtime = 0i64;
    for candidate in &candidates {
        if let Ok(meta) = std::fs::metadata(candidate) {
            if let Ok(m) = meta.modified() {
                if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                    max_mtime = max_mtime.max(d.as_secs() as i64);
                }
            }
        }
    }
    max_mtime
}

/// Parse a percentage from a VPX SetProgress message.
/// Examples: "Initializing Visuals... 10%" → Some(0.10), "Loading..." → None
/// Describe an abnormal child-process exit (signal / crash / fault) and
/// point the user at where the OS stored the dump. Cross-platform:
///
///   - **Linux**: `core_pattern` (systemd-coredump or literal file).
///   - **macOS**: same signal info, plus `~/Library/Logs/DiagnosticReports/`.
///   - **Windows**: NTSTATUS-style exit code + Windows Error Reporting
///     (`%LOCALAPPDATA%\CrashDumps\`).
///
/// Returns `None` if the exit doesn't look like a crash (clean exit).
#[cfg(target_os = "linux")]
fn describe_coredump(child_pid: u32, status: &std::process::ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    let signal = status.signal()?;
    let signal_name = signal_name(signal);
    let mut out = format!("Killed by {signal_name} (signal {signal}).\n");
    if !status.core_dumped() {
        out.push_str(
            "No core dump generated (ulimit -c is likely 0 — `ulimit -c unlimited` to enable).\n",
        );
        return Some(out);
    }
    out.push_str("A core dump was generated.\n\n");

    let pattern = std::fs::read_to_string("/proc/sys/kernel/core_pattern")
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if pattern.starts_with('|') && pattern.contains("systemd-coredump") {
        out.push_str("Captured by systemd-coredump.\n");
        out.push_str(&format!(
            "  list:           coredumpctl list\n  open in gdb:    coredumpctl debug {child_pid}\n  raw file:       coredumpctl info {child_pid}\n  storage:        /var/lib/systemd/coredump/\n"
        ));
    } else if pattern.starts_with('|') {
        out.push_str(&format!("Core piped to handler: {pattern}\n"));
    } else if !pattern.is_empty() {
        out.push_str(&format!("Core file pattern: {pattern}\n"));
        out.push_str("(%p=PID, %e=exe-name, %t=epoch — see core(5))\n");
    } else {
        out.push_str("Core file location is unknown (empty core_pattern).\n");
    }
    out.push_str("\nHow to inspect a core file:\n");
    out.push_str("  https://wiki.archlinux.org/title/Core_dump\n");
    out.push_str("  https://www.freedesktop.org/software/systemd/man/coredumpctl.html\n");
    Some(out)
}

#[cfg(target_os = "macos")]
fn describe_coredump(child_pid: u32, status: &std::process::ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    let signal = status.signal()?;
    let signal_name = signal_name(signal);
    let mut out = format!("Killed by {signal_name} (signal {signal}).\n\n");
    let _ = child_pid;
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".into());
    out.push_str("macOS writes a crash report instead of a Unix-style core file.\n");
    out.push_str(&format!(
        "  per-user reports: {home}/Library/Logs/DiagnosticReports/\n"
    ));
    out.push_str("  system reports:   /Library/Logs/DiagnosticReports/\n");
    out.push_str("  view in GUI:      Console.app → Crash Reports\n");
    out.push_str("\nIf you actually need a core file (rare), enable it with:\n");
    out.push_str("  ulimit -c unlimited        (per-shell)\n");
    out.push_str("  sudo chmod 1777 /cores     (one-time, system-wide)\n");
    out.push_str("\nDocs:\n");
    out.push_str("  https://developer.apple.com/documentation/xcode/diagnosing-issues-using-crash-reports-and-device-logs\n");
    Some(out)
}

#[cfg(target_os = "windows")]
fn describe_coredump(child_pid: u32, status: &std::process::ExitStatus) -> Option<String> {
    let code = status.code()?;
    // Common NTSTATUS values that indicate a crash. Negative i32 ↔ 0x8…
    // / 0xC… NTSTATUS — interpret as u32 hex for clarity.
    let unsigned = code as u32;
    let label = match unsigned {
        0xC0000005 => Some("EXCEPTION_ACCESS_VIOLATION"),
        0xC000001D => Some("EXCEPTION_ILLEGAL_INSTRUCTION"),
        0xC0000094 => Some("EXCEPTION_INT_DIVIDE_BY_ZERO"),
        0xC00000FD => Some("EXCEPTION_STACK_OVERFLOW"),
        0xC0000409 => Some("STATUS_STACK_BUFFER_OVERRUN"),
        0xC0000374 => Some("STATUS_HEAP_CORRUPTION"),
        0xC000013A => Some("STATUS_CONTROL_C_EXIT"),
        _ => None,
    };
    // Anything matching the NTSTATUS severity bits 0xC… looks like a crash.
    let looks_like_crash = label.is_some() || unsigned >= 0xC000_0000;
    if !looks_like_crash {
        return None;
    }
    let mut out = match label {
        Some(name) => format!("Crashed with {name} (exit code 0x{unsigned:08X}).\n"),
        None => format!("Crashed with exit code 0x{unsigned:08X} (NTSTATUS-like).\n"),
    };
    out.push_str(&format!("PID was: {child_pid}\n\n"));
    out.push_str("Windows Error Reporting (if enabled) writes a minidump to:\n");
    out.push_str("  %LOCALAPPDATA%\\CrashDumps\\VPinballX_BGFX*.dmp\n\n");
    out.push_str("Enable user-mode minidumps if not already (one-off, run as admin):\n");
    out.push_str("  reg add \"HKLM\\SOFTWARE\\Microsoft\\Windows\\Windows Error Reporting\\LocalDumps\" /v DumpType /t REG_DWORD /d 2\n\n");
    out.push_str("Open the .dmp with WinDbg or Visual Studio.\n");
    out.push_str("\nDocs:\n");
    out.push_str(
        "  https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps\n",
    );
    out.push_str(
        "  https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/windbg-overview\n",
    );
    Some(out)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn describe_coredump(_child_pid: u32, _status: &std::process::ExitStatus) -> Option<String> {
    None
}

#[cfg(unix)]
fn signal_name(signal: i32) -> &'static str {
    match signal {
        4 => "SIGILL",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        15 => "SIGTERM",
        _ => "unknown",
    }
}

/// True when the exit looks like a crash (signal or NTSTATUS-style
/// error). Used to decide whether to surface an error popup even after
/// the user reached gameplay — a mid-game crash should never be silent.
fn is_abnormal_exit(status: &std::process::ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal().is_some() {
            return true;
        }
    }
    #[cfg(target_os = "windows")]
    {
        match status.code() {
            Some(c) => return (c as u32) >= 0xC000_0000,
            None => return true, // no code = abnormal termination
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = status;
        false
    }
}

/// POSIX-shell quote a path or argument: wrap in single quotes and
/// escape embedded `'` as `'\''`. Result is a string the user can paste
/// into a shell to re-run the exact same command.
fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | ':' | '='))
    {
        // Safe-looking — no quoting needed, keeps the line readable.
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn parse_progress_pct(msg: &str) -> Option<f32> {
    // Look for a number followed by '%'
    let pct_pos = msg.find('%')?;
    let before = &msg[..pct_pos];
    // Walk backwards to find the start of the number
    let num_start = before
        .rfind(|c: char| !c.is_ascii_digit() && c != '.')
        .map(|p| p + 1)
        .unwrap_or(0);
    let num_str = &before[num_start..];
    let pct: f32 = num_str.parse().ok()?;
    Some((pct / 100.0).clamp(0.0, 1.0))
}

impl App {
    pub(super) fn finalize_wizard(&mut self, _ctx: &egui::Context) {
        // Save ALL pages
        self.save_screens();
        self.save_rendering();
        self.save_inputs();
        self.save_tilt();
        self.save_audio();
        self.save_tables_dir();
        self.flush_config();

        if let Err(e) = self.db.set_configured() {
            log::error!("Failed to mark wizard complete: {e}");
        }

        // Apply autostart setting
        if let Err(e) = set_autostart(self.autostart) {
            log::error!("Failed to set autostart: {e}");
        }

        // Apply desktop integration (menu shortcuts + .vpx file association).
        // Pass the resolved VPX path so the .vpx handler points to the right
        // binary; if empty, only PinReady's own shortcut is installed.
        if let Err(e) = set_desktop_integration(self.desktop_integration, &self.vpx_exe_path) {
            log::error!("Failed to set desktop integration: {e}");
        }

        // Knocker surprise — compute its exact playback duration from the
        // decoded PCM so the close deadline matches the real end of the
        // sound (not an arbitrary 800ms timeout).
        let knocker_path = "knocker.ogg";
        let knocker_duration =
            audio::asset_duration(knocker_path).unwrap_or(std::time::Duration::from_millis(300));
        if let Some(tx) = &self.audio_cmd_tx {
            let _ = tx.send(AudioCommand::PlayOnSpeaker {
                path: knocker_path.to_string(),
                target: audio::SpeakerTarget::FrontBoth,
            });
        }

        log::info!(
            "Wizard completed! Config saved; closing eframe in {:?} to let the knocker play out.",
            knocker_duration
        );

        // Signal main.rs that after this eframe exits, relaunch in Launcher
        // mode. The actual Close fires from the `close_at` tick in App::ui.
        // Add a tiny post-roll (50ms) to cover SDL buffering latency.
        crate::app::request_mode_switch(AppMode::Launcher);
        self.close_at = Some(
            std::time::Instant::now() + knocker_duration + std::time::Duration::from_millis(50),
        );
    }

    // Previous versions of this file had `enter_cabinet_mode_if_configured`
    // and `leave_cabinet_mode_live` that mutated the live viewport (rotation,
    // monitor, decorations) between wizard and launcher modes. Those were
    // removed in favour of the restart-eframe-per-mode model driven by
    // `request_mode_switch` + `main.rs` loop: each mode now comes up with
    // its viewport correctly configured at window-creation time, avoiding
    // the dual-render / stale-compositor glitches.

    pub(super) fn scan_tables(&mut self) {
        // Bump the scan generation BEFORE clearing — any in-flight bg
        // thread from a prior scan will continue running and may still
        // emit results, but their (gen, idx) tuples will fail the gen
        // check in `process_bg_extraction` and be discarded. This
        // prevents stale extractions from writing thumbnails onto the
        // wrong rows after a rescan that reshuffled the index space.
        self.scan_generation = self.scan_generation.wrapping_add(1);
        // Drop the prior receiver so the prior thread's `tx.send`
        // becomes a no-op (channel closed) — a small CPU optimisation
        // on top of the gen-check belt-and-suspenders above.
        self.bg_rx = None;
        // Force `preload_images_once` to re-register URIs for the
        // freshly-scanned table set against the new generation.
        self.images_preloaded = false;
        self.tables.clear();
        // Forget per-row image cache entries we may have populated for
        // the previous scan: row 7 used to be Apollo 13, after rescan
        // it might be Avatar, but egui still has `bytes://bg/7` cached
        // pointing at the Apollo 13 JPEG. Clearing the loaders here
        // would be cheaper than a full asset reload, but egui's
        // ImageButton reads via `image()` which respects the include
        // map, so flushing per-uri caches is enough — see
        // `process_bg_extraction` where we re-include with the new
        // generation.
        if self.tables_dir.is_empty() {
            return;
        }
        // Own the path so we can mix immutable reads of `dir_path`
        // with `&mut self` later (scan_vbs_patches needs `&mut self`).
        let dir: String = self.tables_dir.clone();
        let dir_path: std::path::PathBuf = std::path::PathBuf::from(&dir);
        if !dir_path.is_dir() {
            log::warn!("Tables directory does not exist: {}", dir);
            return;
        }
        let dir_path = dir_path.as_path();
        // Scan for .vpx files (folder-per-table layout: each subfolder has a .vpx).
        // Phase 1: collect raw (table_dir, vpx_path, rel_path, source_mtime).
        let mut found: Vec<(std::path::PathBuf, std::path::PathBuf, String, i64)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let table_dir = entry.path();
                if !table_dir.is_dir() {
                    continue;
                }
                if let Ok(files) = std::fs::read_dir(&table_dir) {
                    for file in files.flatten() {
                        let fp = file.path();
                        if fp.extension().and_then(|e| e.to_str()) == Some("vpx") {
                            let rel_path = fp
                                .strip_prefix(dir_path)
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_else(|_| fp.to_string_lossy().into_owned());
                            let source_mtime = max_source_mtime(&table_dir, &fp);
                            found.push((table_dir.clone(), fp, rel_path, source_mtime));
                            break; // one vpx per folder
                        }
                    }
                }
            }
        }

        // Phase 2: build TableEntry list + extraction jobs in a single
        // pass. The jobs reference the final (post-sort) indices so the
        // extraction thread can write to the right row.
        for (table_dir, vpx_path, _, _) in &found {
            let name = table_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .replace('_', " ");
            self.tables.push(TableEntry {
                path: vpx_path.clone(),
                name,
                bg_bytes: None,
                update_available: false,
                vps_id: None,
            });
        }
        self.tables.sort_by_key(|a| a.name.to_lowercase());

        let mut jobs: Vec<(usize, std::path::PathBuf, std::path::PathBuf, i64)> = Vec::new();
        for (table_dir, vpx_path, rel_path, source_mtime) in found {
            let idx = match self.tables.iter().position(|t| t.path == vpx_path) {
                Some(i) => i,
                None => continue,
            };
            self.tables[idx].update_available = self.db.get_update_available(&rel_path);
            self.tables[idx].vps_id = self.db.get_vps_link(&rel_path).map(|l| l.0);
            match self.db.get_backglass(&rel_path) {
                Some((bytes, cached_mtime)) if cached_mtime >= source_mtime => {
                    self.tables[idx].bg_bytes =
                        Some(std::sync::Arc::from(bytes.into_boxed_slice()));
                }
                _ => jobs.push((idx, table_dir, vpx_path, source_mtime)),
            }
        }
        log::info!("Scanned {} tables in {}", self.tables.len(), dir);

        // Cancel any prior scan so the new pool is the only one writing
        // to `medias/` and the DB.
        if let Some(prev) = self.catalog_cancel_token.take() {
            prev.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.catalog_cancel_token = Some(cancel.clone());

        // Build the per-table scan jobs. Each job is a self-contained
        // pipeline: VPSDB match → media install → backglass extract →
        // DB write → UI signal. Workers run them in parallel, sequential
        // within each job — so the same worker that may have just
        // installed `medias/bg.png` is the one that reads it back in
        // the priority chain. No cross-thread file race.
        let scan_jobs: Vec<crate::scan_worker::ScanJob> = jobs
            .into_iter()
            .map(|(idx, table_dir, vpx_path, source_mtime)| {
                let rel_path = vpx_path
                    .strip_prefix(dir_path)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| vpx_path.to_string_lossy().into_owned());
                let folder_name = table_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                crate::scan_worker::ScanJob {
                    idx,
                    rel_path,
                    table_dir,
                    vpx_path,
                    folder_name,
                    source_mtime,
                }
            })
            .collect();

        let (tx, rx) = crossbeam_channel::unbounded();
        self.bg_rx = Some(rx);

        if scan_jobs.is_empty() {
            return;
        }

        let tables_root = dir_path.to_path_buf();
        let gen = self.scan_generation;
        let enrichment_on = self.db.catalog_enrichment_enabled();

        // Sync VPSDB + MediaDb on a small bootstrap thread so we don't
        // block the UI; once both indices are loaded we hand them
        // (Arc-shared) to the worker pool.
        std::thread::Builder::new()
            .name(format!("pinready-scan-bootstrap-{gen}"))
            .spawn(move || {
                use crate::vpsdb;
                use std::sync::Arc;

                let db = match crate::db::Database::open(None) {
                    Ok(db) => db,
                    Err(e) => {
                        log::error!("scan bootstrap: cannot open DB: {e}");
                        return;
                    }
                };
                let mirror = db.mirror_base_url();

                let games: Arc<Vec<vpsdb::models::Game>> = if enrichment_on {
                    let cache = vpsdb::fetch::VpsDbCache::new(vpsdb::fetch::VpsDbCache::default_dir());
                    match vpsdb::fetch::sync_if_stale(&cache) {
                        Ok((games, _outcome)) => Arc::new(games),
                        Err(e) => {
                            log::warn!("scan bootstrap: VPSDB sync failed ({e}) — match-only");
                            Arc::new(Vec::new())
                        }
                    }
                } else {
                    Arc::new(Vec::new())
                };

                let media_db: Option<Arc<crate::mediadb::MediaDb>> = if enrichment_on {
                    match crate::mediadb::MediaDb::sync(
                        crate::mediadb::MediaDb::default_cache_dir(),
                        mirror.as_deref(),
                    ) {
                        Ok(m) => Some(Arc::new(m)),
                        Err(e) => {
                            log::warn!("scan bootstrap: MediaDb sync failed ({e}) — match-only");
                            None
                        }
                    }
                } else {
                    None
                };

                // matcher_version upgrade: bumped when the matcher chain
                // changes (new strategy / confidence shift). Forces a
                // one-shot full re-evaluation of every link this run.
                let stored_matcher_version: i64 = db
                    .get_config("matcher_version")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let force_full_rematch = stored_matcher_version < crate::scan_worker::MATCHER_VERSION;
                if force_full_rematch {
                    log::info!(
                        "Matcher upgraded ({stored_matcher_version} → {}); re-evaluating every vps_link this run",
                        crate::scan_worker::MATCHER_VERSION
                    );
                }

                crate::scan_worker::spawn_pool(
                    scan_jobs,
                    games,
                    media_db,
                    tx,
                    cancel,
                    tables_root,
                    gen,
                    force_full_rematch,
                );

                if force_full_rematch {
                    let _ = db.set_config(
                        "matcher_version",
                        &crate::scan_worker::MATCHER_VERSION.to_string(),
                    );
                }
            })
            .ok();

        // VBS patch pipeline runs independently — separate mtime
        // tracking (sidecar + .vpx only), separate DB table.
        self.scan_vbs_patches(dir_path);
    }

    /// Classify each table's VBS state and apply patches from the
    /// jsm174 catalog when appropriate. Runs the network fetch +
    /// classification + file ops on a background thread; the UI gets
    /// results via `vbs_rx` and folds them into the `vbs_patches`
    /// table in `process_vbs_extraction`.
    fn scan_vbs_patches(&mut self, dir_path: &std::path::Path) {
        // Opt-in: user has to enable auto-patching explicitly from the
        // Tables wizard page. Default is off because the jsm174 catalog
        // occasionally ships patches with regressions (e.g. Apollo 13
        // needs an additional `vpmInit Me` fix on top of their patch —
        // see vpinball/vpinball#1536, #1650).
        if !self.db.jsm174_patching_enabled() {
            log::debug!("vbs_patches: jsm174 auto-patching is disabled — skipping");
            return;
        }

        // Refresh the jsm174 catalog if upstream master has moved.
        // Non-fatal on network error — falls back to cached catalog.
        if let Err(e) = crate::vbs_patches::refresh_catalog_if_stale(&self.db) {
            log::warn!("vbs_patches: catalog refresh failed: {e}");
        }
        let catalog: Vec<crate::vbs_patches::CatalogEntry> = self
            .db
            .get_vbs_catalog()
            .and_then(|(_, json)| crate::vbs_patches::parse_catalog(&json).ok())
            .unwrap_or_default();
        if catalog.is_empty() {
            log::info!("vbs_patches: no catalog available yet (first boot offline?). Skipping.");
            return;
        }

        // Collect jobs for stale / unclassified tables.
        let mut jobs: Vec<(std::path::PathBuf, String, i64)> = Vec::new();
        for table in &self.tables {
            let rel_path = table
                .path
                .strip_prefix(dir_path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| table.path.to_string_lossy().into_owned());
            let vbs_mtime = max_vbs_mtime(&table.path);
            match self.db.get_vbs_patch(&rel_path) {
                Some((_, _, _, cached_mtime)) if cached_mtime >= vbs_mtime => {
                    // Fresh classification — nothing to do.
                }
                _ => jobs.push((table.path.clone(), rel_path, vbs_mtime)),
            }
        }
        if jobs.is_empty() {
            return;
        }
        log::info!(
            "vbs_patches: classifying {} tables in background...",
            jobs.len()
        );

        let (tx, rx) = crossbeam_channel::unbounded();
        std::thread::spawn(move || {
            for (vpx_path, rel_path, mtime) in jobs {
                match crate::vbs_patches::classify(&vpx_path, &catalog) {
                    Ok(classification) => {
                        let decision_status =
                            crate::vbs_patches::decision_status(&classification.decision);
                        // Apply side-effects (download + install). A
                        // failure here flips the recorded status to
                        // Failed so the next scan will retry.
                        let status = match crate::vbs_patches::apply_patch(
                            &vpx_path,
                            &classification.decision,
                        ) {
                            Ok(()) => decision_status.to_string(),
                            Err(e) => {
                                log::warn!("vbs_patches: apply failed for {}: {e}", rel_path);
                                crate::vbs_patches::status::FAILED.to_string()
                            }
                        };
                        log::info!("vbs_patches: {} → {}", rel_path, status);
                        let _ = tx.send((
                            rel_path,
                            classification.embedded_sha,
                            classification.sidecar_sha,
                            status,
                            mtime,
                        ));
                    }
                    Err(e) => {
                        log::warn!("vbs_patches: classify failed for {}: {e}", rel_path);
                        let _ = tx.send((
                            rel_path,
                            String::new(),
                            None,
                            crate::vbs_patches::status::FAILED.to_string(),
                            mtime,
                        ));
                    }
                }
            }
            log::info!("vbs_patches: classification run complete");
        });
        self.vbs_rx = Some(rx);
    }

    /// Nuke PinReady's entire SDL3 footprint before spawning VPX.
    /// Drop the audio sender + flip the joystick running flag, join
    /// both worker threads (guarantees nobody is mid-call into SDL3),
    /// then call `SDL_Quit()` to slam every subsystem + open device
    /// down in one go. After this PinReady's process holds zero SDL3
    /// state — VPX spawns into a fresh SDL3 universe.
    pub(super) fn shutdown_sdl_threads(&mut self) {
        self.audio_cmd_tx = None;
        if let Some(handle) = self.audio_thread.take() {
            let _ = handle.join();
        }

        if let Some(running) = self.joystick_running.take() {
            running.store(false, Ordering::Relaxed);
        }
        self.joystick_rx = None;
        if let Some(handle) = self.joystick_thread.take() {
            let _ = handle.join();
        }

        unsafe {
            sdl3_sys::everything::SDL_Quit();
        }

        // Confirm SDL has fully wound down before we hand off to VPX.
        // `SDL_WasInit(0)` returns the bitmask of currently-initialized
        // subsystems and should hit zero immediately after `SDL_Quit`
        // (https://wiki.libsdl.org/SDL3/SDL_WasInit). On Linux some
        // teardown is observably async (PipeWire audio session
        // retention, joystick hotplug worker) so we poll for up to
        // 300 ms just to be sure.
        let poll_deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        let zero = sdl3_sys::init::SDL_InitFlags(0);
        let mut residual = unsafe { sdl3_sys::everything::SDL_WasInit(zero) };
        while residual.0 != 0 && std::time::Instant::now() < poll_deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
            residual = unsafe { sdl3_sys::everything::SDL_WasInit(zero) };
        }
        if residual.0 != 0 {
            log::warn!(
                "SDL_WasInit still reports {:#x} 300 ms after SDL_Quit — proceeding anyway",
                residual.0
            );
        } else {
            log::info!("SDL_Quit() complete — all subsystems fully released");
        }
    }

    /// Re-spawn audio + joystick threads after VPX exits. New SDL3
    /// subsystem inits happen inside each thread, so the launcher is
    /// fully responsive again as soon as this returns.
    pub(super) fn respawn_sdl_threads(&mut self) {
        let (rx, running, handle) = crate::inputs::spawn_joystick_thread();
        self.joystick_rx = Some(rx);
        self.joystick_running = Some(running);
        self.joystick_thread = Some(handle);

        let (tx, handle) = crate::audio::spawn_audio_thread();
        self.audio_cmd_tx = Some(tx);
        self.audio_thread = Some(handle);
    }

    /// Entry point for launching a table from any UI path (click, Enter,
    /// joystick). On Wayland this is a two-step launch: request a fresh
    /// xdg-activation-v1 token from the compositor, then spawn VPX on a
    /// later frame once winit delivers it via
    /// `Event::ActivationTokenReceived` — without a serial-sealed token,
    /// mutter refuses to grant focus and the table opens behind PinReady.
    /// A 500 ms deadline in `App::ui` falls back to launching without a
    /// token if the compositor never replies.
    ///
    /// Everywhere else (macOS, Windows, X11, headless) the token dance is
    /// pointless — `RequestActivationToken` would be a no-op or produce an
    /// unused X11 startup id, and every launch would eat the full 500 ms
    /// deadline — so we spawn immediately, exactly like pre-0.14.1.
    pub(super) fn begin_table_launch(&mut self, path: std::path::PathBuf, ctx: &egui::Context) {
        if crate::session::detect() == Some("wayland") {
            self.pending_vpx_launch = Some((path, std::time::Instant::now()));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestActivationToken);
        } else {
            self.launch_table(&path, None);
        }
    }

    pub(super) fn launch_table(
        &mut self,
        table_path: &std::path::Path,
        activation_token: Option<String>,
    ) {
        if self.vpx_running.load(Ordering::Relaxed) {
            return;
        }
        // Preview audio stops automatically when we tear down the audio
        // thread below — no explicit PreviewStop needed.
        self.preview_playing = false;
        self.preview_due_at = None;
        let resolved = updater::resolve_vpx_exe(std::path::Path::new(&self.vpx_exe_path));
        if self.vpx_exe_path.is_empty() || !resolved.is_file() {
            log::error!("Visual Pinball executable not found: {}", self.vpx_exe_path);
            return;
        }
        // Release every SDL3 subsystem PinReady is holding (audio
        // device + open joystick handles + their respective subsystem
        // counters) so VPX can claim them cleanly. They'll be re-spawned
        // when VPX exits — see `process_vpx_status`.
        self.shutdown_sdl_threads();
        log::info!(
            "Launching: {} -Play {}",
            resolved.display(),
            table_path.display()
        );
        let exe = resolved.display().to_string();
        let path = table_path.to_path_buf();
        let running = self.vpx_running.clone();
        running.store(true, Ordering::Relaxed);
        self.vpx_loading_msg = t!("launcher_loading").to_string();
        self.vpx_error_log = None;

        let (tx, rx) = crossbeam_channel::unbounded();
        self.vpx_status_rx = Some(rx);

        std::thread::spawn(move || {
            use std::io::BufRead;
            // Reproducible call header: same shell line a user could run
            // by hand, plus the cwd we spawned from and the host system
            // summary. Prepended to every error string so the popup
            // carries enough context to file a bug without going back to
            // re-derive paths or `uname -a` info.
            let call_header = || -> String {
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "?".into());
                let sys = crate::system_info::detect().one_liner();
                format!(
                    "$ {} -Play {}\n  cwd:    {}\n  system: {}\n  client: PinReady v{}\n\n",
                    shell_quote(&exe),
                    shell_quote(&path.display().to_string()),
                    cwd,
                    sys,
                    crate::VERSION,
                )
            };
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg("-Play")
                .arg(&path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            // Pin VPX's SDL backend to PinReady's actual session
            // (Linux only — `detect()` is None on macOS/Windows so this
            // is a no-op there). Newer SDL3 auto-selects XWayland when
            // it sees both DISPLAY and WAYLAND_DISPLAY, which brings
            // back the exact display-placement bugs PinReady exists to
            // avoid on the Wayland side. Forcing the driver name here
            // overrides any inherited or auto-detected value.
            if let Some(driver) = crate::session::detect() {
                log::info!("Pinning VPX's SDL_VIDEODRIVER to {driver}");
                cmd.env("SDL_VIDEODRIVER", driver);
            }
            // Inject the compositor-issued xdg-activation-v1 token into
            // VPX's env so its SDL3 window can grab focus. The token was
            // requested from the compositor via
            // `ViewportCommand::RequestActivationToken` at click time and
            // delivered by winit through `Event::ActivationTokenReceived`.
            // A missing token means either X11/non-Wayland (harmless — the
            // env var is Wayland-specific) or the compositor didn't reply
            // in time — VPX may open behind PinReady in that case.
            if let Some(token) = activation_token {
                log::info!(
                    "xdg-activation token obtained (len {}), passing to VPX",
                    token.len()
                );
                cmd.env("XDG_ACTIVATION_TOKEN", token);
            } else {
                log::debug!(
                    "no xdg-activation token available; VPX may launch behind PinReady on Wayland"
                );
            }
            let child = cmd.spawn();
            match child {
                Ok(mut child) => {
                    log::info!("Visual Pinball launched, reading stdout+stderr...");

                    // Capture stderr on a separate thread
                    let stderr_handle = child.stderr.take().map(|se| {
                        std::thread::spawn(move || {
                            let reader = std::io::BufReader::new(se);
                            let mut lines = Vec::new();
                            for line in reader.lines().map_while(Result::ok) {
                                log::warn!("[VPX stderr] {}", line);
                                lines.push(line);
                            }
                            lines
                        })
                    });

                    let stdout = child.stdout.take();
                    // Two-tier log buffer: the loading phase is kept in
                    // full (header + every line up to "Startup done"),
                    // then post-startup lines flow into a ring of the
                    // last 100. This gives the user always-meaningful
                    // diagnostics in the error popup — even if a table
                    // crashes mid-game (which used to be silent because
                    // `startup_done` short-circuited to ExitOk).
                    const INGAME_TAIL: usize = 100;
                    let mut loading_log: Vec<String> = Vec::new();
                    let mut ingame_log: std::collections::VecDeque<String> =
                        std::collections::VecDeque::with_capacity(INGAME_TAIL);
                    let mut startup_done = false;
                    // Build the full log we hand to ExitError. Always
                    // contains the call header and every loading-phase
                    // line; if startup_done was reached, also a visible
                    // separator and the in-game tail.
                    let build_error_log = |reason: &str,
                                           loading: &[String],
                                           ingame: &std::collections::VecDeque<String>|
                     -> String {
                        let mut out = call_header();
                        if !reason.is_empty() {
                            out.push_str(reason);
                            out.push_str("\n\n");
                        }
                        if !loading.is_empty() {
                            out.push_str("----- loading -----\n");
                            out.push_str(&loading.join("\n"));
                            out.push('\n');
                        }
                        if !ingame.is_empty() {
                            out.push_str("\n----- in-game (last ");
                            out.push_str(&ingame.len().to_string());
                            out.push_str(" lines) -----\n");
                            for l in ingame {
                                out.push_str(l);
                                out.push('\n');
                            }
                        }
                        out
                    };

                    if let Some(so) = stdout {
                        let reader = std::io::BufReader::new(so);
                        let timeout = std::time::Duration::from_secs(30);
                        let (line_tx, line_rx) = crossbeam_channel::unbounded();

                        // Read stdout lines on a helper thread to allow timeout
                        std::thread::spawn(move || {
                            for line in reader.lines().map_while(Result::ok) {
                                if line_tx.send(line).is_err() {
                                    break;
                                }
                            }
                        });

                        loop {
                            match line_rx.recv_timeout(timeout) {
                                Ok(line) => {
                                    log::info!("[VPX] {}", line);
                                    if line.contains("SetProgress") {
                                        if let Some(start) = line.find("] ") {
                                            let msg = &line[start + 2..];
                                            let pct = parse_progress_pct(msg);
                                            let _ =
                                                tx.send(VpxStatus::Loading(msg.to_string(), pct));
                                        }
                                    } else if line.contains("RenderStaticPrepass")
                                        && line.contains("Reflection Probe")
                                    {
                                        let _ = tx.send(VpxStatus::Loading(
                                            "Reflection Probe...".to_string(),
                                            None,
                                        ));
                                    } else if line.contains("PluginLog") {
                                        if let Some(start) = line.rfind("] ") {
                                            let msg = &line[start + 2..];
                                            if let Some(colon) = msg.find(':') {
                                                let plugin = &msg[..colon];
                                                let _ = tx.send(VpxStatus::Loading(
                                                    format!("Plugin {plugin}..."),
                                                    None,
                                                ));
                                            }
                                        }
                                    } else if line.contains("Startup done") {
                                        startup_done = true;
                                        loading_log.push(line);
                                        let _ = tx.send(VpxStatus::Started);
                                        continue;
                                    }
                                    if !startup_done {
                                        loading_log.push(line);
                                    } else {
                                        if ingame_log.len() == INGAME_TAIL {
                                            ingame_log.pop_front();
                                        }
                                        ingame_log.push_back(line);
                                    }
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                                    if startup_done {
                                        // After startup, silence is normal (in-game VPX
                                        // logs sparsely). We must keep draining stdout —
                                        // dropping `line_rx` would close the read side of
                                        // the pipe, and VPX's next write triggers SIGPIPE
                                        // and kills the game mid-play. Wait for VPX to
                                        // close stdout naturally (→ Disconnected).
                                        continue;
                                    }
                                    log::error!(
                                        "VPX stdout timeout (30s without output during loading)"
                                    );
                                    let _ = child.kill();
                                    let err = build_error_log(
                                        "Timeout: Visual Pinball stopped responding during loading (no output for 30s).",
                                        &loading_log,
                                        &ingame_log,
                                    );
                                    let _ = tx.send(VpxStatus::ExitError(err));
                                    running.store(false, Ordering::Relaxed);
                                    return;
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                                    // stdout closed — process is exiting
                                    break;
                                }
                            }
                        }
                    }

                    // Collect stderr
                    let stderr_lines = stderr_handle
                        .and_then(|h| h.join().ok())
                        .unwrap_or_default();

                    let child_pid = child.id();
                    match child.wait() {
                        Ok(status) => {
                            log::info!("Visual Pinball exited with status: {status}");
                            // Decide whether to show the error popup. We
                            // diverge from "any non-zero after startup is
                            // OK": a table that closes mid-game without a
                            // popup is exactly the frustrating thing the
                            // user wants to avoid. Silent only when:
                            //   - exit code 0, OR
                            //   - exited cleanly (no signal, no NTSTATUS
                            //     crash) AND the user reached gameplay.
                            let abnormal = is_abnormal_exit(&status);
                            if status.success() || (startup_done && !abnormal) {
                                let _ = tx.send(VpxStatus::ExitOk);
                            } else {
                                let mut reason =
                                    format!("Visual Pinball exited with status: {status}");
                                if let Some(desc) = describe_coredump(child_pid, &status) {
                                    reason.push_str("\n\n");
                                    reason.push_str(&desc);
                                }
                                let mut combined =
                                    build_error_log(&reason, &loading_log, &ingame_log);
                                if !stderr_lines.is_empty() {
                                    combined.push_str("\n----- stderr -----\n");
                                    combined.push_str(&stderr_lines.join("\n"));
                                    combined.push('\n');
                                }
                                let _ = tx.send(VpxStatus::ExitError(combined));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to wait for Visual Pinball: {e}");
                            let combined = build_error_log(
                                &format!("Process error: {e}"),
                                &loading_log,
                                &ingame_log,
                            );
                            let _ = tx.send(VpxStatus::ExitError(combined));
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to launch Visual Pinball: {e}");
                    let _ = tx.send(VpxStatus::LaunchError(format!(
                        "{}Failed to launch: {e}",
                        call_header()
                    )));
                }
            }
            running.store(false, Ordering::Relaxed);
        });
    }

    /// Drive the per-table audio preview: when `selected_table` changes
    /// we stop any current preview, schedule a debounced PreviewStart, and
    /// fire it once the deadline passes. VPX-running suspends previews so
    /// the table soundtrack doesn't double up with our jingle.
    pub(super) fn process_preview_audio(&mut self, ctx: &egui::Context) {
        if self.tables.is_empty() {
            return;
        }
        let vpx_running = self.vpx_running.load(Ordering::Relaxed);
        if vpx_running {
            if self.preview_playing {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PreviewStop);
                }
                self.preview_playing = false;
            }
            self.preview_last_idx = None;
            self.preview_due_at = None;
            return;
        }

        let cur = self.selected_table;
        if Some(cur) != self.preview_last_idx {
            // Selection changed — stop current, debounce next start.
            if self.preview_playing {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PreviewStop);
                }
                self.preview_playing = false;
            }
            self.preview_last_idx = Some(cur);
            self.preview_due_at =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(700));
        }

        if let Some(due) = self.preview_due_at {
            if std::time::Instant::now() >= due {
                self.preview_due_at = None;
                if let Some(table) = self.tables.get(cur) {
                    if let Some(table_dir) = table.path.parent() {
                        let audio_path = table_dir.join("medias").join("audio.mp3");
                        if audio_path.is_file() {
                            if let Some(tx) = &self.audio_cmd_tx {
                                // Preview clips are halved so they sit
                                // below the in-game soundtrack baseline —
                                // hovering over a card shouldn't be louder
                                // than the table the user is browsing for.
                                let volume =
                                    (self.audio.music_volume as f32 / 100.0 * 0.5).clamp(0.0, 1.0);
                                let _ = tx.send(AudioCommand::PreviewStart {
                                    path: audio_path,
                                    volume,
                                });
                                self.preview_playing = true;
                            }
                        }
                    }
                }
            } else {
                ctx.request_repaint_after(due - std::time::Instant::now());
            }
        }
    }

    pub(super) fn process_bg_extraction(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.bg_rx {
            // Drain without holding a borrow of `self` — we need `&mut self`
            // below for `self.db.set_backglass` and the TableEntry update.
            let drained: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            let disconnected = matches!(
                rx.try_recv(),
                Err(crossbeam_channel::TryRecvError::Disconnected)
            );
            if disconnected {
                log::info!("Background backglass extraction channel closed");
                self.bg_rx = None;
            }
            for (msg_gen, idx, rel_path, bytes, source_mtime) in drained {
                // Drop messages from a prior scan whose index space
                // no longer matches `self.tables` (the user may have
                // hit Rescan while this thread was still extracting).
                if msg_gen != self.scan_generation {
                    log::debug!(
                        "Dropping stale BG result gen={msg_gen} (current={}) for {rel_path}",
                        self.scan_generation
                    );
                    continue;
                }
                // Belt-and-suspenders: the row that was at `idx` in the
                // bg thread's snapshot may be a *different* table now
                // (sort order can shift between scans). Look up the
                // current row by path and trust that over the index.
                let cur_idx = self
                    .tables
                    .iter()
                    .position(|t| {
                        t.path
                            .strip_prefix(&self.tables_dir)
                            .map(|p| p.to_string_lossy() == *rel_path)
                            .unwrap_or(false)
                    })
                    .or(if idx < self.tables.len() {
                        Some(idx)
                    } else {
                        None
                    });
                let Some(idx) = cur_idx else {
                    continue;
                };
                if let Err(e) = self.db.set_backglass(&rel_path, &bytes, source_mtime) {
                    log::error!("Failed to cache backglass for {rel_path}: {e}");
                }
                let arc: std::sync::Arc<[u8]> = std::sync::Arc::from(bytes.into_boxed_slice());
                // Generation-tagged URI: even if egui's image cache
                // still holds `bytes://bg/N` from a prior scan, the new
                // URI guarantees a fresh fetch on the new row.
                let uri = format!("bytes://bg/{}/{idx}", self.scan_generation);
                ctx.include_bytes(uri, arc.clone());
                self.tables[idx].bg_bytes = Some(arc);
                log::debug!("BG cached for table {idx} ({rel_path})");
            }
        }
    }

    /// Drain VBS-patch classification results and persist them in
    /// `vbs_patches`. No UI side-effects — patching is silent by design
    /// (user validates via log + `.pre_standalone.vbs` files appearing).
    pub(super) fn process_vbs_extraction(&mut self) {
        if let Some(rx) = &self.vbs_rx {
            let drained: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            let disconnected = matches!(
                rx.try_recv(),
                Err(crossbeam_channel::TryRecvError::Disconnected)
            );
            if disconnected {
                log::info!("vbs_patches: channel closed");
                self.vbs_rx = None;
            }
            for (rel_path, embedded_sha, sidecar_sha, status, mtime) in drained {
                if let Err(e) = self.db.set_vbs_patch(
                    &rel_path,
                    &embedded_sha,
                    sidecar_sha.as_deref(),
                    &status,
                    mtime,
                ) {
                    log::error!("Failed to upsert vbs_patches row for {rel_path}: {e}");
                }
            }
        }
    }

    pub(super) fn preload_images_once(&mut self, ctx: &egui::Context) {
        if self.images_preloaded {
            return;
        }
        self.images_preloaded = true;
        let mut count = 0;
        for (idx, table) in self.tables.iter().enumerate() {
            if let Some(ref arc) = table.bg_bytes {
                // Generation-tagged: post-rescan, idx might point at a
                // different table than before. The new gen forces egui
                // to refetch and we never reuse a stale `bytes://bg/N`
                // entry that was registered against the prior scan.
                let uri = format!("bytes://bg/{}/{idx}", self.scan_generation);
                ctx.include_bytes(uri, arc.clone());
                count += 1;
            }
        }
        if count > 0 {
            log::info!("Preloaded {count} cached images into RAM");
        }
    }

    /// Find launcher navigation action for a button.
    /// Only matches LeftFlipper, RightFlipper, LeftMagna, RightMagna, Start,
    /// LaunchBall, ExitGame — ignores StagedFlipper and other actions to avoid
    /// conflicts when flipper and staged are on the same physical button.
    fn action_for_launcher_nav(&self, button: u8) -> Option<String> {
        const NAV_ACTIONS: &[&str] = &[
            "LeftFlipper",
            "RightFlipper",
            "LeftMagna",
            "RightMagna",
            "Start",
            "LaunchBall",
            "ExitGame",
        ];
        for action in &self.actions {
            if !NAV_ACTIONS.contains(&action.setting_id) {
                continue;
            }
            if let Some(inputs::CapturedInput::JoystickButton { button: b, .. }) = &action.mapping {
                if *b == button {
                    return Some(action.setting_id.to_string());
                }
            }
        }
        None
    }

    /// Send `Close` to every cover viewport we may have spawned
    /// (BG/DMD/Topper). Must run *before* closing the root viewport
    /// when exiting the launcher: if the root dies first eframe
    /// theoretically tears the rest down in cascade, but on
    /// Wayland/Mutter this leaves the cover windows behind as
    /// compositor ghosts. Closing them ourselves makes the
    /// destruction order deterministic. Sending Close to a viewport
    /// that doesn't exist is a no-op, so addressing all three
    /// unconditionally is safe.
    pub(super) fn close_cover_viewports(ctx: &egui::Context) {
        for cover_id in [
            crate::app::BG_VIEWPORT,
            crate::app::PF_VIEWPORT,
            crate::app::TOPPER_VIEWPORT,
        ] {
            let viewport_id = egui::ViewportId::from_hash_of(cover_id);
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
        }
    }

    /// Unified exit: release cursor capture (otherwise the OS cursor stays
    /// hidden while the window tears down), close every cover viewport
    /// (BG/DMD/Topper) explicitly, then request the root viewport close.
    /// Called from the Quit button, ExitGame joystick action, and Escape key.
    pub(super) fn quit_launcher(&mut self, ctx: &egui::Context) {
        // Releasing the capture is enough since egui-rotate 1.1: the plugin
        // drops its OS grab and stops hiding the pointer on that transition.
        Self::with_software_cursor(ctx, |c| c.release());
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));

        Self::close_cover_viewports(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Indices into `self.tables` of the currently-visible cards
    /// (matches the grid the user actually sees). Empty filter →
    /// every table; non-empty → tables whose name contains the
    /// lowercased filter.
    pub(super) fn visible_indices(&self) -> Vec<usize> {
        if self.table_filter_lower.is_empty() {
            return (0..self.tables.len()).collect();
        }
        let f = self.table_filter_lower.as_str();
        (0..self.tables.len())
            .filter(|&i| self.tables[i].name.to_lowercase().contains(f))
            .collect()
    }

    /// Dispatch a launcher action. Navigation actions loop over the
    /// currently-visible (filtered) tables only; Launch and Cancel
    /// drive table launch and search-clear/quit respectively. Returns
    /// `true` when a directional action moved the selection — used by
    /// the joystick repeat scheduler to decide whether to keep firing.
    pub(super) fn apply_launcher_action(
        &mut self,
        action: launcher_input::LauncherAction,
        ctx: &egui::Context,
    ) -> bool {
        use launcher_input::LauncherAction;
        match action {
            LauncherAction::PrevCard
            | LauncherAction::NextCard
            | LauncherAction::PrevRow
            | LauncherAction::NextRow => {
                let visible = self.visible_indices();
                if visible.is_empty() {
                    return false;
                }
                let cols = self.launcher_cols.max(1);
                let n = visible.len();
                let pos = visible
                    .iter()
                    .position(|&i| i == self.selected_table)
                    .unwrap_or(0);
                let new_pos = match action {
                    LauncherAction::PrevCard => {
                        if pos > 0 {
                            pos - 1
                        } else {
                            n - 1
                        }
                    }
                    LauncherAction::NextCard => (pos + 1) % n,
                    LauncherAction::PrevRow => {
                        if pos >= cols {
                            pos - cols
                        } else {
                            (n - 1).min(pos + n - cols)
                        }
                    }
                    LauncherAction::NextRow => {
                        if pos + cols < n {
                            pos + cols
                        } else {
                            pos % cols
                        }
                    }
                    _ => unreachable!(),
                };
                self.selected_table = visible[new_pos];
                self.scroll_to_selected = true;
                true
            }
            LauncherAction::Launch => {
                if !self.tables.is_empty() {
                    let path = self.tables[self.selected_table].path.clone();
                    self.begin_table_launch(path, ctx);
                }
                false
            }
            LauncherAction::Cancel => {
                if !self.table_filter.is_empty() {
                    self.table_filter.clear();
                    self.table_filter_lower.clear();
                } else {
                    self.quit_launcher(ctx);
                }
                false
            }
        }
    }

    pub(super) fn handle_launcher_joystick(&mut self, ui: &mut egui::Ui) {
        use launcher_input::LauncherAction;
        let vpx_running = self.vpx_running.load(Ordering::Relaxed);
        // Drain joystick events into a local vec to avoid borrow conflict
        let events: Vec<JoystickEvent> = self
            .joystick_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();

        if vpx_running || self.tables.is_empty() {
            return;
        }

        // Key-repeat for held directional button: 400ms initial delay,
        // then 80ms interval — same cadence as a typical OS keyboard.
        const INITIAL_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
        const REPEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);
        if let Some((_, action, pressed_at, last_fire)) = self.nav_held {
            let now = std::time::Instant::now();
            if now.duration_since(pressed_at) >= INITIAL_DELAY
                && now.duration_since(last_fire) >= REPEAT_INTERVAL
                && self.apply_launcher_action(action, ui.ctx())
            {
                if let Some(held) = self.nav_held.as_mut() {
                    held.3 = now;
                }
                ui.ctx().request_repaint();
            }
        }

        for event in events {
            match &event {
                JoystickEvent::ButtonDown { button, .. } => {
                    let Some(action) = self
                        .action_for_launcher_nav(*button)
                        .as_deref()
                        .and_then(LauncherAction::from_vpx_action)
                    else {
                        continue;
                    };
                    // Joystick navigation parks the software cursor (dissolve
                    // + hover cleared): a pointer resting on a card would
                    // otherwise re-select it and override flipper navigation.
                    // egui never sees joystick events, so this is signalled
                    // manually; keyboard gets the same via the plugin's
                    // `with_dormant_on_keys`. Any mouse move reforms it.
                    Self::with_software_cursor(ui.ctx(), |c| c.set_dormant(true));
                    let applied = self.apply_launcher_action(action, ui.ctx());
                    if applied && action.is_directional() {
                        let now = std::time::Instant::now();
                        self.nav_held = Some((*button, action, now, now));
                    }
                }
                JoystickEvent::ButtonUp { button, .. } => {
                    if let Some((held_btn, _, _, _)) = &self.nav_held {
                        if held_btn == button {
                            self.nav_held = None;
                        }
                    }
                }
                JoystickEvent::AccelUpdate { .. } => {}
                _ => {}
            }
        }
    }

    pub(super) fn process_vpx_status(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.vpx_status_rx {
            while let Ok(status) = rx.try_recv() {
                match status {
                    VpxStatus::Loading(msg, pct) => {
                        self.vpx_loading_msg = msg;
                        self.vpx_loading_pct = pct;
                    }
                    VpxStatus::Started => {
                        self.vpx_loading_msg = "Startup done".to_string();
                        self.vpx_loading_pct = None;
                        self.vpx_hide_covers = true;
                        // Release the cursor capture so VPX gets the mouse —
                        // the plugin drops its OS grab on that transition
                        // (egui-rotate 1.1). Focus is released naturally
                        // because the kiosk focus-reclaim loop is gated on
                        // !vpx_running. VPX windows then z-order on top.
                        Self::with_software_cursor(ctx, |c| c.release());
                        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
                    }
                    VpxStatus::ExitOk => {
                        self.vpx_loading_msg.clear();
                        self.vpx_loading_pct = None;
                        self.vpx_hide_covers = false;
                        self.vpx_status_rx = None;
                        self.respawn_sdl_threads();
                        self.restore_kiosk_after_vpx(ctx);
                        return;
                    }
                    VpxStatus::ExitError(log) => {
                        self.vpx_loading_msg.clear();
                        self.vpx_hide_covers = false;
                        self.vpx_error_log = Some(log);
                        self.vpx_status_rx = None;
                        self.respawn_sdl_threads();
                        self.restore_kiosk_after_vpx(ctx);
                        return;
                    }
                    VpxStatus::LaunchError(msg) => {
                        self.vpx_loading_msg.clear();
                        self.vpx_hide_covers = false;
                        self.vpx_error_log = Some(msg);
                        self.vpx_status_rx = None;
                        self.respawn_sdl_threads();
                        self.restore_kiosk_after_vpx(ctx);
                        return;
                    }
                }
            }
        }
    }

    /// When VPX exits, trigger re-warp + re-focus on the next frame. The
    /// kiosk_cursor loop in App::ui handles the actual Focus + CursorPosition
    /// commands once vpx_running flips to false.
    fn restore_kiosk_after_vpx(&mut self, _ctx: &egui::Context) {
        if self.kiosk_cursor {
            self.kiosk_cursor_warped = false;
        }
    }

    pub(super) fn process_update_check(&mut self) {
        // Receive update check result
        if let Some(rx) = &self.update_check_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(release) => {
                        log::info!(
                            "Latest release: {} (installed: {})",
                            release.tag,
                            self.vpx_installed_tag
                        );

                        // Never offer auto-updates for manually installed VPX.
                        // Users managing manual installs are responsible for updates.
                        if self.vpx_install_mode == VpxInstallMode::Manual {
                            log::info!(
                                "Skipping update prompt: VPX was manually installed (not auto-downloaded)"
                            );
                            self.vpx_latest_release = None;
                        } else if release.tag != self.vpx_installed_tag {
                            self.vpx_latest_release = Some(release);
                        } else {
                            self.vpx_latest_release = None;
                        }
                    }
                    Err(e) => {
                        log::warn!("Update check failed: {e}");
                    }
                }
                self.update_check_rx = None;
            }
        }
        // Receive download progress
        if let Some(rx) = &self.update_progress_rx {
            while let Ok(progress) = rx.try_recv() {
                match progress {
                    UpdateProgress::Downloading(current, total) => {
                        self.update_progress = (current, total);
                    }
                    UpdateProgress::Extracting => {
                        self.update_downloading = true;
                    }
                    UpdateProgress::Done(exe_path) => {
                        let path_str = exe_path.display().to_string();
                        self.vpx_exe_path = path_str.clone();
                        let _ = self.db.set_config("vpx_exe_path", &path_str);
                        if let Some(rel) = &self.vpx_latest_release {
                            self.vpx_installed_tag = rel.tag.clone();
                            let _ = self.db.set_config("vpx_installed_tag", &rel.tag);
                        }
                        self.update_downloading = false;
                        self.update_progress = (0, 0);
                        self.vpx_latest_release = None;
                        self.update_progress_rx = None;
                        self.update_error = None;
                        log::info!("Visual Pinball installed to: {}", path_str);
                        return;
                    }
                    UpdateProgress::Error(msg) => {
                        self.update_downloading = false;
                        self.update_error = Some(msg.clone());
                        self.update_progress_rx = None;
                        log::error!("Visual Pinball update failed: {}", msg);
                        return;
                    }
                }
            }
        }
    }

    pub(super) fn start_vpx_download(&mut self, release: &ReleaseInfo) {
        let install_dir = std::path::PathBuf::from(&self.vpx_install_dir);
        let release = release.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.update_progress_rx = Some(rx);
        self.update_downloading = true;
        self.update_progress = (0, release.asset_size);
        self.update_error = None;
        std::thread::spawn(move || {
            if let Err(e) = updater::download_and_install(&release, &install_dir, tx.clone()) {
                let _ = tx.send(UpdateProgress::Error(format!("{e}")));
            }
        });
    }

    /// Poll the PinReady self-update channels. On a completed download the
    /// running process exits immediately — the freshly-spawned child from
    /// `download_pinready_and_replace` takes over as the user-facing instance.
    pub(super) fn process_pinready_update_check(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.pinready_update_check_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(release) => {
                        if updater::is_pinready_update_available(&release) {
                            log::info!(
                                "PinReady update available: {} (running: {})",
                                release.tag,
                                updater::CURRENT_PINREADY_VERSION
                            );
                            self.pinready_latest_release = Some(release);
                        } else {
                            log::info!("PinReady is up to date ({})", release.tag);
                            self.pinready_latest_release = None;
                        }
                    }
                    Err(e) => log::warn!("PinReady update check failed: {e}"),
                }
                self.pinready_update_check_rx = None;
            }
        }

        if let Some(rx) = &self.pinready_update_progress_rx {
            while let Ok(progress) = rx.try_recv() {
                match progress {
                    UpdateProgress::Downloading(current, total) => {
                        self.pinready_update_progress = (current, total);
                    }
                    UpdateProgress::Extracting => {
                        self.pinready_updating = true;
                    }
                    UpdateProgress::Done(_) => {
                        log::info!("PinReady update: binary replaced, restarting");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        std::process::exit(0);
                    }
                    UpdateProgress::Error(msg) => {
                        self.pinready_updating = false;
                        self.pinready_update_error = Some(msg.clone());
                        self.pinready_update_progress_rx = None;
                        log::error!("PinReady update failed: {}", msg);
                        return;
                    }
                }
            }
        }
    }

    pub(super) fn start_pinready_download(&mut self, release: &ReleaseInfo) {
        let release = release.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.pinready_update_progress_rx = Some(rx);
        self.pinready_updating = true;
        self.pinready_update_progress = (0, release.asset_size);
        self.pinready_update_error = None;
        std::thread::spawn(move || {
            if let Err(e) = updater::download_pinready_and_replace(&release, tx.clone()) {
                let _ = tx.send(UpdateProgress::Error(format!("{e}")));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pct_with_integer() {
        let pct = parse_progress_pct("Initializing Visuals... 10%");
        assert!(pct.is_some());
        assert!((pct.unwrap() - 0.10).abs() < 0.001);
    }

    #[test]
    fn parse_pct_full() {
        let pct = parse_progress_pct("Done 100%");
        assert!(pct.is_some());
        assert!((pct.unwrap() - 1.0).abs() < 0.001);
    }

    #[test]
    fn parse_pct_zero() {
        let pct = parse_progress_pct("Starting 0%");
        assert!(pct.is_some());
        assert!((pct.unwrap() - 0.0).abs() < 0.001);
    }

    #[test]
    fn parse_pct_no_percentage() {
        assert!(parse_progress_pct("Loading...").is_none());
    }

    #[test]
    fn parse_pct_no_number_before_percent() {
        assert!(parse_progress_pct("Progress: %").is_none());
    }

    #[test]
    fn parse_pct_clamped_above_100() {
        let pct = parse_progress_pct("Overflow 150%");
        assert!(pct.is_some());
        assert!((pct.unwrap() - 1.0).abs() < 0.001);
    }

    #[test]
    fn parse_pct_with_decimal() {
        let pct = parse_progress_pct("Loading 33.5%");
        assert!(pct.is_some());
        assert!((pct.unwrap() - 0.335).abs() < 0.001);
    }

    #[test]
    fn parse_pct_embedded_in_brackets() {
        // Realistic VPX format: "[INFO SetProgress] Loading Textures... 45%"
        let pct = parse_progress_pct("Loading Textures... 45%");
        assert!(pct.is_some());
        assert!((pct.unwrap() - 0.45).abs() < 0.001);
    }
}
