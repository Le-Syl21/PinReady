use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::audio::{self, AudioCommand, AudioConfig, Sound3DMode};
use crate::config::VpxConfig;
use crate::db::Database;
use crate::i18n::{self, LANGUAGE_OPTIONS};
use crate::inputs::{self, CapturedInput, InputAction, JoystickEvent};
use crate::screens::{self, DisplayInfo, DisplayRole};
use crate::tilt::TiltConfig;
use crate::updater::{self, ReleaseInfo, UpdateProgress};
use rust_i18n::t;

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Wizard,
    Launcher,
}

/// VPX process status messages sent from the launch thread
enum VpxStatus {
    /// Loading progress message (e.g. "Initializing Visuals... 10%")
    Loading(String),
    /// VPX has finished loading ("Startup done")
    Started,
    /// VPX exited normally
    ExitOk,
    /// VPX exited with error — contains captured stderr/stdout log
    ExitError(String),
    /// Failed to launch VPX
    LaunchError(String),
}

/// Viewport ID for the backglass window
const BG_VIEWPORT: &str = "backglass_viewport";
/// Viewport ID for the playfield cover window
const PF_VIEWPORT: &str = "playfield_viewport";
/// Viewport ID for the topper cover window
const TOPPER_VIEWPORT: &str = "topper_viewport";
/// VPX logo bytes (embedded at compile time)
const VPX_LOGO: &[u8] = include_bytes!("../assets/vpinball_logo.png");

/// A discovered table
#[derive(Debug, Clone)]
pub struct TableEntry {
    pub path: std::path::PathBuf,
    pub name: String,
    pub has_directb2s: bool,
    pub bg_path: Option<std::path::PathBuf>,
    pub bg_bytes: Option<std::sync::Arc<[u8]>>,
}

/// Wizard pages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardPage {
    Screens,
    Rendering,
    Inputs,
    Tilt,
    Audio,
    TablesDir,
}

impl WizardPage {
    fn title(&self) -> String {
        match self {
            Self::Screens => t!("page_screens"),
            Self::Rendering => t!("page_rendering"),
            Self::Inputs => t!("page_inputs"),
            Self::Tilt => t!("page_tilt"),
            Self::Audio => t!("page_audio"),
            Self::TablesDir => t!("page_tables"),
        }
        .to_string()
    }

    fn index(&self) -> usize {
        match self {
            Self::Screens => 0,
            Self::Rendering => 1,
            Self::Inputs => 2,
            Self::Tilt => 3,
            Self::Audio => 4,
            Self::TablesDir => 5,
        }
    }

    fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Self::Screens),
            1 => Some(Self::Rendering),
            2 => Some(Self::Inputs),
            3 => Some(Self::Tilt),
            4 => Some(Self::Audio),
            5 => Some(Self::TablesDir),
            _ => None,
        }
    }

    fn count() -> usize {
        6
    }
}

/// How the VPinballX executable is provided
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VpxInstallMode {
    /// Download from GitHub fork release
    Auto,
    /// User provides the path manually
    Manual,
}

/// State for input capture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureState {
    Idle,
    /// Waiting for input for action at this index
    Capturing(usize),
}

pub struct App {
    mode: AppMode,
    page: WizardPage,
    config: VpxConfig,
    db: Database,

    // VPinballX executable path
    vpx_exe_path: String,

    // Page 1 — Screens
    displays: Vec<DisplayInfo>,
    screen_count: usize,
    view_mode: i32, // 0=Desktop, 1=Cabinet, 2=FSS
    disable_touch: bool,

    // Cabinet physical dimensions (cm) for Window projection mode
    screen_inclination: f32, // Playfield screen angle, 0 = horizontal
    lockbar_width: f32,      // Lockbar width in cm
    lockbar_height: f32,     // Lockbar height from ground in cm
    player_x: f32,           // Player X offset from center in cm
    player_y: f32,           // Player Y distance from lockbar in cm (negative = behind)
    player_z: f32,           // Player Z height (eyes) from playfield in cm
    player_height: f32,      // Player total height in cm (used to compute Z)

    // Page 2 — Rendering
    aa_factor: f32,     // Supersampling 0.5–2.0 (default 1.0)
    msaa: i32,          // 0=Off, 1=4x, 2=6x, 3=8x
    fxaa: i32,          // 0=Off, 1–7 various modes
    sharpen: i32,       // 0=Off, 1=CAS, 2=Bilateral CAS
    pf_reflection: i32, // 0–5 reflection quality
    max_tex_dim: i32,   // 512–16384
    sync_mode: i32,     // 0=No sync, 1=VSync
    max_framerate: f32, // -1=display, 0=unlimited, else value

    // Live accelerometer data from joystick thread
    accel_x: f32,
    accel_y: f32,

    // Page 3 — Inputs
    actions: Vec<InputAction>,
    capture_state: CaptureState,
    show_advanced_inputs: bool,
    joystick_rx: Option<crossbeam_channel::Receiver<JoystickEvent>>,
    pinscape_id: Option<String>, // VPX device ID if Pinscape detected
    gamepad_id: Option<String>,  // VPX device ID if generic gamepad detected
    use_gamepad: bool,           // User toggle: use gamepad axes for flippers/nudge/plunger

    // Page 3 — Tilt
    tilt: TiltConfig,

    // Page 4 — Audio
    audio: AudioConfig,
    audio_cmd_tx: Option<crossbeam_channel::Sender<AudioCommand>>,

    // Page 5 — Tables dir
    tables_dir: String,

    // Launcher
    tables: Vec<TableEntry>,
    table_filter: String,
    table_filter_lower: String, // cached lowercase version of table_filter
    selected_table: usize,
    scroll_to_selected: bool, // set by joystick navigation to trigger scroll
    launcher_cols: usize,     // number of columns in the grid (computed in render)
    images_preloaded: bool,
    bg_rx: Option<crossbeam_channel::Receiver<(usize, std::path::PathBuf)>>,

    // VPX process running — disables launcher while true
    vpx_running: Arc<AtomicBool>,
    // VPX launch status received from the VPX process thread
    vpx_status_rx: Option<crossbeam_channel::Receiver<VpxStatus>>,
    vpx_loading_msg: String,
    vpx_hide_covers: bool,         // VPX windows are up, hide covers
    vpx_error_log: Option<String>, // set on unexpected exit, shown as popup

    // Autostart on boot
    autostart: bool,

    // Quit timer (set after finalize_wizard)
    quit_after_ms: Option<std::time::Instant>,

    // Rescan button: long-press (3s) = full regeneration, short click = incremental
    rescan_press_start: Option<std::time::Instant>,

    // Language
    selected_language: usize,

    // VPX updater
    vpx_install_mode: VpxInstallMode,
    vpx_fork_repo: String,
    vpx_installed_tag: String,
    vpx_latest_release: Option<ReleaseInfo>,
    update_check_rx: Option<crossbeam_channel::Receiver<anyhow::Result<ReleaseInfo>>>,
    update_progress_rx: Option<crossbeam_channel::Receiver<UpdateProgress>>,
    update_downloading: bool,
    update_progress: (u64, u64), // (current, total)
    update_error: Option<String>,
}

impl App {
    pub fn new(config: VpxConfig, db: Database, start_in_wizard: bool) -> Self {
        // Enumerate displays
        let displays = screens::enumerate_displays();
        let screen_count = displays.len().min(4);

        // Auto-set cabinet mode for 2+ screens
        let view_mode = if screen_count >= 2 { 1 } else { 0 };
        let disable_touch = config
            .get_i32("Player", "NumberOfTimesToShowTouchMessage")
            .unwrap_or(10)
            == 0;

        // Cabinet physical dimensions
        let screen_inclination = config.get_f32("Player", "ScreenInclination").unwrap_or(0.0);
        let lockbar_width = config.get_f32("Player", "LockbarWidth").unwrap_or(70.0);
        let lockbar_height = config.get_f32("Player", "LockbarHeight").unwrap_or(85.0);
        let player_x = config.get_f32("Player", "ScreenPlayerX").unwrap_or(0.0);
        let player_y = config.get_f32("Player", "ScreenPlayerY").unwrap_or(-10.0);
        let player_z = config.get_f32("Player", "ScreenPlayerZ").unwrap_or(70.0);
        // Reverse-compute player height from Z + lockbar_height + 12cm (eyes-to-top-of-head)
        let player_height = player_z + lockbar_height + 12.0;

        // Load rendering config (pre-fill from ini)
        let aa_factor = config.get_f32("Player", "AAFactor").unwrap_or(1.0);
        let msaa = config.get_i32("Player", "MSAASamples").unwrap_or(0);
        let fxaa = config.get_i32("Player", "FXAA").unwrap_or(0);
        let sharpen = config.get_i32("Player", "Sharpen").unwrap_or(0);
        let pf_reflection = config.get_i32("Player", "PFReflection").unwrap_or(5);
        let max_tex_dim = config.get_i32("Player", "MaxTexDimension").unwrap_or(16384);
        let sync_mode = config.get_i32("Player", "SyncMode").unwrap_or(0);
        let max_framerate = config.get_f32("Player", "MaxFramerate").unwrap_or(-1.0);

        // Load input actions with defaults
        let mut actions = inputs::default_actions();
        // Pre-fill from existing config
        log::info!("Loading input mappings from ini...");
        for action in &mut actions {
            if let Some(mapping_str) = config.get_input_mapping(action.setting_id) {
                if mapping_str.is_empty() {
                    continue;
                }
                log::info!("  {} = {}", action.setting_id, mapping_str);
                // Take first alternative (before |), first combo part (before &)
                let first = mapping_str
                    .split('|')
                    .next()
                    .unwrap_or("")
                    .split('&')
                    .next()
                    .unwrap_or("")
                    .trim();
                if let Some(sc_str) = first.strip_prefix("Key;") {
                    // Keyboard mapping: "Key;<scancode>"
                    if let Ok(sc_val) = sc_str.parse::<i32>() {
                        let scancode = sdl3_sys::everything::SDL_Scancode(sc_val);
                        action.mapping = Some(CapturedInput::Keyboard {
                            scancode,
                            name: inputs::scancode_name(scancode),
                        });
                    }
                } else if let Some(pos) = first.find(';') {
                    // Joystick mapping: "SDLJoy_<id>;<button>[;extra_params...]"
                    let device_id = first[..pos].to_string();
                    let rest = &first[pos + 1..];
                    // Button ID is the first number after the semicolon
                    if let Ok(button) = rest.split(';').next().unwrap_or("").parse::<u8>() {
                        action.mapping = Some(CapturedInput::JoystickButton {
                            device_id: device_id.clone(),
                            button,
                            name: format!("{} Button {}", device_id, button),
                        });
                    }
                }
            }
        }

        // Load tilt config
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&config);

        // Load audio config
        let mut audio = AudioConfig::default();
        audio.load_from_config(&config);
        audio.available_devices = AudioConfig::enumerate_devices();

        // Load VPinballX executable path and updater config
        let vpx_exe_path = db.get_config("vpx_exe_path").unwrap_or_default();
        let vpx_fork_repo = db
            .get_config("vpx_fork_repo")
            .unwrap_or_else(|| updater::DEFAULT_FORK_REPO.to_string());
        let vpx_installed_tag = db.get_config("vpx_installed_tag").unwrap_or_default();
        let vpx_install_mode = if db.get_config("vpx_install_mode").as_deref() == Some("manual") {
            VpxInstallMode::Manual
        } else {
            VpxInstallMode::Auto
        };

        // Load tables directory
        let tables_dir = db.get_tables_dir().unwrap_or_default();

