use super::*;

impl App {
    pub(super) fn save_current_page(&mut self) {
        match self.page {
            WizardPage::Screens => self.save_screens(),
            WizardPage::Rendering => self.save_rendering(),
            WizardPage::Inputs => self.save_inputs(),
            WizardPage::Tilt => self.save_tilt(),
            WizardPage::Audio => self.save_audio(),
            WizardPage::TablesDir => self.save_tables_dir(),
        }
    }

    pub(super) fn save_screens(&mut self) {
        // Save VPX install mode and path
        let mode_str = match self.vpx_install_mode {
            VpxInstallMode::Auto => "auto",
            VpxInstallMode::Manual => "manual",
        };
        let _ = self.db.set_config("vpx_install_mode", mode_str);
        let _ = self.db.set_config("vpx_fork_repo", &self.vpx_fork_repo);
        let _ = self.db.set_config("vpx_install_dir", &self.vpx_install_dir);
        let _ = self.db.set_config(
            "external_dmd",
            if self.external_dmd { "true" } else { "false" },
        );
        if let Err(e) = self.db.set_config("vpx_exe_path", &self.vpx_exe_path) {
            log::error!("Failed to save VPX exe path: {e}");
        }

        self.config.set_view_mode(self.view_mode);

        if self.disable_touch {
            self.config.set_i32("Player", "TouchOverlay", 0);
            self.config
                .set_i32("Player", "NumberOfTimesToShowTouchMessage", 0);
        }

        // Disable outputs that are not used based on screen count
        let has_backglass = self
            .displays
            .iter()
            .any(|d| d.role == DisplayRole::Backglass);
        let has_dmd = self.displays.iter().any(|d| d.role == DisplayRole::Dmd);
        let has_topper = self.displays.iter().any(|d| d.role == DisplayRole::Topper);

        if !has_backglass {
            self.config.set_i32("Backglass", "BackglassOutput", 0);
        }
        if !has_dmd {
            self.config.set_i32("ScoreView", "ScoreViewOutput", 0);
            if self.external_dmd {
                // External DMD device (ZeDMD, PinDMD...) — no overlay needed,
                // VPX auto-detects the device at runtime
                self.config
                    .set_i32("Plugin.B2SLegacy", "BackglassDMDOverlay", 0);
            } else if has_backglass {
                // No DMD screen, no external DMD — overlay DMD on backglass
                self.config
                    .set_i32("Plugin.B2SLegacy", "BackglassDMDOverlay", 1);
                self.config
                    .set_i32("Plugin.B2SLegacy", "BackglassDMDAutoPos", 1);
            }
        } else {
            self.config.set_i32("Plugin.B2SLegacy", "B2SHideGrill", 1);
            self.config
                .set_i32("Plugin.B2SLegacy", "ScoreViewDMDOverlay", 1);
            self.config
                .set_i32("Plugin.B2SLegacy", "ScoreViewDMDAutoPos", 1);
            self.config
                .set_i32("Plugin.B2SLegacy", "BackglassDMDOverlay", 0);
            self.config
                .set_i32("ScoreView", "Priority.B2SLegacyDMD", 10);
            self.config.set_i32("ScoreView", "Priority.ScoreView", 1);
        }
        if !has_topper {
            self.config.set_i32("Topper", "TopperOutput", 0);
        }

        if self.screen_count >= 2 {
            self.config.set_i32("Player", "PlayfieldFullScreen", 1);
        }

        for display in self.displays.iter() {
            match display.role {
                DisplayRole::Playfield => {
                    self.config.set_display(
                        "Player",
                        "Playfield",
                        &display.name,
                        display.width,
                        display.height,
                        false,
                    );
                    self.config
                        .set_f32("Player", "PlayfieldRefreshRate", display.refresh_rate);
                    self.config
                        .set_f32("Player", "MaxFramerate", display.refresh_rate);
                    let w_cm = display.width_mm as f32 / 10.0;
                    let h_cm = display.height_mm as f32 / 10.0;
                    if w_cm > 0.0 && h_cm > 0.0 {
                        let (screen_w, screen_h) = if w_cm >= h_cm {
                            (w_cm, h_cm)
                        } else {
                            (h_cm, w_cm)
                        };
                        self.config.set_f32("Player", "ScreenWidth", screen_w);
                        self.config.set_f32("Player", "ScreenHeight", screen_h);
                    }
                }
                DisplayRole::Backglass => {
                    self.config.set_display(
                        "Backglass",
                        "Backglass",
                        &display.name,
                        display.width,
                        display.height,
                        true,
                    );
                }
                DisplayRole::Dmd => {
                    self.config.set_display(
                        "ScoreView",
                        "ScoreView",
                        &display.name,
                        display.width,
                        display.height,
                        true,
                    );
                }
                DisplayRole::Topper => {
                    self.config.set_display(
                        "Topper",
                        "Topper",
                        &display.name,
                        display.width,
                        display.height,
                        true,
                    );
                }
                DisplayRole::Unused => {}
            }
        }

        // Cabinet physical dimensions
        self.config
            .set_f32("Player", "ScreenInclination", self.screen_inclination);
        self.config
            .set_f32("Player", "LockbarWidth", self.lockbar_width);
        self.config
            .set_f32("Player", "LockbarHeight", self.lockbar_height);
        self.config
            .set_f32("Player", "ScreenPlayerX", self.player_x);
        self.config
            .set_f32("Player", "ScreenPlayerY", self.player_y);
        self.config
            .set_f32("Player", "ScreenPlayerZ", self.player_z);

        if let Err(e) = self.config.save() {
            log::error!("Failed to save config: {e}");
        }
    }

