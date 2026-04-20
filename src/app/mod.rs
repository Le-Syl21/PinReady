use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::audio::{self, AudioCommand, AudioConfig, Sound3DMode};
use crate::config::VpxConfig;
use crate::db::Database;
use crate::i18n::{self, LANGUAGE_OPTIONS};
use crate::inputs::{self, pinscape_button_defaults, CapturedInput, InputAction, JoystickEvent};
use crate::screens::{DisplayInfo, DisplayRole};
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
    /// Loading progress message with optional percentage (0.0–1.0)
    Loading(String, Option<f32>),
    /// VPX has finished loading ("Startup done")
    Started,
    /// VPX exited normally
    ExitOk,
    /// VPX exited with error — contains captured stdout + stderr log
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
const VPX_LOGO: &[u8] = include_bytes!("../../assets/vpinball_logo.png");

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
    Outputs,
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
            Self::Outputs => t!("page_outputs"),
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
            Self::Outputs => 3,
            Self::Tilt => 4,
            Self::Audio => 5,
            Self::TablesDir => 6,
        }
    }

    fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Self::Screens),
            1 => Some(Self::Rendering),
            2 => Some(Self::Inputs),
            3 => Some(Self::Outputs),
            4 => Some(Self::Tilt),
            5 => Some(Self::Audio),
            6 => Some(Self::TablesDir),
            _ => None,
        }
    }

    fn count() -> usize {
        7
    }
}

