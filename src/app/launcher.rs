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
        if let Ok(meta) = std::fs::metadata(candidate)
            && let Ok(m) = meta.modified()
            && let Ok(d) = m.duration_since(std::time::UNIX_EPOCH)
        {
            max_mtime = max_mtime.max(d.as_secs() as i64);
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
        if let Ok(meta) = std::fs::metadata(candidate)
            && let Ok(m) = meta.modified()
            && let Ok(d) = m.duration_since(std::time::UNIX_EPOCH)
        {
            max_mtime = max_mtime.max(d.as_secs() as i64);
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

/// Per-launch transcript of everything we heard from VPX, written as it
/// arrives.
///
/// Three problems, one file. VPX appends every session to the same
/// `vpinball.log`, so a report can carry two launches with no way to tell
/// where one ends. A hard crash costs the tail of whatever was still
/// buffered. And when the error popup itself misbehaves — it has — the user
/// is left looking at a report they cannot get off the screen.
///
/// So: one file per launch, opened before VPX starts, carrying the exact
/// command line, flushed line by line, and named in the error report.
struct RunLog {
    path: std::path::PathBuf,
    file: Option<std::fs::File>,
}

impl RunLog {
    /// Directory holding the transcripts, next to `PinReady.log`.
    fn dir() -> Option<std::path::PathBuf> {
        Some(crate::db::default_db_path().parent()?.join("vpx-runs"))
    }

    /// Open the transcript for a launch and write `header` into it. Failing is
    /// never fatal: losing a diagnostic must not stop anyone playing.
    fn create(table: &std::path::Path, header: &str) -> Self {
        let stem: String = table
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "table".into())
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .take(60)
            .collect();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let Some(dir) = Self::dir() else {
            return Self {
                path: std::path::PathBuf::new(),
                file: None,
            };
        };
        let _ = std::fs::create_dir_all(&dir);
        Self::prune(&dir);
        let path = dir.join(format!("vpx-{stamp}-{stem}.log"));
        let file = std::fs::File::create(&path)
            .inspect_err(|e| {
                log::warn!("could not open the run transcript {}: {e}", path.display());
            })
            .ok();
        let mut me = Self { path, file };
        me.write(header);
        me
    }

    /// Keep the most recent runs and drop the rest: this is a diagnostic aid,
    /// not an archive, and a cabinet should not accumulate them forever.
    fn prune(dir: &std::path::Path) {
        const KEEP: usize = 10;
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect();
        if files.len() < KEEP {
            return;
        }
        // Names start with a timestamp, so lexical order is chronological.
        files.sort();
        for old in files.iter().take(files.len() + 1 - KEEP) {
            let _ = std::fs::remove_file(old);
        }
    }

    /// Append and flush at once: whatever kills VPX must not take the last
    /// lines with it, which is exactly what makes `vpinball.log` unhelpful
    /// after a crash.
    fn write(&mut self, text: &str) {
        use std::io::Write as _;
        let Some(f) = self.file.as_mut() else {
            return;
        };
        if f.write_all(text.as_bytes()).is_err() || f.flush().is_err() {
            self.file = None;
        }
    }

    fn line(&mut self, tag: &str, line: &str) {
        self.write(&format!("[{tag}] {line}\n"));
    }
}

/// One line read from the child, and which stream it came out of.
///
/// Both streams feed a single watcher. Splitting them was a real bug: the
/// startup/progress detection only ever looked at stdout, so anything VPX
/// wrote to stderr — which is where a logger routes warnings by default —
/// could never satisfy it. `Startup done` arriving on the wrong pipe left
/// `startup_done` false and the 30 s loading hang-detector armed for the
/// whole session.
enum VpxLine {
    Out(String),
    Err(String),
    /// A line from VPX's own log file — see [`LogTail`].
    Log(String),
}

/// Follow VPX's own log file the way `tail -f` does.
///
/// This is the only stream VPX itself controls, and the only one that carries
/// `SetProgress` and `Startup done`. Its console output does not: the plog
/// console appender is compiled in under `__STANDALONE__` only, which the MSVC
/// Windows build does not define, so what lands in our pipes there is whatever
/// PinMAME and dmdutil happen to print on their own. Deducing VPX's state from
/// that was guesswork — a table was killed mid-load right after
/// `loading fshtl_5.rom`, because the ROM chatter stopped for 30 s.
struct LogTail {
    path: std::path::PathBuf,
    pos: u64,
    /// Bytes read past the last newline, waiting for the rest of their line.
    carry: String,
}

impl LogTail {
    /// Start at the end of whatever is already there: plog opens the file for
    /// append, so beginning at 0 would replay every previous session as if it
    /// were happening now. A missing file (first ever launch) starts at 0,
    /// which is also correct — VPX is about to create it.
    fn new(path: std::path::PathBuf) -> Self {
        let pos = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self {
            path,
            pos,
            carry: String::new(),
        }
    }

    /// Every complete line appended since the last call. Empty while the file
    /// does not exist yet, which is the normal state for the first seconds of
    /// a first launch — and stays empty forever if the user turned VPX's
    /// logging off, which is why nothing may depend on this being non-empty.
    fn read_new(&mut self) -> Vec<String> {
        use std::io::{Read, Seek};
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.pos {
            // Rolled over (plog keeps 5 MB and one backup) or truncated:
            // whatever we were following is gone, follow the new file.
            self.pos = 0;
            self.carry.clear();
        }
        if len == self.pos {
            return Vec::new();
        }
        if f.seek(std::io::SeekFrom::Start(self.pos)).is_err() {
            return Vec::new();
        }
        let mut buf = Vec::new();
        let read = f
            .by_ref()
            .take(len - self.pos)
            .read_to_end(&mut buf)
            .unwrap_or(0);
        self.pos += read as u64;
        // Lossy on purpose: a torn multi-byte character at the read boundary
        // must not stall the tail, and this is a diagnostic stream.
        self.carry.push_str(&String::from_utf8_lossy(&buf));
        let mut out = Vec::new();
        while let Some(i) = self.carry.find('\n') {
            let line: String = self.carry.drain(..=i).collect();
            out.push(line.trim_end_matches(['\r', '\n']).to_string());
        }
        out
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
        let generation = self.scan_generation;
        let enrichment_on = self.db.catalog_enrichment_enabled();

        // Sync VPSDB + MediaDb on a small bootstrap thread so we don't
        // block the UI; once both indices are loaded we hand them
        // (Arc-shared) to the worker pool.
        std::thread::Builder::new()
            .name(format!("pinready-scan-bootstrap-{generation}"))
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
                    generation,
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
        // The driver was chosen (and the config reconciled to it) once at
        // launcher startup; carry it into the launch thread for SDL_VIDEODRIVER.
        let vpx_driver = self.vpx_driver.clone();
        // VPX writes `vpinball.log` beside the settings file it loaded, so the
        // ini PinReady is actually using is what says where to listen. Deriving
        // it from the default location instead would silently watch the wrong
        // file whenever that ini is somewhere else — and a wrong file reads
        // exactly like a VPX with nothing to say.
        let vpx_log_path = self.config.path().with_file_name("vpinball.log");

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
            // Pin VPX's SDL backend to the driver chosen above (Linux only —
            // `None` on macOS/Windows, a no-op there). Newer SDL3 auto-selects
            // XWayland when it sees both DISPLAY and WAYLAND_DISPLAY; forcing
            // the name here overrides any inherited or auto-detected value, and
            // it matches the driver the `*Display=` names were just reconciled
            // for so VPX finds its screens.
            if let Some(driver) = &vpx_driver {
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
            // Snapshot where VPX's own log currently ends, BEFORE starting it:
            // plog appends, so anything already there belongs to a previous
            // session and must not be replayed as if it were happening now.
            let mut log_tail = LogTail::new(vpx_log_path);
            // Opened BEFORE the spawn, so the exact command line survives even
            // a failure to start.
            let vpx_log_path = log_tail.path.clone();
            let mut run_log = RunLog::create(&path, &call_header());
            let child = cmd.spawn();
            match child {
                Ok(mut child) => {
                    log::info!("Visual Pinball launched, reading stdout+stderr...");

                    let stdout = child.stdout.take();
                    let stderr = child.stderr.take();
                    let mut stderr_lines: Vec<String> = Vec::new();
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
                    // Has a line from VPX's OWN logger ever reached us?
                    //
                    // Not the same question as "did anything arrive". On
                    // Windows plenty arrives — PinMAME prints its ROM loading,
                    // dmdutil64.dll prints its converter warnings — but none of
                    // it comes from VPX, because VPX's plog only gets a console
                    // appender under `__STANDALONE__`, which the MSVC build does
                    // not define. So `SetProgress` and `Startup done` never
                    // arrive, `startup_done` can never flip, and the loading
                    // hang-detector below stays armed for the whole session.
                    let mut saw_vpx_log = false;
                    // Build the full log we hand to ExitError. Always
                    // contains the call header and every loading-phase
                    // line; if startup_done was reached, also a visible
                    // separator and the in-game tail.
                    let run_log_path = run_log.path.clone();
                    let build_error_log = |reason: &str,
                                           loading: &[String],
                                           ingame: &std::collections::VecDeque<String>,
                                           heard_vpx: bool|
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
                        if !run_log_path.as_os_str().is_empty() {
                            // Name the transcript: when the popup itself
                            // misbehaves, this is how the report still
                            // reaches us.
                            out.push_str("\nFull transcript of this launch:\n  ");
                            out.push_str(&run_log_path.display().to_string());
                            out.push('\n');
                        }
                        if !heard_vpx {
                            // Not one line from VPX itself — whatever we did
                            // capture came from PinMAME or dmdutil. That also
                            // means the stopped-responding detector never
                            // armed, so say so: a launch left silently
                            // unprotected is the failure we keep having to
                            // hunt. Naming the file makes a wrong path (a
                            // custom -PrefPath, a different VPX layout)
                            // obvious to whoever reads the report.
                            out.push_str(
                                "\n----- PinReady never heard from VPX -----\n\
                                 No line from VPX's own log reached us, so the \
                                 stopped-responding detector stayed disarmed for this \
                                 launch.\nThe file being watched was:\n  ",
                            );
                            out.push_str(&vpx_log_path.display().to_string());
                            out.push_str(
                                "\nIf that path is wrong, or logging is off in VPX's editor \
                                 options, PinReady is blind to what VPX is doing.\n",
                            );
                        }
                        out
                    };

                    {
                        // Poll often, decide slowly: draining every 250 ms
                        // notices an exit promptly, while the hang rule below
                        // still measures a real 30 s of silence.
                        const POLL: std::time::Duration = std::time::Duration::from_millis(250);
                        const HANG_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
                        let (line_tx, line_rx) = crossbeam_channel::unbounded();

                        // Both pipes feed the channel. They carry the
                        // third-party output — PinMAME's ROM loading, dmdutil's
                        // converter warnings — which is worth keeping for the
                        // report, plus VPX's own lines on the builds that have
                        // a console appender.
                        for (stream, tag) in [
                            (
                                stdout.map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
                                false,
                            ),
                            (
                                stderr.map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
                                true,
                            ),
                        ] {
                            let Some(stream) = stream else { continue };
                            let tx = line_tx.clone();
                            std::thread::spawn(move || {
                                let reader = std::io::BufReader::new(stream);
                                for line in reader.lines().map_while(Result::ok) {
                                    let item = if tag {
                                        VpxLine::Err(line)
                                    } else {
                                        VpxLine::Out(line)
                                    };
                                    if tx.send(item).is_err() {
                                        break;
                                    }
                                }
                            });
                        }
                        // ...and VPX's own log file, the one stream it drives
                        // on every platform.
                        let tail_stop = std::sync::Arc::new(AtomicBool::new(false));
                        let tail_handle = {
                            let tx = line_tx.clone();
                            let stop = std::sync::Arc::clone(&tail_stop);
                            std::thread::spawn(move || {
                                loop {
                                    // Read the flag first, then the file: the
                                    // pass after VPX exits still collects what
                                    // it wrote on the way out, which is exactly
                                    // where a crash reason lives.
                                    let stopping = stop.load(Ordering::Relaxed);
                                    for line in log_tail.read_new() {
                                        if tx.send(VpxLine::Log(line)).is_err() {
                                            return;
                                        }
                                    }
                                    if stopping {
                                        return;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(200));
                                }
                            })
                        };
                        // Our own handle must go, or `Disconnected` never fires.
                        drop(line_tx);

                        let mut last_vpx_line = std::time::Instant::now();
                        let mut drain_until: Option<std::time::Instant> = None;
                        loop {
                            match line_rx.recv_timeout(POLL) {
                                Ok(item) => {
                                    let (line, from_log) = match item {
                                        VpxLine::Out(l) => {
                                            log::info!("[VPX] {}", l);
                                            run_log.line("out", &l);
                                            (l, false)
                                        }
                                        VpxLine::Err(l) => {
                                            log::warn!("[VPX stderr] {}", l);
                                            run_log.line("err", &l);
                                            stderr_lines.push(l.clone());
                                            (l, false)
                                        }
                                        VpxLine::Log(l) => {
                                            log::info!("[VPX log] {}", l);
                                            run_log.line("log", &l);
                                            (l, true)
                                        }
                                    };
                                    // A line is VPX's own if it came from VPX's
                                    // log file, or if it carries a marker only
                                    // VPX emits. Anything else is a library
                                    // shouting into our pipe and says nothing
                                    // about whether VPX is alive.
                                    if from_log
                                        || line.contains("SetProgress")
                                        || line.contains("Startup done")
                                        || line.contains("RenderStaticPrepass")
                                        || line.contains("PluginLog")
                                    {
                                        saw_vpx_log = true;
                                        last_vpx_line = std::time::Instant::now();
                                    }
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
                                    continue;
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                                    // Every producer is gone.
                                    break;
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                            }

                            // Idle tick: has VPX exited, or stopped talking?
                            if let Some(deadline) = drain_until {
                                // It exited; keep pumping briefly so the log's
                                // final lines make it into the report.
                                if std::time::Instant::now() >= deadline {
                                    break;
                                }
                                continue;
                            }
                            if matches!(child.try_wait(), Ok(Some(_))) {
                                tail_stop.store(true, Ordering::Relaxed);
                                drain_until = Some(
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                );
                                continue;
                            }
                            if last_vpx_line.elapsed() >= HANG_AFTER {
                                if startup_done {
                                    // After startup, silence is normal: an
                                    // in-game VPX logs sparsely. Keep draining —
                                    // dropping `line_rx` would close the read end
                                    // of the pipes, and VPX's next write triggers
                                    // SIGPIPE and kills the game mid-play.
                                    continue;
                                }
                                if !saw_vpx_log {
                                    // VPX has never spoken to us, so its silence says
                                    // nothing and killing on it is a guess. On Windows
                                    // that guess is always wrong: whatever we did
                                    // receive came from PinMAME or dmdutil, and once
                                    // those go quiet — a long load, or simply a table
                                    // running — the 30 s timer fires on a perfectly
                                    // healthy game. Field report: a table killed
                                    // mid-load right after `loading fshtl_5.rom`.
                                    //
                                    // Treat the process as up instead: mark startup
                                    // done so the covers lift and the cursor is handed
                                    // to VPX, keep draining, and let VPX decide when it
                                    // exits. We lose the hang detector where VPX does
                                    // not report — there is no signal here to rebuild
                                    // it from, and killing someone's game on a guess is
                                    // the worse failure.
                                    log::warn!(
                                        "no VPX log line in 30s (only third-party output, if any): \
                                             this build has no console logger. Assuming it started; \
                                             VPX's own diagnostics are in vpinball.log."
                                    );
                                    startup_done = true;
                                    let _ = tx.send(VpxStatus::Started);
                                    continue;
                                }
                                log::error!(
                                    "VPX log silent for 30s during loading — treating as hung"
                                );
                                let _ = child.kill();
                                let err = build_error_log(
                                    "Timeout: Visual Pinball stopped responding during loading (no output for 30s).",
                                    &loading_log,
                                    &ingame_log,
                                    saw_vpx_log,
                                );
                                let _ = tx.send(VpxStatus::ExitError(err));
                                tail_stop.store(true, Ordering::Relaxed);
                                running.store(false, Ordering::Relaxed);
                                return;
                            }
                        }
                        tail_stop.store(true, Ordering::Relaxed);
                        let _ = tail_handle.join();
                    }

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
                                let mut combined = build_error_log(
                                    &reason,
                                    &loading_log,
                                    &ingame_log,
                                    saw_vpx_log,
                                );
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
                                saw_vpx_log,
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
                if let Some(table) = self.tables.get(cur)
                    && let Some(table_dir) = table.path.parent()
                {
                    let audio_path = table_dir.join("medias").join("audio.mp3");
                    if audio_path.is_file()
                        && let Some(tx) = &self.audio_cmd_tx
                    {
                        // Preview clips are halved so they sit
                        // below the in-game soundtrack baseline —
                        // hovering over a card shouldn't be louder
                        // than the table the user is browsing for.
                        let volume = (self.audio.music_volume as f32 / 100.0 * 0.5).clamp(0.0, 1.0);
                        let _ = tx.send(AudioCommand::PreviewStart {
                            path: audio_path,
                            volume,
                        });
                        self.preview_playing = true;
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
            if let Some(inputs::CapturedInput::JoystickButton { button: b, .. }) = &action.joystick
                && *b == button
            {
                return Some(action.setting_id.to_string());
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
            crate::app::DMD_VIEWPORT,
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

    /// How long the launcher stays deaf after VPX exits.
    pub(super) const INPUT_GRACE_AFTER_VPX: std::time::Duration = std::time::Duration::from_secs(1);

    /// True while the post-VPX-exit grace window is still open. Clears the
    /// deadline once it has elapsed so the check stays cheap afterwards.
    pub(super) fn launcher_input_suppressed(&mut self) -> bool {
        match self.input_resume_at {
            Some(at) if std::time::Instant::now() < at => true,
            Some(_) => {
                self.input_resume_at = None;
                false
            }
            None => false,
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

        // Post-exit grace: the events we just drained (the exit button, its
        // release, any repeat) are discarded, and a held-nav is cancelled,
        // until the window elapses. Draining here is what stops them firing
        // the instant the window closes.
        if self.launcher_input_suppressed() {
            self.nav_held = None;
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
                    if let Some((held_btn, _, _, _)) = &self.nav_held
                        && held_btn == button
                    {
                        self.nav_held = None;
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
        // Hold off launcher input for a beat: the exit button/key that just
        // closed VPX is frequently still down (or already queued in our
        // joystick channel) when control returns here, and the launcher
        // would read it as `Cancel` and quit itself. Input paths drain and
        // ignore events until this instant.
        //
        // A second, not a few frames. The key does not arrive when VPX dies
        // but when the compositor hands focus back, which on a cabinet comes
        // after the window teardown, the output remode and our own repaint —
        // easily past a short window, and the launcher then quits along with
        // the table.
        self.input_resume_at = Some(std::time::Instant::now() + Self::INPUT_GRACE_AFTER_VPX);
    }

    pub(super) fn process_update_check(&mut self) {
        // Receive update check result
        if let Some(rx) = &self.update_check_rx
            && let Ok(result) = rx.try_recv()
        {
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
        if let Some(rx) = &self.pinready_update_check_rx
            && let Ok(result) = rx.try_recv()
        {
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

    /// A pre-existing log belongs to the previous session: starting at 0 would
    /// replay it and, worse, re-trigger `Startup done` before VPX had started.
    #[test]
    fn log_tail_starts_at_the_end_of_what_is_already_there() {
        let path = std::env::temp_dir().join("pinready-tail-existing.log");
        std::fs::write(&path, "old line from last time\n").unwrap();
        let mut tail = LogTail::new(path.clone());
        assert!(
            tail.read_new().is_empty(),
            "the old session must not replay"
        );
        append(&path, "brand new line\n");
        assert_eq!(tail.read_new(), vec!["brand new line".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    /// The first launch creates the file after we start watching: a missing
    /// file is a normal state, not an error, and must be picked up once it
    /// appears.
    #[test]
    fn log_tail_waits_for_a_file_that_does_not_exist_yet() {
        let path = std::env::temp_dir().join("pinready-tail-absent.log");
        let _ = std::fs::remove_file(&path);
        let mut tail = LogTail::new(path.clone());
        assert!(tail.read_new().is_empty());
        std::fs::write(&path, "first line\n").unwrap();
        assert_eq!(tail.read_new(), vec!["first line".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    /// Reads land mid-line all the time. Half a line must be held back, not
    /// emitted as if it were complete — `Startup don` matches nothing.
    #[test]
    fn log_tail_holds_back_a_partial_line() {
        let path = std::env::temp_dir().join("pinready-tail-partial.log");
        std::fs::write(&path, "").unwrap();
        let mut tail = LogTail::new(path.clone());
        append(&path, "Startup do");
        assert!(tail.read_new().is_empty(), "half a line is not a line");
        append(&path, "ne\nnext\n");
        assert_eq!(
            tail.read_new(),
            vec!["Startup done".to_string(), "next".to_string()]
        );
        let _ = std::fs::remove_file(&path);
    }

    /// plog rolls the file at 5 MB. After a roll the new file is shorter than
    /// our cursor; following from the old offset would skip real content or
    /// read garbage.
    #[test]
    fn log_tail_follows_the_file_across_a_rollover() {
        let path = std::env::temp_dir().join("pinready-tail-roll.log");
        std::fs::write(&path, "a long first session line\n").unwrap();
        let mut tail = LogTail::new(path.clone());
        append(&path, "second line\n");
        assert_eq!(tail.read_new(), vec!["second line".to_string()]);
        // Rolled over: same name, fresh and shorter.
        std::fs::write(&path, "after roll\n").unwrap();
        assert_eq!(tail.read_new(), vec!["after roll".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    fn append(path: &std::path::Path, text: &str) {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    /// The transcript must carry the launch command *before* VPX has said
    /// anything: a crash on start still leaves the exact command line on disk.
    #[test]
    fn run_log_records_the_command_before_any_output() {
        let table = std::path::Path::new("/tmp/Fish Tales (Williams 1992).vpx");
        let header = "$ 'VPinballX' -Play 'Fish Tales'\n  system: test\n\n";
        let mut rl = RunLog::create(table, header);
        assert!(rl.path.exists(), "the file is created up front");
        rl.line("out", "loading fshtl_5.rom");
        rl.line("err", "something on stderr");
        let body = std::fs::read_to_string(&rl.path).unwrap();
        assert!(body.contains("-Play 'Fish Tales'"), "{body}");
        assert!(body.contains("[out] loading fshtl_5.rom"), "{body}");
        assert!(body.contains("[err] something on stderr"), "{body}");
        // Named after the table, so a folder of them reads at a glance.
        let name = rl.path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.contains("Fish-Tales"), "{name}");
        let _ = std::fs::remove_file(&rl.path);
    }

    /// A cabinet launches tables all evening; the folder must not grow without
    /// bound, and it must be the OLDEST that go.
    #[test]
    fn run_log_prunes_to_the_most_recent() {
        let dir = std::env::temp_dir().join("pinready-runlog-prune");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..15 {
            std::fs::write(dir.join(format!("vpx-{i:04}-t.log")), "x").unwrap();
        }
        RunLog::prune(&dir);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        // 9 kept, so the run about to be created makes 10.
        assert_eq!(left.len(), 9, "{left:?}");
        assert_eq!(
            left[0], "vpx-0006-t.log",
            "the oldest must be the ones dropped"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
