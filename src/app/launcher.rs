use super::*;

impl App {
    pub(super) fn finalize_wizard(&mut self, _ctx: &egui::Context) {
        // Save ALL pages
        self.save_screens();
        self.save_rendering();
        self.save_inputs();
        self.save_tilt();
        self.save_audio();
        self.save_tables_dir();

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
        self.tables
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
        if self.vpx_exe_path.is_empty() || !std::path::Path::new(&self.vpx_exe_path).is_file() {
            log::error!("VPinballX executable not found: {}", self.vpx_exe_path);
            return;
        }
        log::info!(
            "Launching: {} -Play {}",
            self.vpx_exe_path,
            table_path.display()
        );
        let exe = self.vpx_exe_path.clone();
        let path = table_path.to_path_buf();
        let running = self.vpx_running.clone();
        running.store(true, Ordering::Relaxed);
        self.vpx_loading_msg = "Lancement de VPinballX...".to_string();
        self.vpx_error_log = None;

        let (tx, rx) = crossbeam_channel::unbounded();
        self.vpx_status_rx = Some(rx);

        std::thread::spawn(move || {
            use std::io::BufRead;
            let child = std::process::Command::new(&exe)
                .arg("-Play")
                .arg(&path)
                .stdout(std::process::Stdio::piped())
                .spawn();
            match child {
                Ok(mut child) => {
                    log::info!("VPinballX launched, reading stdout...");
                    let stdout = child.stdout.take();
                    let mut log_lines: Vec<String> = Vec::new();
                    let mut startup_done = false;

                    // Read stdout — parse progress and startup markers
                    if let Some(so) = stdout {
                        let reader = std::io::BufReader::new(so);
                        for line in reader.lines().map_while(Result::ok) {
                            log::info!("[VPX] {}", line);
                            if line.contains("SetProgress") {
                                if let Some(start) = line.find("] ") {
                                    let msg = &line[start + 2..];
                                    let _ = tx.send(VpxStatus::Loading(msg.to_string()));
                                }
                            } else if line.contains("RenderStaticPrepass")
                                && line.contains("Reflection Probe")
                            {
                                let _ =
                                    tx.send(VpxStatus::Loading("Reflection Probe...".to_string()));
                            } else if line.contains("PluginLog") {
                                // Extract plugin name from "B2SLegacy: ..." or "VNI: ..."
                                if let Some(start) = line.rfind("] ") {
                                    let msg = &line[start + 2..];
                                    if let Some(colon) = msg.find(':') {
                                        let plugin = &msg[..colon];
                                        let _ = tx.send(VpxStatus::Loading(format!(
                                            "Plugin {plugin}..."
                                        )));
                                    }
                                }
                            } else if line.contains("Startup done") {
                                startup_done = true;
                                let _ = tx.send(VpxStatus::Started);
                            }
                            log_lines.push(line);
                        }
                    }

                    match child.wait() {
                        Ok(status) => {
                            log::info!("VPinballX exited with status: {status}");
                            if status.success() || startup_done {
                                let _ = tx.send(VpxStatus::ExitOk);
                            } else {
                                let tail: Vec<String> =
                                    log_lines.iter().rev().take(50).rev().cloned().collect();
                                let _ = tx.send(VpxStatus::ExitError(tail.join("\n")));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to wait for VPinballX: {e}");
                            let _ = tx.send(VpxStatus::ExitError(format!("Process error: {e}")));
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to launch VPinballX: {e}");
                    let _ = tx.send(VpxStatus::LaunchError(format!("{e}")));
                }
            }
            running.store(false, Ordering::Relaxed);
        });
    }

    pub(super) fn process_bg_extraction(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.bg_rx {
            while let Ok((idx, path)) = rx.try_recv() {
                if idx < self.tables.len() {
                    if let Ok(bytes) = std::fs::read(&path) {
                        let arc: std::sync::Arc<[u8]> =
                            std::sync::Arc::from(bytes.into_boxed_slice());
                        let uri = format!("bytes://bg/{idx}");
                        ctx.include_bytes(uri, arc.clone());
                        self.tables[idx].bg_bytes = Some(arc);
                    }
                    self.tables[idx].bg_path = Some(path);
                }
            }
        }
    }

    pub(super) fn preload_images(&mut self, ctx: &egui::Context) {
        if self.images_preloaded {
            return;
        }
        self.images_preloaded = true;
        for (idx, table) in self.tables.iter().enumerate() {
            if let Some(ref path) = table.bg_path {
                if let Ok(bytes) = std::fs::read(path) {
                    let uri = format!("bytes://bg/{idx}");
                    ctx.include_bytes(uri, bytes);
                }
            }
        }
        log::info!(
            "Preloaded {} images into RAM",
            self.tables.iter().filter(|t| t.bg_path.is_some()).count()
        );
    }

    /// Find which action is mapped to a given joystick button number.
    pub(super) fn action_for_button(&self, button: u8) -> Option<String> {
        for action in &self.actions {
            if let Some(inputs::CapturedInput::JoystickButton { button: b, .. }) = &action.mapping {
                if *b == button {
                    return Some(action.setting_id.to_string());
                }
            }
        }
        None
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

        let len = self.tables.len();
        let cols = self.launcher_cols.max(1);
        for event in events {
            match &event {
                JoystickEvent::ButtonDown { button, .. } => {
                    let action = self.action_for_button(*button);
                    match action.as_deref() {
                        Some("LeftFlipper") => {
                            if self.selected_table > 0 {
                                self.selected_table -= 1;
                            } else {
                                self.selected_table = len - 1;
                            }
                            self.scroll_to_selected = true;
                        }
                        Some("RightFlipper") => {
                            self.selected_table = (self.selected_table + 1) % len;
                            self.scroll_to_selected = true;
                        }
                        Some("LeftStagedFlipper") => {
                            if self.selected_table >= cols {
                                self.selected_table -= cols;
                            } else {
                                self.selected_table =
                                    (len - 1).min(self.selected_table + len - cols);
                            }
                            self.scroll_to_selected = true;
                        }
                        Some("RightStagedFlipper") => {
                            if self.selected_table + cols < len {
                                self.selected_table += cols;
                            } else {
                                self.selected_table %= cols;
                            }
                            self.scroll_to_selected = true;
                        }
                        Some("Start") => {
                            let path = self.tables[self.selected_table].path.clone();
                            self.launch_table(&path);
                        }
                        Some("LaunchBall") => {
                            self.mode = AppMode::Wizard;
                        }
                        Some("ExitGame") => {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        _ => {}
                    }
                }
                JoystickEvent::AccelUpdate { .. } => {}
                _ => {}
            }
        }
    }

    pub(super) fn process_vpx_status(&mut self) {
        if let Some(rx) = &self.vpx_status_rx {
            while let Ok(status) = rx.try_recv() {
                match status {
                    VpxStatus::Loading(msg) => {
                        self.vpx_loading_msg = msg;
                    }
                    VpxStatus::Started => {
                        self.vpx_loading_msg = "Startup done".to_string();
                        self.vpx_hide_covers = true;
                    }
                    VpxStatus::ExitOk => {
                        self.vpx_loading_msg.clear();
                        self.vpx_hide_covers = false;
                        self.vpx_status_rx = None;
                        return;
                    }
                    VpxStatus::ExitError(log) => {
                        self.vpx_loading_msg.clear();
                        self.vpx_hide_covers = false;
                        self.vpx_error_log = Some(log);
                        self.vpx_status_rx = None;
                        return;
                    }
                    VpxStatus::LaunchError(msg) => {
                        self.vpx_loading_msg.clear();
                        self.vpx_hide_covers = false;
                        self.vpx_error_log = Some(msg);
                        self.vpx_status_rx = None;
                        return;
                    }
                }
            }
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
                        // Update last check timestamp
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs().to_string())
                            .unwrap_or_default();
                        let _ = self.db.set_config("vpx_last_check", &now);
                        if release.tag != self.vpx_installed_tag {
                            self.vpx_latest_release = Some(release);
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
                        log::info!("VPinballX installed to: {}", path_str);
                        return;
                    }
                    UpdateProgress::Error(msg) => {
                        self.update_downloading = false;
                        self.update_error = Some(msg.clone());
                        self.update_progress_rx = None;
                        log::error!("VPinballX update failed: {}", msg);
                        return;
                    }
                }
            }
        }
    }

    pub(super) fn start_vpx_download(&mut self, release: &ReleaseInfo) {
        let install_dir = updater::default_install_dir();
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