        // Start joystick thread (keyboard captured via egui)
        let joystick_rx = inputs::spawn_joystick_thread();

        let audio_cmd_tx = audio::spawn_audio_thread();

        // Language detection: from DB, or system, or default English
        let selected_language = if let Some(saved_lang) = db.get_config("language") {
            LANGUAGE_OPTIONS
                .iter()
                .position(|(c, _)| *c == saved_lang)
                .unwrap_or_else(i18n::detect_system_language)
        } else {
            i18n::detect_system_language()
        };
        let (lang_code, _) = LANGUAGE_OPTIONS[selected_language];
        i18n::set_locale(lang_code);
        log::info!(
            "Language: {} ({})",
            lang_code,
            LANGUAGE_OPTIONS[selected_language].1
        );

        // Spawn background update check (throttled to once per hour)
        let update_check_rx = {
            let last_check = db.get_config("vpx_last_check").unwrap_or_default();
            let should_check = last_check.parse::<i64>().map_or(true, |ts| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                now - ts > 3600
            });
            if should_check && !vpx_fork_repo.is_empty() {
                let repo = vpx_fork_repo.clone();
                log::info!("Checking for VPinballX updates from {repo}...");
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let result = updater::check_latest_release(&repo);
                    let _ = tx.send(result);
                });
                Some(rx)
            } else {
                None
            }
        };

        let mut s = Self {
            mode: if start_in_wizard {
                AppMode::Wizard
            } else {
                AppMode::Launcher
            },
            page: WizardPage::Screens,
            config,
            db,
            vpx_exe_path,
            displays,
            screen_count,
            view_mode,
            disable_touch,
            screen_inclination,
            lockbar_width,
            lockbar_height,
            player_x,
            player_y,
            player_z,
            player_height,
            actions,
            accel_x: 0.0,
            accel_y: 0.0,
            aa_factor,
            msaa,
            fxaa,
            sharpen,
            pf_reflection,
            max_tex_dim,
            sync_mode,
            max_framerate,
            capture_state: CaptureState::Idle,
            show_advanced_inputs: false,
            joystick_rx: Some(joystick_rx),
            pinscape_id: None,
            gamepad_id: None,
            use_gamepad: false,
            tilt,
            audio,
            audio_cmd_tx: Some(audio_cmd_tx),
            tables_dir,
            tables: Vec::new(),
            table_filter: String::new(),
            table_filter_lower: String::new(),
            selected_table: 0,
            scroll_to_selected: false,
            launcher_cols: 1,
            images_preloaded: false,
            bg_rx: None,
            vpx_running: Arc::new(AtomicBool::new(false)),
            vpx_status_rx: None,
            vpx_loading_msg: String::new(),
            vpx_hide_covers: false,
            vpx_error_log: None,
            autostart: is_autostart_enabled(),
            quit_after_ms: None,
            rescan_press_start: None,
            selected_language,
            vpx_install_mode,
            vpx_fork_repo,
            vpx_installed_tag,
            vpx_latest_release: None,
            update_check_rx,
            update_progress_rx: None,
            update_downloading: false,
            update_progress: (0, 0),
            update_error: None,
        };
        // Pre-scan tables if starting in launcher mode
        if !start_in_wizard {
            s.scan_tables();
        }
        s
    }

    fn next_page(&mut self) {
        let next = self.page.index() + 1;
        if let Some(page) = WizardPage::from_index(next) {
            self.save_current_page();
            self.page = page;
        }
    }

    fn prev_page(&mut self) {
        if self.page.index() > 0 {
            if let Some(page) = WizardPage::from_index(self.page.index() - 1) {
                self.page = page;
            }
        }
    }

    fn reset_current_page(&mut self) {
        match self.page {
            WizardPage::Screens => {
                self.view_mode = if self.screen_count >= 2 { 1 } else { 0 };
                self.screen_inclination = 0.0;
                self.lockbar_width = 70.0;
                self.lockbar_height = 85.0;
                self.player_x = 0.0;
                self.player_y = -10.0;
                self.player_height = 167.0;
                self.player_z = (self.player_height - 12.0 - self.lockbar_height).max(0.0);
            }
            WizardPage::Rendering => {
                self.aa_factor = 1.0;
                self.msaa = 0;
                self.fxaa = 0;
                self.sharpen = 0;
                self.pf_reflection = 5;
                self.max_tex_dim = 16384;
                self.sync_mode = 0;
                self.max_framerate = -1.0;
            }
            WizardPage::Inputs => {
                self.actions = crate::inputs::default_actions();
                self.capture_state = CaptureState::Idle;
                self.use_gamepad = false;
            }
            WizardPage::Tilt => {
                self.tilt = TiltConfig::default();
            }
            WizardPage::Audio => {
                self.audio = AudioConfig::default();
                self.audio.available_devices = AudioConfig::enumerate_devices();
            }
            WizardPage::TablesDir => {
                self.tables_dir = String::new();
            }
        }
    }

    fn save_current_page(&mut self) {
        match self.page {
            WizardPage::Screens => self.save_screens(),
            WizardPage::Rendering => self.save_rendering(),
            WizardPage::Inputs => self.save_inputs(),
            WizardPage::Tilt => self.save_tilt(),
            WizardPage::Audio => self.save_audio(),
            WizardPage::TablesDir => self.save_tables_dir(),
        }
    }

    fn save_screens(&mut self) {
        // Save VPX install mode and path
        let mode_str = match self.vpx_install_mode {
            VpxInstallMode::Auto => "auto",
            VpxInstallMode::Manual => "manual",
        };
        let _ = self.db.set_config("vpx_install_mode", mode_str);
        let _ = self.db.set_config("vpx_fork_repo", &self.vpx_fork_repo);
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
            self.config.set_i32("Backglass", "BackglassOutput", 0); // Disabled
        }
        if !has_dmd {
            self.config.set_i32("ScoreView", "ScoreViewOutput", 0); // Disabled
                                                                    // DMD overlaid on backglass with auto-position
            if has_backglass {
                self.config
                    .set_i32("Plugin.B2SLegacy", "BackglassDMDOverlay", 1);
                self.config
                    .set_i32("Plugin.B2SLegacy", "BackglassDMDAutoPos", 1);
            }
        } else {
            // DMD has its own screen — hide grill from backglass, enable DMD overlay on ScoreView
            self.config.set_i32("Plugin.B2SLegacy", "B2SHideGrill", 1);
            self.config
                .set_i32("Plugin.B2SLegacy", "ScoreViewDMDOverlay", 1);
            self.config
                .set_i32("Plugin.B2SLegacy", "ScoreViewDMDAutoPos", 1);
            self.config
                .set_i32("Plugin.B2SLegacy", "BackglassDMDOverlay", 0);
            // B2SLegacyDMD must win over ScoreView to keep the B2S frame around the DMD
            self.config
                .set_i32("ScoreView", "Priority.B2SLegacyDMD", 10);
            self.config.set_i32("ScoreView", "Priority.ScoreView", 1);
        }
        if !has_topper {
            self.config.set_i32("Topper", "TopperOutput", 0); // Disabled
        }

        // PlayfieldFullScreen is required for correct multi-screen positioning
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
                    // Write exact refresh rate — VPX crashes if value doesn't match exactly
                    self.config
                        .set_f32("Player", "PlayfieldRefreshRate", display.refresh_rate);
                    self.config
                        .set_f32("Player", "MaxFramerate", display.refresh_rate);
                    // Physical screen size in cm for Window projection (mm -> cm, width > height)
                    let w_cm = display.width_mm as f32 / 10.0;
                    let h_cm = display.height_mm as f32 / 10.0;
                    if w_cm > 0.0 && h_cm > 0.0 {
                        // VPX expects width > height (landscape dimensions)
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

    fn save_rendering(&mut self) {
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

    fn save_inputs(&mut self) {
        // If Pinscape detected, write device declaration + analog mappings
        // This replaces VPX's "Apply Device Layout?" dialog
        if let Some(psc_id) = &self.pinscape_id {
            let psc_id = psc_id.clone();
            self.config.set("Input", "Devices", "");
            self.config.set("Input", "Device.Key.Type", "");
            self.config.set("Input", "Device.Key.NoAutoLayout", "");
            self.config.set("Input", "Device.Key.Name", "");
            self.config.set("Input", "Device.Mouse.Type", "");
            self.config.set("Input", "Device.Mouse.NoAutoLayout", "");
            self.config
                .set("Input", &format!("Device.{psc_id}.Type"), "");
            self.config
                .set("Input", &format!("Device.{psc_id}.NoAutoLayout"), "1");
            self.config
                .set("Input", &format!("Device.{psc_id}.Name"), "");
            // Plunger: axis 0x0202=514, Position mode
            self.config.set(
                "Input",
                "Mapping.PlungerPos",
                &format!("{psc_id};514;P;0.000000;1.000000;1.000000"),
            );
            // Nudge: axes 0x0200=512 / 0x0201=513, Acceleration mode
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
        }

        // If gamepad detected, control whether VPX should auto-configure it
        if let Some(gp_id) = &self.gamepad_id {
            let gp_id = gp_id.clone();
            self.config
                .set("Input", &format!("Device.{gp_id}.Type"), "");
            self.config
                .set("Input", &format!("Device.{gp_id}.Name"), "");
            if self.use_gamepad {
                // NoAutoLayout absent or 0 → VPX will propose its gamepad layout on first launch
                self.config
                    .set("Input", &format!("Device.{gp_id}.NoAutoLayout"), "");
            } else {
                // NoAutoLayout = 1 → VPX won't ask to apply gamepad layout
                self.config
                    .set("Input", &format!("Device.{gp_id}.NoAutoLayout"), "1");
            }
        }

        // Write digital mappings
        for action in &self.actions {
            let mapping = match &action.mapping {
                Some(captured) => captured.to_mapping_string(),
                None => {
                    if action.default_scancode == sdl3_sys::everything::SDL_SCANCODE_UNKNOWN {
                        continue; // No default, no mapping
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

    fn save_tilt(&mut self) {
        self.tilt.save_to_config(&mut self.config);
        if let Err(e) = self.config.save() {
            log::error!("Failed to save config: {e}");
        }
    }

    fn save_audio(&mut self) {
        self.audio.save_to_config(&mut self.config);
        if let Err(e) = self.config.save() {
            log::error!("Failed to save config: {e}");
        }
    }

    fn save_tables_dir(&mut self) {
        if let Err(e) = self.db.set_tables_dir(&self.tables_dir) {
            log::error!("Failed to save tables dir: {e}");
        }
    }

    fn finalize_wizard(&mut self, _ctx: &egui::Context) {
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

    fn scan_tables(&mut self) {
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

    fn launch_table(&mut self, table_path: &std::path::Path) {
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

    fn process_bg_extraction(&mut self, ctx: &egui::Context) {
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

    fn preload_images(&mut self, ctx: &egui::Context) {
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
    fn action_for_button(&self, button: u8) -> Option<String> {
        for action in &self.actions {
            if let Some(inputs::CapturedInput::JoystickButton { button: b, .. }) = &action.mapping {
                if *b == button {
                    return Some(action.setting_id.to_string());
                }
            }
        }
        None
    }

    fn handle_launcher_joystick(&mut self, ui: &mut egui::Ui) {
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

    fn process_vpx_status(&mut self) {
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

    fn process_update_check(&mut self) {
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

    fn start_vpx_download(&mut self, release: &ReleaseInfo) {
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

    #[allow(deprecated)]
    fn render_launcher(&mut self, ui: &mut egui::Ui) {
        // Install image loaders once
        egui_extras::install_image_loaders(ui.ctx());

        self.process_bg_extraction(ui.ctx());
        self.preload_images(ui.ctx());
        self.handle_launcher_joystick(ui);
        self.process_vpx_status();
        self.process_update_check();
        // Only repaint when needed: bg extraction in progress, VPX running, joystick connected, or update in progress
        if self.bg_rx.is_some()
            || self.vpx_running.load(Ordering::Relaxed)
            || self.joystick_rx.is_some()
            || self.update_downloading
            || self.update_check_rx.is_some()
        {
            ui.ctx().request_repaint();
        }

        // Keyboard navigation in launcher
        if !self.tables.is_empty() && !self.vpx_running.load(Ordering::Relaxed) {
            let len = self.tables.len();
            let cols = self.launcher_cols.max(1);
            ui.input(|i| {
                for event in &i.events {
                    if let egui::Event::Key {
                        key, pressed: true, ..
                    } = event
                    {
                        match key {
                            egui::Key::ArrowLeft => {
                                self.selected_table = if self.selected_table > 0 {
                                    self.selected_table - 1
                                } else {
                                    len - 1
                                };
                                self.scroll_to_selected = true;
                            }
                            egui::Key::ArrowRight => {
                                self.selected_table = (self.selected_table + 1) % len;
                                self.scroll_to_selected = true;
                            }
                            egui::Key::ArrowUp => {
                                self.selected_table = if self.selected_table >= cols {
                                    self.selected_table - cols
                                } else {
                                    self.selected_table
                                };
                                self.scroll_to_selected = true;
                            }
                            egui::Key::ArrowDown => {
                                self.selected_table = if self.selected_table + cols < len {
                                    self.selected_table + cols
                                } else {
                                    self.selected_table
                                };
                                self.scroll_to_selected = true;
                            }
                            egui::Key::Enter => {
                                let path = self.tables[self.selected_table].path.clone();
                                self.launch_table(&path);
                            }
                            egui::Key::Escape => {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            _ => {}
                        }
                    }
                }
            });
        }

        // Multi-screen: position main window (table selector) on the right display
        // 2 screens: selector on Playfield, BG viewport on Backglass
        // 3+ screens: selector on DMD, BG viewport on Backglass, cover on Playfield (+Topper)
        let has_dmd = self.displays.iter().any(|d| d.role == DisplayRole::Dmd);
        let has_bg = self
            .displays
            .iter()
            .any(|d| d.role == DisplayRole::Backglass);
        if has_dmd {
            // 3+ screens: main window on DMD
            if let Some(dmd) = self.displays.iter().find(|d| d.role == DisplayRole::Dmd) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        dmd.x as f32,
                        dmd.y as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        dmd.width as f32,
                        dmd.height as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            }
        } else if has_bg {
            // 2 screens (PF + BG): main window on Playfield
            if let Some(pf) = self
                .displays
                .iter()
                .find(|d| d.role == DisplayRole::Playfield)
            {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        pf.x as f32,
                        pf.y as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        pf.width as f32,
                        pf.height as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            }
        }

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("launcher_title"))
                    .size(24.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("launcher_quit")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button(t!("launcher_config")).clicked() {
                    self.mode = AppMode::Wizard;
                }
                // Update button — visible when a new release is available
                if self.update_downloading {
                    let (current, total) = self.update_progress;
                    if total > 0 {
                        let pct = (current as f32 / total as f32 * 100.0) as u32;
                        let mb = current / (1024 * 1024);
                        let total_mb = total / (1024 * 1024);
                        ui.add(
                            egui::ProgressBar::new(current as f32 / total as f32).text(t!(
                                "update_progress",
                                mb = mb,
                                total = total_mb,
                                pct = pct
                            )),
                        );
                    } else {
                        ui.spinner();
                        ui.label(t!("update_extracting"));
                    }
                } else if let Some(release) = self.vpx_latest_release.clone() {
                    let btn = ui.button(
                        egui::RichText::new(t!("update_button", tag = release.tag.as_str()))
                            .color(egui::Color32::from_rgb(100, 200, 100)),
                    );
                    if btn.clicked() {
                        self.start_vpx_download(&release);
                    }
                }
                if let Some(ref err) = self.update_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 100, 100),
                        t!("update_error", msg = err.as_str()),
                    );
                }
                let rescan_label = if let Some(start) = self.rescan_press_start {
                    let held = start.elapsed().as_secs_f32();
                    let pct = ((held / 3.0) * 100.0).min(100.0) as u32;
                    t!("launcher_reset_pct", pct = pct).to_string()
                } else {
                    t!("launcher_rescan").to_string()
                };
                let rescan_btn = ui.button(&rescan_label);
                if rescan_btn.is_pointer_button_down_on() {
                    if self.rescan_press_start.is_none() {
                        self.rescan_press_start = Some(std::time::Instant::now());
                    }
                    if let Some(start) = self.rescan_press_start {
                        if start.elapsed().as_secs_f32() >= 3.0 {
                            // Long press: delete all caches and full rescan
                            log::info!("Long press: full backglass regeneration");
                            self.rescan_press_start = None;
                            let dir = std::path::Path::new(&self.tables_dir);
                            if dir.is_dir() {
                                for entry in walkdir::WalkDir::new(dir)
                                    .max_depth(2)
                                    .into_iter()
                                    .flatten()
                                {
                                    let p = entry.path();
                                    if p.file_name()
                                        .and_then(|f| f.to_str())
                                        .is_some_and(|f| f.starts_with(".pinready_bg_v"))
                                    {
                                        let _ = std::fs::remove_file(p);
                                    }
                                }
                            }
                            self.scan_tables();
                        } else {
                            ui.ctx().request_repaint();
                        }
                    }
                } else if self.rescan_press_start.is_some() {
                    // Released before 3s: incremental rescan
                    self.rescan_press_start = None;
                    self.scan_tables();
                }
            });
        });
        ui.add_space(8.0);

        // VPX loading overlay — show spinner but don't return, viewports need to render below
        let vpx_loading =
            self.vpx_running.load(Ordering::Relaxed) && !self.vpx_loading_msg.is_empty();
        if vpx_loading {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.spinner();
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(&self.vpx_loading_msg)
                        .size(18.0)
                        .strong(),
                );
            });
        }

        // VPX error popup
        if self.vpx_error_log.is_some() {
            let mut close = false;
            egui::Window::new(t!("launcher_error_title").to_string())
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_size([600.0, 400.0])
                .show(ui.ctx(), |ui| {
                    ui.label(
                        egui::RichText::new(t!("launcher_vpx_crashed").to_string())
                            .size(16.0)
                            .strong()
                            .color(egui::Color32::RED),
                    );
                    ui.add_space(8.0);
                    if let Some(ref log) = self.vpx_error_log {
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                ui.monospace(log);
                            });
                    }
                    ui.add_space(8.0);
                    if ui.button(t!("launcher_close").to_string()).clicked() {
                        close = true;
                    }
                });
            if close {
                self.vpx_error_log = None;
            }
        }

        // Search filter
        ui.horizontal(|ui| {
            ui.label(t!("launcher_search").to_string());
            if ui.text_edit_singleline(&mut self.table_filter).changed() {
                self.table_filter_lower = self.table_filter.to_lowercase();
            }
            ui.label(t!("launcher_table_count", count = self.tables.len()).to_string());
        });
        ui.add_space(8.0);

        if self.tables.is_empty() {
            ui.label(t!("launcher_no_tables").to_string());
            return;
        }

        // Table grid with backglass images
        let filter = &self.table_filter_lower;
        let mut launch_idx: Option<usize> = None;
        let card_width = 400.0;
        let card_height = 520.0;
        let img_height = 400.0;
        let card_spacing = 8.0;
        let available_width = ui.available_width();
        let cols = ((available_width / (card_width + card_spacing)) as usize).max(1);
        self.launcher_cols = cols;
        let row_height = card_height + card_spacing;

        let mut scroll_area = egui::ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);

        // Auto-scroll to selected table when navigating with joystick
        if self.scroll_to_selected {
            self.scroll_to_selected = false;
            // Compute the row of the selected table and scroll to it
            let selected_row = self.selected_table / cols;
            let target_y = selected_row as f32 * row_height;
            scroll_area = scroll_area.vertical_scroll_offset(target_y);
        }

        scroll_area.show(ui, |ui| {
            let filtered: Vec<usize> = (0..self.tables.len())
                .filter(|&i| {
                    filter.is_empty()
                        || self.tables[i].name.to_lowercase().contains(filter.as_str())
                })
                .collect();

            for row_start in (0..filtered.len()).step_by(cols) {
                // Center the row
                let row_count = (filtered.len() - row_start).min(cols);
                let row_width = row_count as f32 * (card_width + 8.0) - 8.0;
                let left_pad = ((available_width - row_width) / 2.0).max(0.0);

                ui.horizontal(|ui| {
                    ui.add_space(left_pad);
                    for col in 0..cols {
                        let fi = row_start + col;
                        if fi >= filtered.len() {
                            break;
                        }
                        let idx = filtered[fi];
                        let table = &self.tables[idx];

                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(card_width, card_height),
                            egui::Sense::click(),
                        );

                        if response.hovered() {
                            self.selected_table = idx;
                        }
                        if response.clicked() {
                            launch_idx = Some(idx);
                        }

                        let painter = ui.painter_at(rect);
                        let is_selected = idx == self.selected_table;

                        // Card background
                        let bg_color = if is_selected {
                            egui::Color32::from_rgb(60, 60, 90)
                        } else if response.hovered() {
                            egui::Color32::from_rgb(50, 50, 65)
                        } else {
                            egui::Color32::from_rgb(35, 35, 45)
                        };
                        painter.rect_filled(rect, 6.0, bg_color);

                        // Selection border (inside to avoid clipping by painter_at)
                        if is_selected {
                            painter.rect_stroke(
                                rect,
                                6.0,
                                egui::Stroke::new(4.0, egui::Color32::from_rgb(255, 200, 0)),
                                egui::StrokeKind::Inside,
                            );
                        }

                        // Backglass image (centered in image area)
                        let img_area = egui::Rect::from_min_size(
                            rect.min + egui::vec2(4.0, 4.0),
                            egui::vec2(card_width - 8.0, img_height - 8.0),
                        );
                        if table.bg_path.is_some() {
                            let uri = format!("bytes://bg/{idx}");
                            let img = egui::Image::new(uri)
                                .shrink_to_fit()
                                .corner_radius(egui::CornerRadius::same(4));
                            img.paint_at(ui, img_area);
                        } else {
                            // Placeholder
                            painter.rect_filled(img_area, 4.0, egui::Color32::from_rgb(25, 25, 30));
                            painter.text(
                                img_area.center(),
                                egui::Align2::CENTER_CENTER,
                                "Pas de backglass",
                                egui::FontId::proportional(18.0),
                                egui::Color32::GRAY,
                            );
                        }

                        // Table name (centered, bigger, bold)
                        let text_center = egui::pos2(
                            rect.center().x,
                            rect.min.y + img_height + (card_height - img_height) / 2.0,
                        );
                        painter.text(
                            text_center,
                            egui::Align2::CENTER_CENTER,
                            &table.name,
                            egui::FontId::new(24.0, egui::FontFamily::Proportional),
                            if is_selected {
                                egui::Color32::from_rgb(255, 200, 0)
                            } else {
                                egui::Color32::WHITE
                            },
                        );

                        // B2S badge
                        if !table.has_directb2s {
                            let badge_pos =
                                egui::pos2(rect.max.x - 12.0, rect.min.y + img_height + 6.0);
                            painter.text(
                                badge_pos,
                                egui::Align2::RIGHT_TOP,
                                "No B2S",
                                egui::FontId::proportional(14.0),
                                egui::Color32::from_rgb(255, 100, 100),
                            );
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });

        if let Some(idx) = launch_idx {
            self.selected_table = idx;
            let path = self.tables[idx].path.clone();
            self.launch_table(&path);
        }

        // 3+ screens: cover Playfield with grey metal background + VPX logo
        // (with 2 screens, Playfield hosts the table selector — no cover needed)
        let has_pf_display = self
            .displays
            .iter()
            .any(|d| d.role == DisplayRole::Playfield);
        if has_pf_display && has_dmd && !self.vpx_hide_covers {
            let pf_display = self
                .displays
                .iter()
                .find(|d| d.role == DisplayRole::Playfield);
            let (pf_x, pf_y, pf_w, pf_h) = pf_display
                .map(|d| (d.x as f32, d.y as f32, d.width as f32, d.height as f32))
                .unwrap_or((0.0, 0.0, 1920.0, 1080.0));

            let pf_viewport_id = egui::ViewportId::from_hash_of(PF_VIEWPORT);
            ui.ctx().show_viewport_deferred(
                pf_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("PinReady — Playfield")
                    .with_position(egui::pos2(pf_x, pf_y))
                    .with_inner_size(egui::vec2(pf_w, pf_h))
                    .with_decorations(false)
                    .with_override_redirect(true),
                move |ctx, _class| {
                    // Force position every frame — WM may ignore initial with_position
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        pf_x, pf_y,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(pf_w, pf_h)));
                    egui_extras::install_image_loaders(ctx);
                    ctx.include_bytes("bytes://vpx_logo", VPX_LOGO);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(80, 80, 85)))
                        .show(ctx, |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.add(
                                    egui::Image::new("bytes://vpx_logo")
                                        .max_size(egui::vec2(512.0, 512.0))
                                        .rotate(270.0_f32.to_radians(), egui::vec2(0.5, 0.5))
                                        .tint(egui::Color32::from_rgba_premultiplied(
                                            180, 180, 190, 200,
                                        )),
                                );
                            });
                        });
                },
            );
        }

        // If multi-screen, show backglass on BG display via secondary viewport
        let has_bg_display = self
            .displays
            .iter()
            .any(|d| d.role == DisplayRole::Backglass);
        if has_bg_display && !self.tables.is_empty() && !self.vpx_hide_covers {
            let selected = self.selected_table.min(self.tables.len() - 1);
            let table_name = self.tables[selected].name.clone();
            let bg_bytes = self.tables[selected].bg_bytes.clone();
            let bg_display = self
                .displays
                .iter()
                .find(|d| d.role == DisplayRole::Backglass);
            let (bg_x, bg_y, bg_w, bg_h) = bg_display
                .map(|d| (d.x as f32, d.y as f32, d.width as f32, d.height as f32))
                .unwrap_or((0.0, 0.0, 1280.0, 1024.0));

            let bg_viewport_id = egui::ViewportId::from_hash_of(BG_VIEWPORT);
            ui.ctx().request_repaint_of(bg_viewport_id);
            ui.ctx().show_viewport_deferred(
                bg_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("PinReady — Backglass")
                    .with_position(egui::pos2(bg_x, bg_y))
                    .with_inner_size(egui::vec2(bg_w, bg_h))
                    .with_decorations(false)
                    .with_override_redirect(true),
                move |ctx, _class| {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        bg_x, bg_y,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(bg_w, bg_h)));
                    egui_extras::install_image_loaders(ctx);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                        .show(ctx, |ui| {
                            if let Some(ref bytes) = bg_bytes {
                                let uri = format!("bytes://viewport_bg/{selected}");
                                ctx.include_bytes(uri.clone(), bytes.clone());
                                ui.centered_and_justified(|ui| {
                                    ui.add(egui::Image::new(uri).shrink_to_fit());
                                });
                            } else {
                                ui.centered_and_justified(|ui| {
                                    ui.colored_label(
                                        egui::Color32::WHITE,
                                        egui::RichText::new(&table_name).size(32.0),
                                    );
                                });
                            }
                        });
                },
            );
        }

        // 3+ screens: cover Topper with grey metal background + VPX logo
        let has_topper_display = self.displays.iter().any(|d| d.role == DisplayRole::Topper);
        if has_topper_display && has_dmd && !self.vpx_hide_covers {
            let topper_display = self.displays.iter().find(|d| d.role == DisplayRole::Topper);
            let (tp_x, tp_y, tp_w, tp_h) = topper_display
                .map(|d| (d.x as f32, d.y as f32, d.width as f32, d.height as f32))
                .unwrap_or((0.0, 0.0, 1920.0, 1080.0));

            let tp_viewport_id = egui::ViewportId::from_hash_of(TOPPER_VIEWPORT);
            ui.ctx().show_viewport_deferred(
                tp_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("PinReady — Topper")
                    .with_position(egui::pos2(tp_x, tp_y))
                    .with_inner_size(egui::vec2(tp_w, tp_h))
                    .with_decorations(false)
                    .with_override_redirect(true),
                move |ctx, _class| {
                    // Force position every frame — WM may ignore initial with_position
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        tp_x, tp_y,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(tp_w, tp_h)));
                    egui_extras::install_image_loaders(ctx);
                    ctx.include_bytes("bytes://vpx_logo", VPX_LOGO);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(80, 80, 85)))
                        .show(ctx, |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.add(
                                    egui::Image::new("bytes://vpx_logo")
                                        .max_size(egui::vec2(512.0, 512.0))
                                        .tint(egui::Color32::from_rgba_premultiplied(
                                            180, 180, 190, 200,
                                        )),
                                );
                            });
                        });
                },
            );
        }
    }

    // --- Page rendering ---

    fn render_screens_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("screens_heading"));
        ui.add_space(8.0);

        // Language selector
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Langue / Language").size(14.0));
            ui.add_space(8.0);
            let prev_lang = self.selected_language;
            let current_label = LANGUAGE_OPTIONS
                .get(self.selected_language)
                .map(|(_, l)| *l)
                .unwrap_or("English");
            egui::ComboBox::from_id_salt("lang_combo")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (idx, (_code, label)) in LANGUAGE_OPTIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_language, idx, *label);
                    }
                });
            if self.selected_language != prev_lang {
                let (code, _) = LANGUAGE_OPTIONS[self.selected_language];
                i18n::set_locale(code);
                let _ = self.db.set_config("language", code);
            }
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // VPinballX installation
        self.process_update_check();
        ui.label(egui::RichText::new(t!("vpx_install_title")).strong());
        ui.add_space(4.0);

        ui.radio_value(
            &mut self.vpx_install_mode,
            VpxInstallMode::Auto,
            t!("vpx_auto_install"),
        );
        if self.vpx_install_mode == VpxInstallMode::Auto {
            ui.indent("auto_install", |ui| {
                if let Some(ref release) = self.vpx_latest_release {
                    ui.label(t!("vpx_version_available", tag = release.tag.as_str()));
                    let size_mb = release.asset_size / (1024 * 1024);
                    ui.label(t!(
                        "vpx_artifact_info",
                        name = release.asset_name.as_str(),
                        size = size_mb
                    ));
                } else if self.update_check_rx.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t!("vpx_checking"));
                    });
                    ui.ctx().request_repaint();
                } else if !self.vpx_installed_tag.is_empty() {
                    ui.label(t!(
                        "vpx_version_installed",
                        tag = self.vpx_installed_tag.as_str()
                    ));
                } else {
                    ui.label(t!("vpx_no_version"));
                }

                if self.update_downloading {
                    let (current, total) = self.update_progress;
                    if total > 0 {
                        let pct = current as f32 / total as f32;
                        let mb = current / (1024 * 1024);
                        let total_mb = total / (1024 * 1024);
                        ui.add(egui::ProgressBar::new(pct).text(format!("{mb}/{total_mb} Mo")));
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(t!("vpx_extracting"));
                        });
                    }
                    ui.ctx().request_repaint();
                } else if let Some(release) = self.vpx_latest_release.clone() {
                    if release.tag != self.vpx_installed_tag
                        && ui.button(t!("vpx_install_button")).clicked()
                    {
                        self.start_vpx_download(&release);
                    }
                }

                if let Some(ref err) = self.update_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 100, 100),
                        t!("vpx_install_error", msg = err.as_str()),
                    );
                }

                let install_dir = updater::default_install_dir();
                ui.label(
                    egui::RichText::new(t!(
                        "vpx_install_dir",
                        path = install_dir.display().to_string()
                    ))
                    .weak(),
                );
            });
        }

        ui.radio_value(
            &mut self.vpx_install_mode,
            VpxInstallMode::Manual,
            t!("vpx_manual_install"),
        );
        if self.vpx_install_mode == VpxInstallMode::Manual {
            ui.indent("manual_install", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.vpx_exe_path).desired_width(400.0));
                    if ui.button(t!("vpx_browse")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title(t!("vpx_file_picker"))
                            .pick_file()
                        {
                            self.vpx_exe_path = path.display().to_string();
                        }
                    }
                });
                let vpx_exists = std::path::Path::new(&self.vpx_exe_path).is_file();
                if !vpx_exists && !self.vpx_exe_path.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 100, 100),
                        format!("⚠ {}", t!("vpx_file_not_found")),
                    );
                }
            });
        }

        ui.add_space(4.0);
        ui.label(egui::RichText::new(t!("vpx_validated_on")).weak().italics());
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // Screen count selection
        ui.label(t!("screens_count_label"));
        ui.horizontal(|ui| {
            for n in 1..=4 {
                let label = match n {
                    1 => t!("screens_1"),
                    2 => t!("screens_2"),
                    3 => t!("screens_3"),
                    _ => t!("screens_4"),
                };
                if ui.radio_value(&mut self.screen_count, n, label).changed() {
                    // Re-assign roles based on new screen count
                    for (i, display) in self.displays.iter_mut().enumerate() {
                        let roles = [
                            DisplayRole::Playfield,
                            DisplayRole::Backglass,
                            DisplayRole::Dmd,
                            DisplayRole::Topper,
                        ];
                        display.role = if i < self.screen_count {
                            roles.get(i).copied().unwrap_or(DisplayRole::Unused)
                        } else {
                            DisplayRole::Unused
                        };
                    }
                }
            }
        });

        ui.add_space(8.0);

        // View mode
        ui.label(t!("screens_display_mode"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.view_mode, 0, "Desktop");
            ui.radio_value(&mut self.view_mode, 1, "Cabinet");
            ui.radio_value(&mut self.view_mode, 2, "Full Single Screen");
        });

        ui.add_space(8.0);

        ui.checkbox(&mut self.disable_touch, t!("screens_disable_touch"));

        ui.add_space(12.0);

        // Display table
        if self.displays.is_empty() {
            ui.label(t!("screens_no_displays"));
            return;
        }

        egui::Grid::new("displays_grid")
            .striped(true)
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.strong(t!("screens_col_screen"));
                ui.strong(t!("screens_col_resolution"));
                ui.strong(t!("screens_col_hz"));
                ui.strong(t!("screens_col_size"));
                ui.strong(t!("screens_col_role"));
                ui.end_row();

                let available_roles: Vec<DisplayRole> = DisplayRole::all()
                    .iter()
                    .copied()
                    .filter(|r| {
                        *r == DisplayRole::Unused
                            || match r {
                                DisplayRole::Playfield => self.screen_count >= 1,
                                DisplayRole::Backglass => self.screen_count >= 2,
                                DisplayRole::Dmd => self.screen_count >= 3,
                                DisplayRole::Topper => self.screen_count >= 4,
                                DisplayRole::Unused => true,
                            }
                    })
                    .collect();

                for display in &mut self.displays {
                    // Name + inches
                    let label = if let Some(inches) = display.size_inches {
                        format!("{} ({}\")", display.name, inches)
                    } else {
                        display.name.clone()
                    };
                    ui.label(&label);
                    ui.label(format!("{}x{}", display.width, display.height));
                    ui.label(format!("{:.0} Hz", display.refresh_rate));
                    // Physical size mm (editable)
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut display.width_mm)
                                .speed(1)
                                .suffix(" mm"),
                        );
                        ui.label("x");
                        ui.add(
                            egui::DragValue::new(&mut display.height_mm)
                                .speed(1)
                                .suffix(" mm"),
                        );
                    });

                    egui::ComboBox::from_id_salt(format!("role_{}", display.name))
                        .selected_text(display.role.label())
                        .show_ui(ui, |ui| {
                            for role in &available_roles {
                                ui.selectable_value(&mut display.role, *role, role.label());
                            }
                        });
                    ui.end_row();
                }
            });

        // Only show cabinet dimensions in Cabinet mode
        if self.view_mode == 1 {
            ui.add_space(16.0);
            ui.strong(t!("cabinet_dimensions"));
            ui.add_space(4.0);
            ui.label(t!("cabinet_drag_hint"));
            ui.add_space(8.0);

            // Layout: schema on the left, values on the right
            ui.horizontal(|ui| {
                // === Interactive cabinet schema (side view) ===
                let schema_size = egui::vec2(450.0, 500.0);
                let (rect, response) =
                    ui.allocate_exact_size(schema_size, egui::Sense::click_and_drag());
                let painter = ui.painter_at(rect);

                // Scale: 1cm = 2.0px
                let scale = 2.0_f32;
                // Ground line at bottom of the schema
                let ground_y = rect.bottom() - 25.0;
                // Cabinet front (lockbar) X position
                let cab_x = rect.left() + 220.0;

                // Colors
                let col_cab = egui::Color32::from_rgb(120, 80, 50); // brown cabinet
                let col_screen = egui::Color32::from_rgb(60, 120, 200); // blue screen
                let col_player = egui::Color32::from_rgb(80, 180, 80); // green player
                let col_dim = egui::Color32::from_rgb(200, 60, 60); // red dimensions
                let col_ground = egui::Color32::GRAY;
                let col_handle = egui::Color32::from_rgb(255, 200, 0); // yellow handles

                // Ground
                painter.line_segment(
                    [
                        egui::pos2(rect.left() + 10.0, ground_y),
                        egui::pos2(rect.right() - 10.0, ground_y),
                    ],
                    egui::Stroke::new(2.0, col_ground),
                );
                painter.text(
                    egui::pos2(rect.right() - 30.0, ground_y + 8.0),
                    egui::Align2::CENTER_CENTER,
                    "Sol",
                    egui::FontId::proportional(10.0),
                    col_ground,
                );

                // Cabinet body (side view: front=lockbar side, back=backglass side)
                let lockbar_y = ground_y - self.lockbar_height * scale;
                let font_label = egui::FontId::proportional(12.0);
                let font_dim = egui::FontId::proportional(11.0);

                // Screen (inclined from lockbar toward backglass)
                let screen_len_px = 150.0;
                let incl_rad = self.screen_inclination.to_radians();
                let screen_end_x = cab_x + screen_len_px * incl_rad.cos();
                let screen_end_y = lockbar_y - screen_len_px * incl_rad.sin();

                // Cabinet legs: front leg under lockbar, back leg under screen end
                let front_leg_x = cab_x;
                let back_leg_x = screen_end_x;
                painter.line_segment(
                    [
                        egui::pos2(front_leg_x, ground_y),
                        egui::pos2(front_leg_x, lockbar_y),
                    ],
                    egui::Stroke::new(3.0, col_cab),
                );
                painter.line_segment(
                    [
                        egui::pos2(back_leg_x, ground_y),
                        egui::pos2(back_leg_x, screen_end_y),
                    ],
                    egui::Stroke::new(3.0, col_cab),
                );

                // Lockbar (horizontal bar at front)
                painter.line_segment(
                    [
                        egui::pos2(cab_x - 15.0, lockbar_y),
                        egui::pos2(cab_x + 15.0, lockbar_y),
                    ],
                    egui::Stroke::new(5.0, col_cab),
                );
                painter.text(
                    egui::pos2(cab_x, lockbar_y + 12.0),
                    egui::Align2::CENTER_TOP,
                    "Lockbar",
                    font_label.clone(),
                    col_cab,
                );

                // Playfield screen (on top of the cab frame)
                painter.line_segment(
                    [
                        egui::pos2(cab_x, lockbar_y),
                        egui::pos2(screen_end_x, screen_end_y),
                    ],
                    egui::Stroke::new(6.0, col_screen),
                );
                painter.text(
                    egui::pos2(
                        (cab_x + screen_end_x) / 2.0,
                        (lockbar_y + screen_end_y) / 2.0 - 14.0,
                    ),
                    egui::Align2::CENTER_CENTER,
                    "Playfield",
                    font_label.clone(),
                    col_screen,
                );

                // Backglass (vertical from end of screen)
                let bg_height = 80.0;
                painter.line_segment(
                    [
                        egui::pos2(screen_end_x, screen_end_y),
                        egui::pos2(screen_end_x, screen_end_y - bg_height),
                    ],
                    egui::Stroke::new(4.0, col_screen.linear_multiply(0.6)),
                );
                painter.text(
                    egui::pos2(screen_end_x + 8.0, screen_end_y - bg_height / 2.0),
                    egui::Align2::LEFT_CENTER,
                    "BG",
                    font_label,
                    col_screen,
                );

                // Player (stick figure, side view — facing right toward cabinet)
                let player_base_x = cab_x - (-self.player_y) * scale;
                let player_feet_y = ground_y;
                let player_head_y = ground_y - self.player_height * scale;
                let player_hip_y = ground_y - self.player_height * scale * 0.45;
                let player_shoulder_y = ground_y - self.player_height * scale * 0.72;
                let player_neck_y = ground_y - self.player_height * scale * 0.82;
                let head_radius = 10.0;
                let head_center_y = player_head_y + head_radius;
                let leg_spread = 14.0; // feet spread front/back for side view
                let stroke = egui::Stroke::new(3.0, col_player);

                // Legs (spread front/back, side view)
                let front_foot_x = player_base_x + leg_spread;
                let back_foot_x = player_base_x - leg_spread;
                // Front leg
                painter.line_segment(
                    [
                        egui::pos2(front_foot_x, player_feet_y),
                        egui::pos2(player_base_x + 2.0, player_hip_y),
                    ],
                    stroke,
                );
                // Back leg
                painter.line_segment(
                    [
                        egui::pos2(back_foot_x, player_feet_y),
                        egui::pos2(player_base_x - 2.0, player_hip_y),
                    ],
                    stroke,
                );
                // Torso (hip to neck, slight lean forward)
                painter.line_segment(
                    [
                        egui::pos2(player_base_x, player_hip_y),
                        egui::pos2(player_base_x + 3.0, player_neck_y),
                    ],
                    stroke,
                );
                // Head
                painter.circle_filled(
                    egui::pos2(player_base_x + 3.0, head_center_y),
                    head_radius,
                    col_player,
                );
                // Eye (facing right toward cab)
                painter.circle_filled(
                    egui::pos2(player_base_x + 7.0, head_center_y - 2.0),
                    2.0,
                    egui::Color32::WHITE,
                );
                // Arms (reaching toward lockbar)
                let hand_x = player_base_x + 20.0; // hands forward toward cab
                let hand_y = player_shoulder_y + 15.0; // hands at lockbar height-ish
                painter.line_segment(
                    [
                        egui::pos2(player_base_x + 3.0, player_shoulder_y),
                        egui::pos2(hand_x, hand_y),
                    ],
                    stroke,
                );

                // === Dimension arrows ===

                // Lockbar height (sol -> lockbar)
                let arrow_x = cab_x + 50.0;
                painter.line_segment(
                    [
                        egui::pos2(arrow_x, ground_y),
                        egui::pos2(arrow_x, lockbar_y),
                    ],
                    egui::Stroke::new(1.5, col_dim),
                );
                painter.text(
                    egui::pos2(arrow_x + 5.0, (ground_y + lockbar_y) / 2.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{:.0} cm", self.lockbar_height),
                    font_dim.clone(),
                    col_dim,
                );

                // Player height
                let parrow_x = player_base_x - 25.0;
                painter.line_segment(
                    [
                        egui::pos2(parrow_x, player_feet_y),
                        egui::pos2(parrow_x, player_head_y),
                    ],
                    egui::Stroke::new(1.5, col_player),
                );
                painter.text(
                    egui::pos2(parrow_x - 5.0, (player_feet_y + player_head_y) / 2.0),
                    egui::Align2::RIGHT_CENTER,
                    format!("{:.0} cm", self.player_height),
                    font_dim.clone(),
                    col_player,
                );

                // Player Y distance
                painter.line_segment(
                    [
                        egui::pos2(player_base_x, lockbar_y + 12.0),
                        egui::pos2(cab_x, lockbar_y + 12.0),
                    ],
                    egui::Stroke::new(1.0, col_dim),
                );
                painter.text(
                    egui::pos2((player_base_x + cab_x) / 2.0, lockbar_y + 24.0),
                    egui::Align2::CENTER_CENTER,
                    format!("Y={:.0} cm", self.player_y),
                    font_dim.clone(),
                    col_dim,
                );

                // Screen inclination arc
                if self.screen_inclination.abs() > 0.5 {
                    painter.text(
                        egui::pos2(cab_x + 30.0, lockbar_y - 12.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{:.0} deg", self.screen_inclination),
                        font_dim,
                        col_screen,
                    );
                }

                // === Drag handles ===
                let handle_radius = 6.0;

                // Handle: lockbar height (drag vertically on the lockbar)
                let h_lockbar = egui::pos2(cab_x, lockbar_y);
                painter.circle_filled(h_lockbar, handle_radius, col_handle);

                // Handle: player height (drag on head)
                let h_head = egui::pos2(player_base_x, player_head_y);
                painter.circle_filled(h_head, handle_radius, col_handle);

                // Handle: player Y (drag on waist)
                let h_waist = egui::pos2(player_base_x, player_hip_y);
                painter.circle_filled(h_waist, handle_radius, col_handle);

                // Handle: screen inclination (drag on screen end)
                let h_screen = egui::pos2(screen_end_x, screen_end_y);
                painter.circle_filled(h_screen, handle_radius, col_handle);

                // Handle dragging logic
                if response.dragged() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let dist =
                            |p: egui::Pos2| ((pos.x - p.x).powi(2) + (pos.y - p.y).powi(2)).sqrt();

                        if dist(h_lockbar) < 30.0 {
                            // Drag lockbar height
                            let new_h = (ground_y - pos.y) / scale;
                            self.lockbar_height = new_h.clamp(0.0, 250.0);
                        } else if dist(h_head) < 30.0 {
                            // Drag player height
                            let new_h = (ground_y - pos.y) / scale;
                            self.player_height = new_h.clamp(75.0, 250.0);
                        } else if dist(h_waist) < 30.0 {
                            // Drag player Y (horizontal movement)
                            let new_y = -(cab_x - pos.x) / scale;
                            self.player_y = new_y.clamp(-70.0, 30.0);
                        } else if dist(h_screen) < 30.0 {
                            // Drag screen inclination
                            let dx = pos.x - cab_x;
                            let dy = lockbar_y - pos.y;
                            let angle = dy.atan2(dx).to_degrees();
                            self.screen_inclination = angle.clamp(-30.0, 30.0);
                        }
                    }
                }

                // Recompute player_z from height
                self.player_z = (self.player_height - 12.0 - self.lockbar_height).max(0.0);

                // === Values panel on the right ===
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.strong(t!("cabinet_values"));
                    ui.add_space(8.0);

                    ui.label(t!("cabinet_lockbar_width"));
                    ui.add(
                        egui::DragValue::new(&mut self.lockbar_width)
                            .range(10.0..=150.0)
                            .speed(1.0)
                            .suffix(" cm"),
                    );
                    ui.add_space(4.0);

                    ui.label(t!("cabinet_lockbar_height"));
                    ui.add(
                        egui::DragValue::new(&mut self.lockbar_height)
                            .range(0.0..=250.0)
                            .speed(1.0)
                            .suffix(" cm"),
                    );
                    ui.add_space(4.0);

                    ui.label(t!("cabinet_screen_inclination"));
                    ui.add(
                        egui::DragValue::new(&mut self.screen_inclination)
                            .range(-30.0..=30.0)
                            .speed(0.5)
                            .suffix(" deg"),
                    );
                    ui.add_space(4.0);

                    ui.label(t!("cabinet_player_height"));
                    ui.add(
                        egui::DragValue::new(&mut self.player_height)
                            .range(75.0..=250.0)
                            .speed(1.0)
                            .suffix(" cm"),
                    );
                    ui.add_space(4.0);

                    ui.label(t!("cabinet_player_distance"));
                    ui.add(
                        egui::DragValue::new(&mut self.player_y)
                            .range(-70.0..=30.0)
                            .speed(1.0)
                            .suffix(" cm"),
                    );
                    ui.add_space(4.0);

                    ui.label(t!("cabinet_player_offset"));
                    ui.add(
                        egui::DragValue::new(&mut self.player_x)
                            .range(-30.0..=30.0)
                            .speed(1.0)
                            .suffix(" cm"),
                    );
                    ui.add_space(12.0);

                    ui.separator();
                    ui.label(t!(
                        "cabinet_eye_height",
                        value = format!("{:.0}", self.player_z)
                    ));
                    ui.label(t!("cabinet_eye_formula"));
                });
            });
        }
    }

    fn render_rendering_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("rendering_heading"));
        ui.add_space(4.0);
        ui.label(t!("rendering_desc"));
        ui.add_space(12.0);

        egui::Grid::new("rendering_grid")
            .min_col_width(250.0)
            .striped(true)
            .show(ui, |ui| {
                // Sync mode
                ui.label(t!("rendering_sync"));
                egui::ComboBox::from_id_salt("sync_mode")
                    .selected_text(match self.sync_mode {
                        0 => t!("rendering_sync_none").to_string(),
                        1 => t!("rendering_sync_vsync").to_string(),
                        _ => t!("rendering_sync_vsync").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.sync_mode,
                            0,
                            t!("rendering_sync_none").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.sync_mode,
                            1,
                            t!("rendering_sync_vsync_default").to_string(),
                        );
                    });
                ui.end_row();

                // Max framerate — auto-set from playfield refresh rate
                ui.label(t!("rendering_fps_limit"));
                let pf_refresh = self
                    .displays
                    .iter()
                    .find(|d| d.role == DisplayRole::Playfield)
                    .map(|d| d.refresh_rate)
                    .unwrap_or(60.0);
                self.max_framerate = pf_refresh;
                ui.label(t!("rendering_fps_info", hz = format!("{:.2}", pf_refresh)));
                ui.end_row();

                // Supersampling
                ui.label(t!("rendering_supersampling"));
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.aa_factor, 0.5..=2.0).step_by(0.25));
                    let tip = if self.aa_factor < 0.8 {
                        t!("rendering_aa_perf")
                    } else if self.aa_factor <= 1.1 {
                        t!("rendering_aa_default")
                    } else if self.aa_factor <= 1.5 {
                        t!("rendering_aa_quality")
                    } else {
                        t!("rendering_aa_quality_heavy")
                    };
                    ui.label(tip.to_string());
                });
                ui.end_row();

                // MSAA
                ui.label(t!("rendering_msaa"));
                egui::ComboBox::from_id_salt("msaa")
                    .selected_text(match self.msaa {
                        0 => t!("rendering_msaa_off").to_string(),
                        1 => t!("rendering_msaa_4").to_string(),
                        2 => t!("rendering_msaa_6").to_string(),
                        3 => t!("rendering_msaa_8").to_string(),
                        _ => t!("rendering_msaa_off").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.msaa,
                            0,
                            t!("rendering_msaa_off_default").to_string(),
                        );
                        ui.selectable_value(&mut self.msaa, 1, t!("rendering_msaa_4").to_string());
                        ui.selectable_value(&mut self.msaa, 2, t!("rendering_msaa_6").to_string());
                        ui.selectable_value(&mut self.msaa, 3, t!("rendering_msaa_8").to_string());
                    });
                ui.end_row();

                // Post-process AA
                ui.label(t!("rendering_fxaa"));
                egui::ComboBox::from_id_salt("fxaa")
                    .selected_text(match self.fxaa {
                        0 => t!("rendering_fxaa_off").to_string(),
                        1 => t!("rendering_fxaa_fast").to_string(),
                        2 => t!("rendering_fxaa_standard").to_string(),
                        3 => t!("rendering_fxaa_quality").to_string(),
                        4 => t!("rendering_fxaa_nfaa").to_string(),
                        5 => t!("rendering_fxaa_dlaa").to_string(),
                        6 => t!("rendering_fxaa_smaa").to_string(),
                        7 => t!("rendering_fxaa_faaa").to_string(),
                        _ => t!("rendering_fxaa_off").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.fxaa,
                            0,
                            t!("rendering_fxaa_off").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            1,
                            t!("rendering_fxaa_fast").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            2,
                            t!("rendering_fxaa_standard").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            3,
                            t!("rendering_fxaa_quality").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            4,
                            t!("rendering_fxaa_nfaa").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            5,
                            t!("rendering_fxaa_dlaa").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            6,
                            t!("rendering_fxaa_smaa").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            7,
                            t!("rendering_fxaa_faaa").to_string(),
                        );
                    });
                ui.end_row();

                // Sharpening
                ui.label(t!("rendering_sharpen"));
                egui::ComboBox::from_id_salt("sharpen")
                    .selected_text(match self.sharpen {
                        0 => t!("rendering_sharpen_off").to_string(),
                        1 => t!("rendering_sharpen_cas").to_string(),
                        2 => t!("rendering_sharpen_bilateral").to_string(),
                        _ => t!("rendering_sharpen_off").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.sharpen,
                            0,
                            t!("rendering_sharpen_off").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.sharpen,
                            1,
                            t!("rendering_sharpen_cas").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.sharpen,
                            2,
                            t!("rendering_sharpen_bilateral").to_string(),
                        );
                    });
                ui.end_row();

                // Reflections
                ui.label(t!("rendering_reflections"));
                egui::ComboBox::from_id_salt("pf_reflection")
                    .selected_text(match self.pf_reflection {
                        0 => t!("rendering_reflect_off").to_string(),
                        1 => t!("rendering_reflect_balls").to_string(),
                        2 => t!("rendering_reflect_static").to_string(),
                        3 => t!("rendering_reflect_static_balls").to_string(),
                        4 => t!("rendering_reflect_static_dynamic").to_string(),
                        5 => t!("rendering_reflect_dynamic").to_string(),
                        _ => t!("rendering_reflect_dynamic").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            0,
                            t!("rendering_reflect_off_perf").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            1,
                            t!("rendering_reflect_balls").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            2,
                            t!("rendering_reflect_static_zero").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            3,
                            t!("rendering_reflect_static_balls").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            4,
                            t!("rendering_reflect_static_dynamic").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            5,
                            t!("rendering_reflect_dynamic_default").to_string(),
                        );
                    });
                ui.end_row();

                // Max texture dimension
                ui.label(t!("rendering_tex_size"));
                egui::ComboBox::from_id_salt("max_tex")
                    .selected_text(self.max_tex_dim.to_string())
                    .show_ui(ui, |ui| {
                        for &size in &[512, 1024, 2048, 4096, 8192, 16384] {
                            let label = if size == 16384 {
                                t!("rendering_tex_default").to_string()
                            } else {
                                format!("{size}")
                            };
                            ui.selectable_value(&mut self.max_tex_dim, size, label);
                        }
                    });
                ui.end_row();
            });
    }

    fn render_inputs_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("inputs_heading"));
        ui.add_space(4.0);

        // Detected controllers info
        if self.pinscape_id.is_some() {
            ui.label(t!("inputs_pinscape").to_string());
        }
        if self.gamepad_id.is_some() {
            ui.checkbox(&mut self.use_gamepad, t!("inputs_gamepad").to_string());
        }

        ui.add_space(4.0);
        ui.label(t!("inputs_instructions").to_string());
        ui.add_space(8.0);

        // Process keyboard input via egui (has window focus)
        if let CaptureState::Capturing(idx) = self.capture_state {
            // Check for modifier-only presses (Shift, Ctrl, Alt)
            let modifiers = ui.input(|i| i.modifiers);
            let mut captured = false;

            // Check key events
            let key_events: Vec<(egui::Key, bool)> = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| {
                        if let egui::Event::Key { key, pressed, .. } = e {
                            Some((*key, *pressed))
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            for &(key, pressed) in &key_events {
                if pressed {
                    if key == egui::Key::Escape {
                        self.capture_state = CaptureState::Idle;
                        captured = true;
                        break;
                    }
                    if let Some(sc) = inputs::egui_key_to_scancode(key) {
                        if idx < self.actions.len() {
                            self.actions[idx].mapping = Some(CapturedInput::Keyboard {
                                scancode: sc,
                                name: inputs::scancode_name(sc),
                            });
                        }
                        self.capture_state = CaptureState::Idle;
                        captured = true;
                        break;
                    }
                }
            }

            // Check modifier-only press (e.g., just Shift pressed alone)
            if !captured && (modifiers.shift || modifiers.ctrl || modifiers.alt) {
                // Wait for a key event to pair with the modifier, or capture modifier alone
                // We only capture modifier alone if no other key event came through
                if key_events.is_empty() {
                    if let Some(sc) = inputs::egui_modifiers_to_scancode(&modifiers) {
                        if idx < self.actions.len() {
                            self.actions[idx].mapping = Some(CapturedInput::Keyboard {
                                scancode: sc,
                                name: inputs::scancode_name(sc),
                            });
                        }
                        self.capture_state = CaptureState::Idle;
                    }
                }
            }

            // Joystick events are processed in the main ui() method

            // Request repaint while capturing to stay responsive
            ui.ctx().request_repaint();
        }

        // Conflicts
        let conflicts = inputs::find_conflicts(&self.actions);

        // Essential actions
        ui.strong(t!("inputs_essential").to_string());
        self.render_action_list(ui, true, &conflicts);

        ui.add_space(8.0);
        ui.checkbox(
            &mut self.show_advanced_inputs,
            t!("inputs_show_advanced").to_string(),
        );
        if self.show_advanced_inputs {
            ui.add_space(4.0);
            ui.strong(t!("inputs_advanced").to_string());
            self.render_action_list(ui, false, &conflicts);
        }
    }

    fn render_action_list(
        &mut self,
        ui: &mut egui::Ui,
        essential: bool,
        conflicts: &[(usize, usize)],
    ) {
        egui::Grid::new(if essential {
            "essential_inputs"
        } else {
            "advanced_inputs"
        })
        .striped(true)
        .min_col_width(120.0)
        .show(ui, |ui| {
            ui.strong(t!("inputs_col_action").to_string());
            ui.strong(t!("inputs_col_binding").to_string());
            ui.strong("");
            ui.end_row();

            for (idx, action) in self.actions.iter().enumerate() {
                if action.essential != essential {
                    continue;
                }

                ui.label(action.label);

                // Current binding display
                let is_capturing = self.capture_state == CaptureState::Capturing(idx);
                let binding_text = if is_capturing {
                    t!("inputs_capturing").to_string()
                } else if let Some(captured) = &action.mapping {
                    captured.display_name().to_string()
                } else if action.default_scancode != sdl3_sys::everything::SDL_SCANCODE_UNKNOWN {
                    format!(
                        "{}{}",
                        inputs::scancode_name(action.default_scancode),
                        t!("inputs_default_suffix")
                    )
                } else {
                    t!("inputs_unassigned").to_string()
                };

                // Conflict warning
                let has_conflict = conflicts.iter().any(|(a, b)| *a == idx || *b == idx);
                if has_conflict {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 165, 0),
                        format!("/!\\ {binding_text}"),
                    );
                } else {
                    ui.label(&binding_text);
                }

                // Capture button
                let btn_label = if is_capturing {
                    t!("inputs_cancel").to_string()
                } else {
                    t!("inputs_map").to_string()
                };
                if ui.button(btn_label).clicked() {
                    if is_capturing {
                        self.capture_state = CaptureState::Idle;
                    } else {
                        self.capture_state = CaptureState::Capturing(idx);
                    }
                }
                ui.end_row();
            }
        });
    }

    fn render_tilt_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tilt_heading"));
        ui.add_space(4.0);
        ui.label(t!("tilt_desc"));
        ui.add_space(12.0);

        // Request repaint for live accelerometer data
        ui.ctx().request_repaint();

        // --- Nudge section ---
        ui.separator();
        ui.strong(t!("tilt_nudge"));
        ui.add_space(4.0);

        ui.checkbox(&mut self.tilt.nudge_filter, t!("tilt_noise_filter"));
        ui.add_space(4.0);

        ui.label(t!("tilt_sensitivity"));
        ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Slider::new(&mut self.tilt.nudge_scale, 0.1..=2.0)
                .custom_formatter(|v, _| format!("{:.1}x", v)),
        );
        ui.add_space(12.0);

        // --- Tilt section ---
        ui.separator();
        ui.strong(t!("tilt_section"));
        ui.add_space(4.0);

        ui.label(t!("tilt_threshold"));
        ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Slider::new(&mut self.tilt.plumb_threshold_angle, 5.0..=60.0)
                .suffix("°")
                .custom_formatter(|v, _| format!("{:.0}°", v)),
        );
        ui.add_space(8.0);

        // Single visualization circle: live accel dot + tilt threshold ring
        ui.label(t!("tilt_visualization"));
        ui.add_space(4.0);
        let viz_size = egui::vec2(240.0, 240.0);
        let (rect, _response) = ui.allocate_exact_size(viz_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let center = rect.center();
        let radius = 110.0;

        // Outer circle (max range)
        painter.circle_stroke(center, radius, egui::Stroke::new(2.0, egui::Color32::GRAY));
        // Cross hairs
        painter.line_segment(
            [
                center - egui::vec2(radius, 0.0),
                center + egui::vec2(radius, 0.0),
            ],
            egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
        );
        painter.line_segment(
            [
                center - egui::vec2(0.0, radius),
                center + egui::vec2(0.0, radius),
            ],
            egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
        );

        // TILT threshold ring (red)
        let threshold_radius = radius * (self.tilt.plumb_threshold_angle / 60.0);
        painter.circle_stroke(
            center,
            threshold_radius,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 80, 80)),
        );
        painter.text(
            center + egui::vec2(threshold_radius + 4.0, -10.0),
            egui::Align2::LEFT_CENTER,
            "TILT",
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(255, 80, 80),
        );

        // Live accelerometer dot — apply nudge_scale so slider changes are visible live
        let scale = self.tilt.nudge_scale * 8.0; // 8x base amplification for visibility
        let dot_x = center.x + (self.accel_x * scale).clamp(-1.0, 1.0) * radius;
        let dot_y = center.y + (self.accel_y * scale).clamp(-1.0, 1.0) * radius;
        let dot_pos = egui::pos2(dot_x, dot_y);
        let dist = ((dot_x - center.x).powi(2) + (dot_y - center.y).powi(2)).sqrt();
        let dot_color = if dist > threshold_radius {
            egui::Color32::from_rgb(255, 50, 50) // in TILT zone
        } else {
            egui::Color32::from_rgb(100, 220, 100) // safe
        };
        painter.circle_filled(dot_pos, 7.0, dot_color);
    }

    fn render_audio_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("audio_heading"));
        ui.add_space(8.0);

        // Device assignment
        ui.strong(t!("audio_device_assignment"));
        ui.add_space(4.0);

        egui::Grid::new("audio_devices")
            .min_col_width(150.0)
            .show(ui, |ui| {
                ui.label(t!("audio_backglass"));
                egui::ComboBox::from_id_salt("device_bg")
                    .selected_text(if self.audio.device_bg.is_empty() {
                        t!("audio_default_device").to_string()
                    } else {
                        self.audio.device_bg.clone()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.audio.device_bg,
                            String::new(),
                            t!("audio_default_device"),
                        );
                        for dev in &self.audio.available_devices {
                            ui.selectable_value(&mut self.audio.device_bg, dev.clone(), dev);
                        }
                    });
                ui.end_row();

                ui.label(t!("audio_playfield"));
                egui::ComboBox::from_id_salt("device_pf")
                    .selected_text(if self.audio.device_pf.is_empty() {
                        t!("audio_default_device").to_string()
                    } else {
                        self.audio.device_pf.clone()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.audio.device_pf,
                            String::new(),
                            t!("audio_default_device"),
                        );
                        for dev in &self.audio.available_devices {
                            ui.selectable_value(&mut self.audio.device_pf, dev.clone(), dev);
                        }
                    });
                ui.end_row();
            });

        ui.add_space(12.0);

        // Sound3D mode
        ui.strong(t!("audio_output_mode"));
        ui.add_space(4.0);
        for mode in Sound3DMode::all() {
            ui.radio_value(&mut self.audio.sound_3d_mode, *mode, mode.label());
        }

        // Wiring guide based on selected mode
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.strong(t!("audio_wiring_required"));
        ui.add_space(4.0);

        match self.audio.sound_3d_mode {
            Sound3DMode::FrontStereo | Sound3DMode::RearStereo => {
                ui.label(t!("audio_card_stereo"));
                ui.label(format!("  {}", t!("audio_wiring_stereo_front")));
                ui.label(format!("  {}", t!("audio_wiring_stereo_no_spatial")));
            }
            Sound3DMode::SurroundRearLockbar => {
                ui.label(t!("audio_card_51"));
                ui.label(format!("  {}", t!("audio_wiring_51_front_top")));
                ui.label(format!("  {}", t!("audio_wiring_51_rear_bottom")));
                ui.label(format!("  {}", t!("audio_wiring_51_center_sub")));
                ui.label(format!("  {}", t!("audio_wiring_51_bg_separate")));
            }
            Sound3DMode::SurroundFrontLockbar => {
                ui.label(t!("audio_card_51"));
                ui.label(format!("  {}", t!("audio_wiring_51_front_bottom")));
                ui.label(format!("  {}", t!("audio_wiring_51_rear_top")));
                ui.label(format!("  {}", t!("audio_wiring_51_center_sub")));
                ui.label(format!("  {}", t!("audio_wiring_51_bg_separate")));
            }
            Sound3DMode::SsfLegacy | Sound3DMode::SsfNew => {
                ui.label(t!("audio_card_71"));
                ui.add_space(4.0);
                egui::Grid::new("wiring_grid")
                    .striped(true)
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        ui.strong(t!("audio_wiring_col_output"));
                        ui.strong(t!("audio_wiring_col_channel"));
                        ui.strong(t!("audio_wiring_col_connection"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_green"));
                        ui.label(t!("audio_wiring_71_green_ch"));
                        ui.label(t!("audio_wiring_71_green_conn"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_black"));
                        ui.label(t!("audio_wiring_71_black_ch"));
                        ui.label(t!("audio_wiring_71_black_conn"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_grey"));
                        ui.label(t!("audio_wiring_71_grey_ch"));
                        ui.label(t!("audio_wiring_71_grey_conn"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_orange"));
                        ui.label(t!("audio_wiring_71_orange_ch"));
                        ui.label(t!("audio_wiring_71_orange_conn"));
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.label(t!("audio_wiring_note"));
            }
        }

        ui.add_space(12.0);

        // Volumes
        ui.strong(t!("audio_volumes"));
        ui.add_space(4.0);
        egui::Grid::new("audio_volumes")
            .min_col_width(150.0)
            .show(ui, |ui| {
                ui.label(t!("audio_music_volume"));
                ui.add(egui::Slider::new(&mut self.audio.music_volume, 0..=100).suffix("%"));
                ui.end_row();

                ui.label(t!("audio_sound_volume"));
                ui.add(egui::Slider::new(&mut self.audio.sound_volume, 0..=100).suffix("%"));
                ui.end_row();
            });

        ui.add_space(12.0);

        // === Tests audio ===
        ui.strong(t!("audio_test_music"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let music_label = if self.audio.music_looping {
                "[Stop]"
            } else {
                "[Play]"
            };
            if ui.button(music_label).clicked() {
                self.audio.music_looping = !self.audio.music_looping;
                if let Some(tx) = &self.audio_cmd_tx {
                    if self.audio.music_looping {
                        self.audio.music_pan = 0.0;
                        let _ = tx.send(AudioCommand::StartMusic {
                            path: "music.ogg".to_string(),
                        });
                    } else {
                        let _ = tx.send(AudioCommand::StopMusic);
                    }
                }
            }
            if self.audio.music_looping {
                ui.label(t!("audio_pan"));
                let pan_slider = egui::Slider::new(&mut self.audio.music_pan, -1.0..=1.0)
                    .custom_formatter(|v, _| {
                        if v < -0.8 {
                            t!("audio_pan_left").to_string()
                        } else if v <= 0.2 {
                            t!("audio_pan_center").to_string()
                        } else {
                            t!("audio_pan_right").to_string()
                        }
                    });
                let response = ui.add_sized([ui.available_width(), 20.0], pan_slider);
                if response.drag_stopped() || (response.changed() && !response.dragged()) {
                    if let Some(tx) = &self.audio_cmd_tx {
                        let _ = tx.send(AudioCommand::SetMusicPan {
                            pan: self.audio.music_pan,
                        });
                    }
                }
            }
        });

        ui.add_space(12.0);
        ui.strong(t!("audio_test_speakers"));
        ui.add_space(4.0);
        ui.label(t!("audio_speakers_hint"));
        ui.add_space(4.0);

        // 4 speaker buttons in a square layout + 2 ball tests in the middle
        let btn_w = 140.0;
        let btn_h = 30.0;
        let gap = 20.0;

        // Row 1: Top Left / Top Right
        ui.horizontal(|ui| {
            ui.add_space(gap);
            if ui
                .add_sized(
                    [btn_w, btn_h],
                    egui::Button::new(t!("audio_top_left").to_string()),
                )
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayOnSpeaker {
                        path: "ball_roll.ogg".to_string(),
                        target: audio::SpeakerTarget::TopLeft,
                    });
                }
            }
            ui.add_space(gap * 2.0);
            if ui
                .add_sized(
                    [btn_w, btn_h],
                    egui::Button::new(t!("audio_top_right").to_string()),
                )
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayOnSpeaker {
                        path: "ball_roll.ogg".to_string(),
                        target: audio::SpeakerTarget::TopRight,
                    });
                }
            }
        });

        // Row 2: Ball test buttons (centered)
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(gap + btn_w / 2.0);
            if ui
                .add_sized(
                    [btn_w + gap, btn_h],
                    egui::Button::new(t!("audio_ball_top_bottom").to_string()),
                )
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayBallSequence {
                        path: "ball_roll.ogg".to_string(),
                        from: audio::SpeakerTarget::TopBoth,
                        to: audio::SpeakerTarget::BottomBoth,
                        hold_start_ms: 1500,
                        fade_ms: 3000,
                        hold_end_ms: 1500,
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(gap + btn_w / 2.0);
            if ui
                .add_sized(
                    [btn_w + gap, btn_h],
                    egui::Button::new(t!("audio_ball_left_right").to_string()),
                )
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayBallSequence {
                        path: "ball_roll.ogg".to_string(),
                        from: audio::SpeakerTarget::LeftBoth,
                        to: audio::SpeakerTarget::RightBoth,
                        hold_start_ms: 1500,
                        fade_ms: 3000,
                        hold_end_ms: 1500,
                    });
                }
            }
        });
        ui.add_space(4.0);

        // Row 3: Bottom Left / Bottom Right
        ui.horizontal(|ui| {
            ui.add_space(gap);
            if ui
                .add_sized(
                    [btn_w, btn_h],
                    egui::Button::new(t!("audio_bottom_left").to_string()),
                )
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayOnSpeaker {
                        path: "ball_roll.ogg".to_string(),
                        target: audio::SpeakerTarget::BottomLeft,
                    });
                }
            }
            ui.add_space(gap * 2.0);
            if ui
                .add_sized(
                    [btn_w, btn_h],
                    egui::Button::new(t!("audio_bottom_right").to_string()),
                )
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayOnSpeaker {
                        path: "ball_roll.ogg".to_string(),
                        target: audio::SpeakerTarget::BottomRight,
                    });
                }
            }
        });
    }

    fn render_tables_dir_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tables_heading"));
        ui.add_space(4.0);
        ui.label(t!("tables_desc"));
        ui.add_space(8.0);

        ui.label(t!("tables_structure"));
        ui.add_space(4.0);
        ui.code(
            "Tables/
  Table_Name/
    Table_Name.vpx               <- table (required)
    Table_Name.directb2s         <- backglass (same name as .vpx)
    Table_Name.ini               <- per-table config (optional)
    pinmame/
      roms/rom_name.zip          <- PinMAME ROM
      nvram/rom_name.nv          <- save data
    altcolor/rom_name/            <- Serum/VNI colorization
    medias/                       <- frontend images/videos",
        );

        ui.add_space(8.0);
        ui.label(t!("tables_modifiable"));
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            ui.label(t!("tables_path"));
            ui.text_edit_singleline(&mut self.tables_dir);
            if ui.button(t!("tables_browse")).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(t!("tables_folder_picker"))
                    .pick_folder()
                {
                    self.tables_dir = path.to_string_lossy().into_owned();
                }
            }
        });

        if !self.tables_dir.is_empty() {
            let path = std::path::Path::new(&self.tables_dir);
            if path.is_dir() {
                let count = std::fs::read_dir(path)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_dir())
                            .count()
                    })
                    .unwrap_or(0);
                ui.add_space(8.0);
                ui.label(t!("tables_valid", count = count));
            } else {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::RED, t!("tables_invalid"));
            }
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.checkbox(&mut self.autostart, t!("autostart_label"));
        ui.label(egui::RichText::new(t!("autostart_hint")).weak());
    }
}