    pub(super) fn save_rendering(&mut self) {
        self.config.set_f32("Player", "AAFactor", self.aa_factor);
        self.config.set_i32("Player", "MSAASamples", self.msaa);
        self.config.set_i32("Player", "FXAA", self.fxaa);
        self.config.set_i32("Player", "Sharpen", self.sharpen);
        self.config
            .set_i32("Player", "PFReflection", self.pf_reflection);
        self.config
            .set_i32("Player", "MaxTexDimension", self.max_tex_dim);
        self.config.set_i32("Player", "SyncMode", self.sync_mode);
        self.config
            .set_f32("Player", "MaxFramerate", self.max_framerate);
        if let Err(e) = self.config.save() {
            log::error!("Failed to save config: {e}");
        }
    }

    pub(super) fn save_inputs(&mut self) {
        let _ = self
            .db
            .set_config("pinscape_profile", &self.pinscape_profile.to_string());

        if let Some(psc_id) = &self.pinscape_id {
            let psc_id = psc_id.clone();
            // Register all known devices with proper types (matches VPX auto-mapping format)
            self.config
                .set("Input", "Devices", &format!("Key;Mouse;{psc_id}"));
            // Keyboard device
            self.config.set("Input", "Device.Key.Type", "1");
            self.config.set("Input", "Device.Key.NoAutoLayout", "");
            self.config.set("Input", "Device.Key.Name", "Keyboards");
            // Mouse device
            self.config.set("Input", "Device.Mouse.Type", "3");
            self.config.set("Input", "Device.Mouse.NoAutoLayout", "");
            self.config.set("Input", "Device.Mouse.Name", "Mouse");
            // Pinscape device — type 2 (joystick), NoAutoLayout = 1 (don't re-prompt)
            self.config
                .set("Input", &format!("Device.{psc_id}.Type"), "2");
            self.config
                .set("Input", &format!("Device.{psc_id}.NoAutoLayout"), "1");
            self.config.set(
                "Input",
                &format!("Device.{psc_id}.Name"),
                &format!("mjrnet Pinscape Controller {psc_id}"),
            );
            // Pinscape axis elements (matches VPX auto-mapping output)
            let elements = [
                "512;A;Nudge X Acceleration",
                "513;A;Nudge Y Acceleration",
                "514;A;Plunger Position",
                "515;A;Nudge X Velocity",
                "516;A;Nudge Y Velocity",
                "517;A;Plunger Velocity",
                "518;A;Axis #6",
                "519;A;Axis #7",
            ];
            for (i, elem) in elements.iter().enumerate() {
                self.config
                    .set("Input", &format!("Device.{psc_id}.Element{i}"), elem);
            }
            // Analog mappings — plunger + nudge
            self.config.set(
                "Input",
                "Mapping.PlungerPos",
                &format!("{psc_id};514;P;0.000000;1.000000;1.000000"),
            );
            self.config.set(
                "Input",
                "Mapping.NudgeX1",
                &format!(
                    "{psc_id};512;A;0.100000;{:.6};1.000000",
                    self.tilt.nudge_scale
                ),
            );
            self.config.set(
                "Input",
                "Mapping.NudgeY1",
                &format!(
                    "{psc_id};513;A;0.100000;{:.6};1.000000",
                    self.tilt.nudge_scale
                ),
            );
            // Empty entries to match VPX format (prevents VPX from re-prompting)
            self.config.set("Input", "Mapping.PlungerVel", "");
            self.config.set("Input", "Mapping.NudgeX2", "");
            self.config.set("Input", "Mapping.NudgeY2", "");
        }

        if let Some(gp_id) = &self.gamepad_id {
            let gp_id = gp_id.clone();
            self.config
                .set("Input", &format!("Device.{gp_id}.Type"), "2");
            self.config
                .set("Input", &format!("Device.{gp_id}.Name"), "Gamepad");
            if self.use_gamepad {
                self.config
                    .set("Input", &format!("Device.{gp_id}.NoAutoLayout"), "");
            } else {
                self.config
                    .set("Input", &format!("Device.{gp_id}.NoAutoLayout"), "1");
            }
            // Add gamepad to Devices list if not already there
            if let Some(devices) = self.config.get("Input", "Devices") {
                if !devices.contains(&gp_id) {
                    self.config
                        .set("Input", "Devices", &format!("{devices};{gp_id}"));
                }
            }
        }

        for action in &self.actions {
            let mapping = match &action.mapping {
                Some(captured) => captured.to_mapping_string(),
                None => {
                    if action.default_scancode == sdl3_sys::everything::SDL_SCANCODE_UNKNOWN {
                        continue;
                    }
                    format!("Key;{}", action.default_scancode.0)
                }
            };
            self.config.set_input_mapping(action.setting_id, &mapping);
        }

        if let Err(e) = self.config.save() {
            log::error!("Failed to save config: {e}");
        }
    }

    pub(super) fn save_tilt(&mut self) {
        self.tilt.save_to_config(&mut self.config);
        if let Err(e) = self.config.save() {
            log::error!("Failed to save config: {e}");
        }
    }

    pub(super) fn save_audio(&mut self) {
        self.audio.save_to_config(&mut self.config);
        if let Err(e) = self.config.save() {
            log::error!("Failed to save config: {e}");
        }
    }

    pub(super) fn save_tables_dir(&mut self) {
        if let Err(e) = self.db.set_tables_dir(&self.tables_dir) {
            log::error!("Failed to save tables dir: {e}");
        }
    }
}
