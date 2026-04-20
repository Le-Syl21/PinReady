use super::*;

/// Parse a percentage from a VPX SetProgress message.
/// Examples: "Initializing Visuals... 10%" → Some(0.10), "Loading..." → None
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
    pub(super) fn finalize_wizard(&mut self, ctx: &egui::Context) {
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

        // Knocker surprise!
        if let Some(tx) = &self.audio_cmd_tx {
            let _ = tx.send(AudioCommand::PlayOnSpeaker {
                path: "knocker.ogg".to_string(),
                target: audio::SpeakerTarget::FrontBoth,
            });
        }

        log::info!("Wizard completed! Config saved to VPinballX.ini");

        // Scan tables and switch to launcher mode
        self.scan_tables();
        self.mode = AppMode::Launcher;

        // Apply cabinet mode live if BGSet=1. OuterPosition on a mapped window
        // may be ignored by Mutter/GNOME — a cold restart is then more reliable
        // (the next run creates the window directly at the PF position).
        self.enter_cabinet_mode_if_configured(ctx);
    }

    /// If config has BGSet=1 and a Playfield display is known, rotate the
    /// viewport CW90 and move it to the PF monitor via SetMonitor. Cursor
    /// lock + warp are enabled via enable_kiosk_cursor.
    pub(super) fn enter_cabinet_mode_if_configured(&mut self, ctx: &egui::Context) {
        let cabinet = self.config.get_i32("Player", "BGSet") == Some(1);
        if !cabinet {
            return;
        }
        let playfield_name = match self.config.get("Player", "PlayfieldDisplay") {
            Some(n) if !n.is_empty() => n,
            _ => return,
        };
        let idx = match self.displays.iter().position(|d| d.name == playfield_name) {
            Some(i) => i,
            None => {
                log::warn!(
                    "Cabinet mode requested but Playfield display '{}' not found",
                    playfield_name
                );
                return;
            }
        };
        log::info!(
            "Entering cabinet mode live: rotating CW90, moving to monitor {}",
            idx
        );
        ctx.set_viewport_rotation(eframe::emath::ViewportRotation::CW90);
        ctx.send_viewport_cmd(egui::ViewportCommand::SetMonitor(idx));
        self.enable_kiosk_cursor();
    }

    pub(super) fn scan_tables(&mut self) {
        self.tables.clear();
        let dir = if self.tables_dir.is_empty() {
            return;
        } else {
            &self.tables_dir
        };
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            log::warn!("Tables directory does not exist: {}", dir);
            return;
        }
        // Scan for .vpx files (folder-per-table layout: each subfolder has a .vpx)
        if let Ok(entries) = std::fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                // Look for .vpx file inside this folder
                if let Ok(files) = std::fs::read_dir(&path) {
                    for file in files.flatten() {
                        let fp = file.path();
                        if fp.extension().and_then(|e| e.to_str()) == Some("vpx") {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .replace('_', " ");
                            let b2s_path = fp.with_extension("directb2s");
                            let has_directb2s = b2s_path.exists();
                            let cached = crate::assets::cached_bg_path(&path);
                            let (bg_path, bg_bytes) = if cached.exists() {
                                let bytes = std::fs::read(&cached)
                                    .ok()
                                    .map(|b| std::sync::Arc::from(b.into_boxed_slice()));
                                (Some(cached), bytes)
                            } else {
                                (None, None)
                            };
                            self.tables.push(TableEntry {
                                path: fp,
                                name,
                                has_directb2s,
                                bg_path,
                                bg_bytes,
                            });
                            break; // one vpx per folder
                        }
                    }
                }
            }
        }
        self.tables.sort_by_key(|a| a.name.to_lowercase());
        log::info!("Scanned {} tables in {}", self.tables.len(), dir);

        // Spawn background thread to extract missing backglass images
        let (tx, rx) = crossbeam_channel::unbounded();
        let jobs: Vec<(usize, std::path::PathBuf, std::path::PathBuf)> = self
            .tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.bg_path.is_none() && t.has_directb2s)
            .map(|(i, t)| {
                let b2s = t.path.with_extension("directb2s");
                let table_dir = t
                    .path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_path_buf();
                (i, b2s, table_dir)
            })
            .collect();
        if !jobs.is_empty() {
            log::info!(
                "Extracting {} backglass images in background...",
                jobs.len()
            );
            std::thread::spawn(move || {
                for (idx, b2s_path, table_dir) in jobs {
                    if let Some(cached) = crate::assets::extract_backglass(&b2s_path, &table_dir) {
                        let _ = tx.send((idx, cached));
                    }
                }
                log::info!("Background backglass extraction complete");
            });
        }
        self.bg_rx = Some(rx);
    }

    pub(super) fn launch_table(&mut self, table_path: &std::path::Path) {
        if self.vpx_running.load(Ordering::Relaxed) {
            return;
        }
        let resolved = updater::resolve_vpx_exe(std::path::Path::new(&self.vpx_exe_path));
        if self.vpx_exe_path.is_empty() || !resolved.is_file() {
            log::error!("Visual Pinball executable not found: {}", self.vpx_exe_path);
            return;
        }
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
            let child = std::process::Command::new(&exe)
                .arg("-Play")
                .arg(&path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();
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
                    let mut log_lines: Vec<String> = Vec::new();
                    let mut startup_done = false;

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
                                        let _ = tx.send(VpxStatus::Started);
                                    }
                                    log_lines.push(line);
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                                    if startup_done {
                                        // After startup, silence is normal (game is running)
                                        // Wait for process exit instead
                                        break;
                                    }
                                    log::error!(
                                        "VPX stdout timeout (30s without output during loading)"
                                    );
                                    let _ = child.kill();
                                    let mut err = "Timeout: Visual Pinball stopped responding during loading (no output for 30s).\n\n".to_string();
                                    let tail: Vec<String> =
                                        log_lines.iter().rev().take(50).rev().cloned().collect();
                                    err.push_str(&tail.join("\n"));
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

                    match child.wait() {
                        Ok(status) => {
                            log::info!("Visual Pinball exited with status: {status}");
                            if status.success() || startup_done {
                                let _ = tx.send(VpxStatus::ExitOk);
                            } else {
                                let mut combined = String::new();
                                let tail: Vec<String> =
                                    log_lines.iter().rev().take(50).rev().cloned().collect();
                                combined.push_str(&tail.join("\n"));
                                if !stderr_lines.is_empty() {
                                    combined.push_str("\n\n--- stderr ---\n");
                                    combined.push_str(&stderr_lines.join("\n"));
                                }
                                let _ = tx.send(VpxStatus::ExitError(combined));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to wait for Visual Pinball: {e}");
                            let _ = tx.send(VpxStatus::ExitError(format!("Process error: {e}")));
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to launch Visual Pinball: {e}");
                    let _ = tx.send(VpxStatus::LaunchError(format!("{e}")));
                }
            }
            running.store(false, Ordering::Relaxed);
        });
    }

    pub(super) fn process_bg_extraction(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.bg_rx {
            loop {
                match rx.try_recv() {
                    Ok((idx, path)) => {
                        if idx < self.tables.len() {
                            if let Ok(bytes) = std::fs::read(&path) {
                                let arc: std::sync::Arc<[u8]> =
                                    std::sync::Arc::from(bytes.into_boxed_slice());
                                let uri = format!("bytes://bg/{idx}");
                                ctx.include_bytes(uri, arc.clone());
                                self.tables[idx].bg_bytes = Some(arc);
                            }
                            self.tables[idx].bg_path = Some(path.clone());
                            log::debug!("BG extracted for table {idx}: {}", path.display());
                        }
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        // Extraction thread finished — stop polling
                        log::info!("Background backglass extraction channel closed");
                        self.bg_rx = None;
                        break;
                    }
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
            let uri = format!("bytes://bg/{idx}");
            if let Some(ref arc) = table.bg_bytes {
                ctx.include_bytes(uri, arc.clone());
                count += 1;
            } else if let Some(ref path) = table.bg_path {
                if let Ok(bytes) = std::fs::read(path) {
                    ctx.include_bytes(uri, bytes);
                    count += 1;
                }
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

    /// Unified exit: release cursor capture (otherwise the OS cursor stays
    /// hidden while the window tears down), then request window close.
    /// Called from the Quit button, ExitGame joystick action, and Escape key.
    pub(super) fn quit_launcher(&self, ctx: &egui::Context) {
        ctx.set_cursor_lock(false);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Apply a repeatable launcher navigation action. Returns true if applied.
    pub(super) fn apply_nav_action(&mut self, action: &str) -> bool {
        let len = self.tables.len();
        let cols = self.launcher_cols.max(1);
        match action {
            "LeftFlipper" => {
                self.selected_table = if self.selected_table > 0 {
                    self.selected_table - 1
                } else {
                    len - 1
                };
                self.scroll_to_selected = true;
                true
            }
            "RightFlipper" => {
                self.selected_table = (self.selected_table + 1) % len;
                self.scroll_to_selected = true;
                true
            }
            "LeftMagna" => {
                self.selected_table = if self.selected_table >= cols {
                    self.selected_table - cols
                } else {
                    (len - 1).min(self.selected_table + len - cols)
                };
                self.scroll_to_selected = true;
                true
            }
            "RightMagna" => {
                self.selected_table = if self.selected_table + cols < len {
                    self.selected_table + cols
                } else {
                    self.selected_table % cols
                };
                self.scroll_to_selected = true;
                true
            }
            _ => false,
        }
    }

    pub(super) fn handle_launcher_joystick(&mut self, ui: &mut egui::Ui) {
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

        // Key-repeat for held nav button: 400ms initial delay, then 80ms interval
        const INITIAL_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
        const REPEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);
        if let Some((_, action, pressed_at, last_fire)) = self.nav_held.clone() {
            let now = std::time::Instant::now();
            if now.duration_since(pressed_at) >= INITIAL_DELAY
                && now.duration_since(last_fire) >= REPEAT_INTERVAL
                && self.apply_nav_action(&action)
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
                    let action = self.action_for_launcher_nav(*button);
                    match action.as_deref() {
                        Some(a @ ("LeftFlipper" | "RightFlipper" | "LeftMagna" | "RightMagna"))
                            if self.apply_nav_action(a) =>
                        {
                            let now = std::time::Instant::now();
                            self.nav_held = Some((*button, a.to_string(), now, now));
                        }
                        Some("Start") | Some("LaunchBall") => {
                            let path = self.tables[self.selected_table].path.clone();
                            self.launch_table(&path);
                        }
                        Some("ExitGame") => {
                            self.quit_launcher(ui.ctx());
                        }
                        _ => {}
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
                        // Release cursor lock so VPX gets the mouse. Focus is
                        // released naturally because the kiosk focus-reclaim
                        // loop is gated on !vpx_running. VPX windows then
                        // z-order on top of PinReady.
                        ctx.set_cursor_lock(false);
                    }
                    VpxStatus::ExitOk => {
                        self.vpx_loading_msg.clear();
                        self.vpx_loading_pct = None;
                        self.vpx_hide_covers = false;
                        self.vpx_status_rx = None;
                        self.restore_kiosk_after_vpx(ctx);
                        return;
                    }
                    VpxStatus::ExitError(log) => {
                        self.vpx_loading_msg.clear();
                        self.vpx_hide_covers = false;
                        self.vpx_error_log = Some(log);
                        self.vpx_status_rx = None;
                        self.restore_kiosk_after_vpx(ctx);
                        return;
                    }
                    VpxStatus::LaunchError(msg) => {
                        self.vpx_loading_msg.clear();
                        self.vpx_hide_covers = false;
                        self.vpx_error_log = Some(msg);
                        self.vpx_status_rx = None;
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