/// How the Visual Pinball executable is provided
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

    // Visual Pinball executable path and install directory
    vpx_exe_path: String,
    vpx_install_dir: String,

    // Page 1 — Screens
    displays: Vec<DisplayInfo>,
    screen_count: usize,
    view_mode: i32, // 0=Desktop, 1=Cabinet, 2=FSS
    disable_touch: bool,
    external_dmd: bool, // ZeDMD, PinDMD, etc. — DMD handled by external device, not a screen

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
    pinscape_id: Option<String>, // VPX device ID if pinball controller detected
    pinscape_profile: usize,     // 0 = KL25Z, 1 = Pico, 2 = DudesCab
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
    // Kiosk mode: lock the cursor inside the PF window and center it on the grid.
    // Window placement itself is handled at creation via ViewportBuilder::with_monitor.
    kiosk_cursor: bool, // scale + lock the cursor, warp once window is mapped
    kiosk_cursor_warped: bool, // one-shot: warp cursor after window settles
    // Launcher joystick nav auto-repeat: track which nav button is held
    nav_held: Option<(u8, String, std::time::Instant, std::time::Instant)>,
    bg_rx: Option<crossbeam_channel::Receiver<(usize, std::path::PathBuf)>>,

    // VPX process running — disables launcher while true
    vpx_running: Arc<AtomicBool>,
    // VPX launch status received from the VPX process thread
    vpx_status_rx: Option<crossbeam_channel::Receiver<VpxStatus>>,
    vpx_loading_msg: String,
    vpx_loading_pct: Option<f32>, // loading progress 0.0–1.0, if parseable
    vpx_hide_covers: bool,        // VPX windows are up, hide covers
    vpx_error_log: Option<String>, // set on unexpected exit, shown as popup

    // Autostart on boot
    autostart: bool,

    // Quit timer (set after finalize_wizard)
    quit_after_ms: Option<std::time::Instant>,

    // Rescan button: long-press (3s) = full regeneration, short click = incremental
    rescan_press_start: Option<std::time::Instant>,
    // Rescan feedback flash: (timestamp, is_full_reset)
    rescan_flash: Option<(std::time::Instant, bool)>,

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
    pub fn new(
        config: VpxConfig,
        db: Database,
        start_in_wizard: bool,
        displays: Vec<DisplayInfo>,
    ) -> Self {
        let screen_count = displays.len().min(4);
        let view_mode = if screen_count >= 2 { 1 } else { 0 };
        let disable_touch = config
            .get_i32("Player", "NumberOfTimesToShowTouchMessage")
            .unwrap_or(10)
            == 0;

        let (
            screen_inclination,
            lockbar_width,
            lockbar_height,
            player_x,
            player_y,
            player_z,
            player_height,
        ) = Self::load_cabinet_dimensions(&config);
        let (aa_factor, msaa, fxaa, sharpen, pf_reflection, max_tex_dim, sync_mode, max_framerate) =
            Self::load_rendering_config(&config);
        let actions = Self::load_input_mappings(&config);

        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&config);

        let mut audio = AudioConfig::default();
        audio.load_from_config(&config);
        audio.available_devices = AudioConfig::enumerate_devices();

        let (vpx_exe_path, vpx_install_dir, vpx_fork_repo, vpx_installed_tag, vpx_install_mode) =
            Self::load_updater_config(&db);
        let tables_dir = db.get_tables_dir().unwrap_or_default();
        let external_dmd = db.get_config("external_dmd").as_deref() == Some("true");
        let pinscape_profile = db
            .get_config("pinscape_profile")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let joystick_rx = inputs::spawn_joystick_thread();
        let audio_cmd_tx = audio::spawn_audio_thread();
        let selected_language = Self::detect_language(&db);
        let update_check_rx = Self::spawn_update_check(&vpx_fork_repo);

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
            vpx_install_dir,
            displays,
            screen_count,
            view_mode,
            disable_touch,
            external_dmd,
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
            pinscape_profile,
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
            kiosk_cursor: false,
            kiosk_cursor_warped: false,
            nav_held: None,
            bg_rx: None,
            vpx_running: Arc::new(AtomicBool::new(false)),
            vpx_status_rx: None,
            vpx_loading_msg: String::new(),
            vpx_loading_pct: None,
            vpx_hide_covers: false,
            vpx_error_log: None,
            autostart: is_autostart_enabled(),
            quit_after_ms: None,
            rescan_press_start: None,
            rescan_flash: None,
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
        if !start_in_wizard {
            s.scan_tables();
        }
        s
    }

    fn load_cabinet_dimensions(config: &VpxConfig) -> (f32, f32, f32, f32, f32, f32, f32) {
        let screen_inclination = config.get_f32("Player", "ScreenInclination").unwrap_or(0.0);
        let lockbar_width = config.get_f32("Player", "LockbarWidth").unwrap_or(70.0);
        let lockbar_height = config.get_f32("Player", "LockbarHeight").unwrap_or(85.0);
        let player_x = config.get_f32("Player", "ScreenPlayerX").unwrap_or(0.0);
        let player_y = config.get_f32("Player", "ScreenPlayerY").unwrap_or(-10.0);
        let player_z = config.get_f32("Player", "ScreenPlayerZ").unwrap_or(70.0);
        let player_height = player_z + lockbar_height + 12.0;
        (
            screen_inclination,
            lockbar_width,
            lockbar_height,
            player_x,
            player_y,
            player_z,
            player_height,
        )
    }

    /// Enable kiosk cursor behavior: software-scaled cursor, locked inside the
    /// window, and warped to center once the window is mapped. Window placement
    /// is handled separately via `ViewportBuilder::with_monitor`.
    pub fn enable_kiosk_cursor(&mut self) {
        self.kiosk_cursor = true;
        self.kiosk_cursor_warped = false;
    }

    fn load_rendering_config(config: &VpxConfig) -> (f32, i32, i32, i32, i32, i32, i32, f32) {
        (
            config.get_f32("Player", "AAFactor").unwrap_or(1.0),
            config.get_i32("Player", "MSAASamples").unwrap_or(0),
            config.get_i32("Player", "FXAA").unwrap_or(0),
            config.get_i32("Player", "Sharpen").unwrap_or(0),
            config.get_i32("Player", "PFReflection").unwrap_or(5),
            config.get_i32("Player", "MaxTexDimension").unwrap_or(16384),
            config.get_i32("Player", "SyncMode").unwrap_or(0),
            config.get_f32("Player", "MaxFramerate").unwrap_or(-1.0),
        )
    }

    fn load_input_mappings(config: &VpxConfig) -> Vec<InputAction> {
        let mut actions = inputs::default_actions();
        log::info!("Loading input mappings from ini...");
        for action in &mut actions {
            if let Some(mapping_str) = config.get_input_mapping(action.setting_id) {
                if mapping_str.is_empty() {
                    continue;
                }
                log::info!("  {} = {}", action.setting_id, mapping_str);
                let first = mapping_str
                    .split('|')
                    .next()
                    .unwrap_or("")
                    .split('&')
                    .next()
                    .unwrap_or("")
                    .trim();
                if let Some(sc_str) = first.strip_prefix("Key;") {
                    if let Ok(sc_val) = sc_str.parse::<i32>() {
                        let scancode = sdl3_sys::everything::SDL_Scancode(sc_val);
                        action.mapping = Some(CapturedInput::Keyboard {
                            scancode,
                            name: inputs::scancode_name(scancode),
                        });
                    }
                } else if let Some(pos) = first.find(';') {
                    let device_id = first[..pos].to_string();
                    let rest = &first[pos + 1..];
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
        actions
    }

    fn load_updater_config(db: &Database) -> (String, String, String, String, VpxInstallMode) {
        let vpx_exe_path = db.get_config("vpx_exe_path").unwrap_or_default();
        let vpx_install_dir = db
            .get_config("vpx_install_dir")
            .unwrap_or_else(|| updater::default_install_dir().display().to_string());
        let vpx_fork_repo = db
            .get_config("vpx_fork_repo")
            .unwrap_or_else(|| updater::DEFAULT_FORK_REPO.to_string());
        let mut vpx_installed_tag = db.get_config("vpx_installed_tag").unwrap_or_default();
        let vpx_install_mode = if db.get_config("vpx_install_mode").as_deref() == Some("manual") {
            VpxInstallMode::Manual
        } else {
            VpxInstallMode::Auto
        };

        // Verify the executable still exists — if the install dir was deleted,
        // reset to fresh-install state so the user gets prompted to reinstall
        if !vpx_exe_path.is_empty() {
            let resolved = updater::resolve_vpx_exe(std::path::Path::new(&vpx_exe_path));
            if !resolved.is_file() {
                log::warn!(
                    "VPX executable no longer exists at {}, resetting install state",
                    resolved.display()
                );
                vpx_installed_tag.clear();
                let _ = db.set_config("vpx_installed_tag", "");
                let _ = db.set_config("vpx_exe_path", "");
                return (
                    String::new(),
                    vpx_install_dir,
                    vpx_fork_repo,
                    vpx_installed_tag,
                    vpx_install_mode,
                );
            }
        }

        (
            vpx_exe_path,
            vpx_install_dir,
            vpx_fork_repo,
            vpx_installed_tag,
            vpx_install_mode,
        )
    }

    fn detect_language(db: &Database) -> usize {
        let selected = if let Some(saved_lang) = db.get_config("language") {
            LANGUAGE_OPTIONS
                .iter()
                .position(|(c, _)| *c == saved_lang)
                .unwrap_or_else(i18n::detect_system_language)
        } else {
            i18n::detect_system_language()
        };
        let (lang_code, _) = LANGUAGE_OPTIONS[selected];
        i18n::set_locale(lang_code);
        log::info!("Language: {} ({})", lang_code, LANGUAGE_OPTIONS[selected].1);
        selected
    }

    fn spawn_update_check(
        vpx_fork_repo: &str,
    ) -> Option<crossbeam_channel::Receiver<anyhow::Result<ReleaseInfo>>> {
        if vpx_fork_repo.is_empty() {
            return None;
        }
        let repo = vpx_fork_repo.to_string();
        log::info!("Checking for Visual Pinball updates from {repo}...");
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let result = updater::check_latest_release(&repo);
            let _ = tx.send(result);
        });
        Some(rx)
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
                self.external_dmd = false;
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
            WizardPage::Outputs => {
                // Purely informational page — nothing to reset
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

    /// Process joystick events during wizard mode (tilt viz, input capture, device detection).
    fn process_wizard_joystick_events(&mut self) {
        let events: Vec<JoystickEvent> = self
            .joystick_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();

        for event in events {
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
                JoystickEvent::ButtonUp { .. } => {}
                JoystickEvent::AxisMotion { .. } => {}
                JoystickEvent::PinscapeDetected { vpx_id } => {
                    self.apply_pinscape_defaults(vpx_id);
                }
                JoystickEvent::DudesCabDetected { vpx_id } => {
                    log::info!("DudesCab detected in UI: {}", vpx_id);
                    self.pinscape_profile = 2;
                    self.apply_pinscape_defaults(vpx_id);
                }
                JoystickEvent::PinOneDetected { vpx_id } => {
                    log::info!("CSD PinOne detected in UI: {}", vpx_id);
                    self.pinscape_profile = 3;
                    self.apply_pinscape_defaults(vpx_id);
                }
                JoystickEvent::GamepadDetected { vpx_id, name } => {
                    log::info!("Gamepad detected in UI: {} ({})", name, vpx_id);
                    self.gamepad_id = Some(vpx_id.clone());
                }
            }
        }
    }

    /// Apply Pinscape default button mapping when a controller is detected.
    /// Profile is selected by `pinscape_profile`: 0 = KL25Z, 1 = Pico (OpenPinballDevice).
    fn apply_pinscape_defaults(&mut self, vpx_id: &str) {
        log::info!("Pinscape detected in UI: {}", vpx_id);
        self.pinscape_id = Some(vpx_id.to_string());
        let defaults = pinscape_button_defaults(self.pinscape_profile);
        for (action_id, button) in defaults {
            if let Some(action) = self.actions.iter_mut().find(|a| a.setting_id == *action_id) {
                if action.mapping.is_none() {
                    action.mapping = Some(CapturedInput::JoystickButton {
                        device_id: vpx_id.to_string(),
                        button: *button,
                        name: format!("{} Button {}", vpx_id, button),
                    });
                }
            }
        }
    }
}

mod audio_page;
mod autostart;
mod inputs_page;
mod launcher;
mod launcher_ui;
mod outputs_page;
mod rendering_page;
mod save;
mod screens_page;
mod tables_dir_page;
mod tilt_page;

use autostart::{is_autostart_enabled, set_autostart};

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Kiosk cursor: scale + lock + one-shot warp + persistent focus.
        // We reclaim focus every frame when the PF is unfocused, because
        // secondary viewports (BG/DMD/Topper) can steal it on some WMs
        // despite with_active(false) at creation time.
        //
        // DISABLED while VPX is running: we must release the cursor and stop
        // fighting for focus so VPX can take over keyboard/mouse input.
        let vpx_running = self.vpx_running.load(Ordering::Relaxed);
        if self.kiosk_cursor && !vpx_running {
            let ctx = ui.ctx();
            ctx.set_software_cursor_scale(3.0);
            ctx.set_cursor_lock(true);
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
            if !focused {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
            if !self.kiosk_cursor_warped {
                if let Some(inner) = ctx.input(|i| i.viewport().inner_rect) {
                    let size = inner.size();
                    ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(egui::pos2(
                        size.x / 2.0,
                        size.y / 2.0,
                    )));
                    self.kiosk_cursor_warped = true;
                }
                ctx.request_repaint();
            }
        }

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

        self.process_wizard_joystick_events();

        // === Wizard mode ===

        // Push the scrollbar flush to the window edge — default bar_outer_margin
        // leaves a small gap on the right that looks awkward on this layout.
        ui.style_mut().spacing.scroll.bar_outer_margin = 0.0;

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
                    // Block navigation on Screens page if VPX is not installed or downloading
                    let on_screens_page = self.page == WizardPage::Screens;
                    let downloading = self.update_downloading && on_screens_page;
                    let vpx_missing = on_screens_page && {
                        let resolved =
                            updater::resolve_vpx_exe(std::path::Path::new(&self.vpx_exe_path));
                        self.vpx_exe_path.is_empty() || !resolved.is_file()
                    };
                    let blocked = downloading || vpx_missing;

                    if self.page.index() < WizardPage::count() - 1 {
                        let btn = egui::Button::new(t!("wizard_next"));
                        if ui.add_enabled(!blocked, btn).clicked() {
                            self.next_page();
                        }
                    } else {
                        let btn = egui::Button::new(t!("wizard_finish"));
                        if ui.add_enabled(!blocked, btn).clicked() {
                            self.finalize_wizard(ui.ctx());
                        }
                    }
                    if vpx_missing && !downloading {
                        ui.colored_label(egui::Color32::from_rgb(255, 180, 50), t!("vpx_required"));
                    }
                    if downloading {
                        let (current, total) = self.update_progress;
                        if total > 0 {
                            let pct = current as f32 / total as f32;
                            let mb = current / (1024 * 1024);
                            let total_mb = total / (1024 * 1024);
                            ui.add(
                                egui::ProgressBar::new(pct)
                                    .text(format!("{mb}/{total_mb} MB"))
                                    .desired_width(200.0),
                            );
                        } else {
                            ui.spinner();
                        }
                        ui.ctx().request_repaint();
                    }
                });
            });
            ui.add_space(4.0);
        });

        // Main content — zero right/bottom inner+outer margins and no stroke
        // so the scrollbar sits flush against the window edge.
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(ui.style())
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 0,
                        top: 8,
                        bottom: 0,
                    })
                    .outer_margin(egui::Margin::ZERO)
                    .stroke(egui::Stroke::NONE),
            )
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        ui.add_space(0.0); // ensure full width
                        let _ = ui.available_width(); // force layout to use full width
                                                      // Process VPX download progress on every page so the
                                                      // download completes even when the user navigates away
                                                      // from the Screens page.
                        self.process_update_check();

                        match self.page {
                            WizardPage::Screens => self.render_screens_page(ui),
                            WizardPage::Rendering => self.render_rendering_page(ui),
                            WizardPage::Inputs => self.render_inputs_page(ui),
                            WizardPage::Outputs => self.render_outputs_page(ui),
                            WizardPage::Tilt => self.render_tilt_page(ui),
                            WizardPage::Audio => self.render_audio_page(ui),
                            WizardPage::TablesDir => self.render_tables_dir_page(ui),
                        }
                    });
            });
    }
}