// ---------------------------------------------------------------------------
// Autostart — cross-platform: Linux (.desktop), macOS (LaunchAgent), Windows (Startup shortcut)
// ---------------------------------------------------------------------------

/// Path to the autostart entry for the current platform.
fn autostart_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        dirs_path("HOME", ".config/autostart/pinready.desktop")
    }
    #[cfg(target_os = "macos")]
    {
        dirs_path("HOME", "Library/LaunchAgents/com.pinready.plist")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(|appdata| {
            std::path::PathBuf::from(appdata)
                .join(r"Microsoft\Windows\Start Menu\Programs\Startup\PinReady.lnk")
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn dirs_path(env_var: &str, suffix: &str) -> Option<std::path::PathBuf> {
    std::env::var(env_var)
        .ok()
        .map(|home| std::path::PathBuf::from(home).join(suffix))
}

/// Check if autostart is currently enabled.
fn is_autostart_enabled() -> bool {
    autostart_path().is_some_and(|p| p.exists())
}

/// Enable or disable autostart.
fn set_autostart(enabled: bool) -> anyhow::Result<()> {
    let path = autostart_path()
        .ok_or_else(|| anyhow::anyhow!("Autostart not supported on this platform"))?;

    if !enabled {
        if path.exists() {
            std::fs::remove_file(&path)?;
            log::info!("Autostart disabled: removed {}", path.display());
        }
        return Ok(());
    }

    // Get the path to our own executable
    let exe = std::env::current_exe()?;
    let exe_str = exe.display().to_string();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(target_os = "linux")]
    {
        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=PinReady\n\
             Comment=VPinballX configurator and launcher\n\
             Exec={exe_str}\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
        std::fs::write(&path, content)?;
    }

    #[cfg(target_os = "macos")]
    {
        let content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
                 <key>Label</key>\n\
                 <string>com.pinready</string>\n\
                 <key>ProgramArguments</key>\n\
                 <array>\n\
                     <string>{exe_str}</string>\n\
                 </array>\n\
                 <key>RunAtLoad</key>\n\
                 <true/>\n\
             </dict>\n\
             </plist>\n"
        );
        std::fs::write(&path, content)?;
    }

    #[cfg(target_os = "windows")]
    {
        // Create a .lnk shortcut via PowerShell
        let ps_cmd = format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $s = $ws.CreateShortcut('{}'); \
             $s.TargetPath = '{}'; \
             $s.WorkingDirectory = '{}'; \
             $s.Save()",
            path.display(),
            exe_str,
            exe.parent().unwrap_or(&exe).display(),
        );
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_cmd])
            .output()?;
    }

    log::info!("Autostart enabled: {}", path.display());
    Ok(())
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Check quit timer (after knocker plays)
        if let Some(start) = self.quit_after_ms {
            if start.elapsed().as_millis() > 800 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            ui.ctx().request_repaint(); // keep ticking
        }

        // Route based on mode — joystick events are handled per-mode
        if self.mode == AppMode::Launcher {
            self.render_launcher(ui);
            return;
        }

        // === Wizard mode: process joystick events for tilt viz + input capture ===
        if let Some(rx) = &self.joystick_rx {
            while let Ok(event) = rx.try_recv() {
                match &event {
                    JoystickEvent::AccelUpdate { x, y } => {
                        self.accel_x = *x;
                        self.accel_y = *y;
                    }
                    JoystickEvent::ButtonDown {
                        device_id,
                        button,
                        name,
                    } => {
                        if let CaptureState::Capturing(idx) = self.capture_state {
                            if idx < self.actions.len() {
                                self.actions[idx].mapping = Some(CapturedInput::JoystickButton {
                                    device_id: device_id.clone(),
                                    button: *button,
                                    name: name.clone(),
                                });
                            }
                            self.capture_state = CaptureState::Idle;
                        }
                    }
                    JoystickEvent::AxisMotion { .. } => {}
                    JoystickEvent::PinscapeDetected { vpx_id } => {
                        log::info!("Pinscape detected in UI: {}", vpx_id);
                        self.pinscape_id = Some(vpx_id.clone());
                        // KL Shield V5 default mapping (Pinscape KL25Z)
                        let brain_defaults: &[(&str, u8)] = &[
                            // Flippers
                            ("LeftFlipper", 7),
                            ("RightFlipper", 8),
                            // Upper flippers: mapped to both Magna and Staged (same physical button)
                            ("LeftMagna", 2),
                            ("RightMagna", 3),
                            ("LeftStagedFlipper", 2),
                            ("RightStagedFlipper", 3),
                            // Game controls
                            ("Start", 0),
                            ("LaunchBall", 4),
                            ("ExitGame", 5),
                            ("ExtraBall", 6),
                            // Coin & service
                            ("Credit1", 12),
                            ("CoinDoor", 13),
                            ("Service1", 14),
                            ("Service2", 15),
                            ("Service3", 16),
                            ("Service4", 17),
                            // Volume
                            ("VolumeDown", 18),
                            ("VolumeUp", 19),
                        ];
                        for (action_id, button) in brain_defaults {
                            if let Some(action) =
                                self.actions.iter_mut().find(|a| a.setting_id == *action_id)
                            {
                                if action.mapping.is_none() {
                                    action.mapping = Some(CapturedInput::JoystickButton {
                                        device_id: vpx_id.clone(),
                                        button: *button,
                                        name: format!("{} Button {}", vpx_id, button),
                                    });
                                }
                            }
                        }
                    }
                    JoystickEvent::GamepadDetected { vpx_id, name } => {
                        log::info!("Gamepad detected in UI: {} ({})", name, vpx_id);
                        self.gamepad_id = Some(vpx_id.clone());
                    }
                }
            }
        }

        // === Wizard mode ===

        // Header
        egui::Panel::top("wizard_header").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("PinReady");
                ui.separator();
                for i in 0..WizardPage::count() {
                    let page = WizardPage::from_index(i).expect("WizardPage index within count()");
                    let is_current = page == self.page;
                    let label = format!("{}. {}", i + 1, page.title());
                    if is_current {
                        ui.strong(&label);
                    } else {
                        ui.label(&label);
                    }
                    if i < WizardPage::count() - 1 {
                        ui.label(">");
                    }
                }
            });
        });

        // Navigation footer
        egui::Panel::bottom("wizard_nav").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.page.index() > 0 && ui.button(t!("wizard_previous")).clicked() {
                    self.prev_page();
                }

                if ui.button(t!("wizard_reset")).clicked() {
                    self.reset_current_page();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.page.index() < WizardPage::count() - 1 {
                        if ui.button(t!("wizard_next")).clicked() {
                            self.next_page();
                        }
                    } else if ui.button(t!("wizard_finish")).clicked() {
                        self.finalize_wizard(ui.ctx());
                    }
                });
            });
            ui.add_space(4.0);
        });

        // Main content — no inner margin so scrollbar sticks to window edge
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin {
                    left: 8,
                    right: 0,
                    top: 8,
                    bottom: 8,
                }),
            )
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        ui.add_space(0.0); // ensure full width
                        let _ = ui.available_width(); // force layout to use full width
                        match self.page {
                            WizardPage::Screens => self.render_screens_page(ui),
                            WizardPage::Rendering => self.render_rendering_page(ui),
                            WizardPage::Inputs => self.render_inputs_page(ui),
                            WizardPage::Tilt => self.render_tilt_page(ui),
                            WizardPage::Audio => self.render_audio_page(ui),
                            WizardPage::TablesDir => self.render_tables_dir_page(ui),
                        }
                    });
            });
    }
}
