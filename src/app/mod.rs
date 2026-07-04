use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::audio::{self, AudioCommand, AudioConfig, Sound3DMode};
use crate::config::VpxConfig;
use crate::db::Database;
use crate::i18n::{self, LANGUAGE_OPTIONS};
use crate::inputs::{self, pinscape_button_defaults, CapturedInput, InputAction, JoystickEvent};
use crate::outputs_hid::DiscoveryState;
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

/// Cross-session signal for `main.rs` to know what to do after the current
/// eframe session exits. Set by UI click handlers (Finish, Config, etc.) just
/// before they send `ViewportCommand::Close`; `main.rs` reads it after
/// `run_native` returns and relaunches a fresh eframe with the right
/// viewport for the new mode.
///
/// We use a static rather than carrying state through `App` because
/// `eframe::run_native` consumes the App and we want a minimal handshake
/// that survives the Drop.
static NEXT_ACTION: Mutex<Option<AppMode>> = Mutex::new(None);

/// Request a mode switch after the current eframe session closes. Call
/// this, then send `ViewportCommand::Close` to trigger the transition.
pub fn request_mode_switch(mode: AppMode) {
    *NEXT_ACTION.lock().unwrap() = Some(mode);
}

/// Consume the pending mode switch request, if any. `main.rs` calls this
/// after each `run_native` returns; `Some(mode)` means relaunch in `mode`,
/// `None` means the user is quitting the app.
pub fn take_next_mode() -> Option<AppMode> {
    NEXT_ACTION.lock().unwrap().take()
}

/// One-shot signal set by `main.rs` when a display-server change was
/// detected between two PinReady boots (X11↔Wayland). Consumed by the
/// first `App::new` after startup and rendered as a modal notice inside
/// the wizard so the user understands why the launcher didn't open.
static SESSION_CHANGE_NOTICE: Mutex<Option<(String, String)>> = Mutex::new(None);

/// Publish the session-change signal. Called from `main.rs` after the
/// DB comparison.
pub fn set_session_change_notice(from: String, to: String) {
    *SESSION_CHANGE_NOTICE.lock().unwrap() = Some((from, to));
}

/// Consume the session-change signal (drained once). Read from `App::new`.
pub fn take_session_change_notice() -> Option<(String, String)> {
    SESSION_CHANGE_NOTICE.lock().unwrap().take()
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

/// Third-party crates PinReady depends on directly (one entry per
/// [dependencies] line in Cargo.toml, no groupings, no sub-crates).
/// Rendered as bullet list in the About window; keep alphabetically
/// sorted (case-insensitive) so additions land in an obvious place at
/// review time.
const ABOUT_CRATE_THANKS: &[&str] = &[
    "anyhow",
    "base64",
    "crossbeam-channel",
    "directb2s",
    "dirs",
    "display-info",
    "eframe",
    "egui",
    "egui-rotate",
    "egui_extras",
    "env_logger",
    "flate2",
    "hidapi",
    "image",
    "ini-preserve",
    "log",
    "noto-fonts-dl",
    "percent-encoding",
    "regex",
    "rfd",
    "rusqlite",
    "rust-i18n",
    "sdl-keybridge",
    "sdl3-sys",
    "self-replace",
    "serde",
    "serde_json",
    "sha2",
    "symphonia",
    "tar",
    "time",
    "ureq",
    "vpin",
    "walkdir",
    "winapi",
    "zip",
];

/// VP-ecosystem contributors specifically thanked — kept in sync with
/// the wizard's last-page credits (`system_page.rs`). Alphabetical,
/// case-insensitive.
const ABOUT_PEOPLE_THANKS: &[&str] = &[
    "Caviar4456",
    "Francisdb",
    "Jsm174",
    "Major Frenchy",
    "Somatik",
    "Spielfool",
    "Superhac",
    "Toxie",
    "Vbousquet",
];

/// A completed background-extraction result: (scan generation, table
/// index, relative .vpx path used as DB key, encoded image bytes,
/// source mtime for cache invalidation). The generation is bumped at
/// every `scan_tables()` so receiver can drop stale messages from a
/// previous scan whose indices no longer match the current grid.
pub type BgExtraction = (u64, usize, String, Vec<u8>, i64);

/// A completed VBS-patch classification+apply result: (rel_path,
/// embedded_sha256, sidecar_sha256, status, last_checked_mtime).
/// `status` is one of `vbs_patches::status::*`.
pub type VbsPatchRecord = (String, String, Option<String>, String, i64);

/// A discovered table
#[derive(Debug, Clone)]
pub struct TableEntry {
    pub path: std::path::PathBuf,
    pub name: String,
    /// Backglass image bytes (JPEG/PNG/WebP depending on source) loaded
    /// from the SQLite cache on scan, or `None` if extraction is pending
    /// or nothing could be found. When `None`, the grid renders a
    /// localized placeholder with instructions.
    pub bg_bytes: Option<std::sync::Arc<[u8]>>,
    /// `true` when the live VPSDB `Game.updated_at` has moved past the
    /// value we stored at link time → a fresher version of this table
    /// (or its metadata/media) is published in the catalog. Surfaced in
    /// the launcher as a "↑" badge + a header counter.
    pub update_available: bool,
    /// VPS-assigned game ID stored in `vps_link.vps_id`. Used by the
    /// outdated-badge click-handler to deep-link into the catalog page
    /// at `virtualpinballspreadsheet.github.io/games?game=<vps_id>`.
    pub vps_id: Option<String>,
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
    System,
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
            Self::System => t!("page_system"),
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
            Self::System => 7,
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
            7 => Some(Self::System),
            _ => None,
        }
    }

    fn count() -> usize {
        8
    }
}

/// How the Visual Pinball executable is provided
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpxInstallMode {
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
    #[allow(dead_code)]
    // kept for backwards compat with persisted DB; UI dropped in favour of the unified inputs list
    show_advanced_inputs: bool,
    /// Auto-map mode: when true, the inputs page advances to the next
    /// unmapped action automatically after each successful capture (or
    /// Escape, which leaves that action's binding unchanged). Stops
    /// when reaching the end of the list or when the user clicks Cancel.
    auto_map_active: bool,
    joystick_rx: Option<crossbeam_channel::Receiver<JoystickEvent>>,
    /// Set to false to ask the joystick thread to drain + exit. The
    /// thread joins quickly (≤ 10 ms — its sleep granularity), closes
    /// every `SDL_OpenJoystick` handle, and `SDL_QuitSubSystem(JOYSTICK)`
    /// before returning.
    joystick_running: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// JoinHandle for the joystick thread — kept so `launch_table` can
    /// `.join()` after flipping `joystick_running` to false, ensuring
    /// SDL3 joystick state is fully released before VPX is spawned.
    joystick_thread: Option<std::thread::JoinHandle<()>>,
    pinscape_id: Option<String>, // VPX device ID if pinball controller detected
    pinscape_profile: usize,     // 0 = KL25Z, 1 = Pico, 2 = DudesCab
    gamepad_id: Option<String>,  // VPX device ID if generic gamepad detected
    use_gamepad: bool,           // User toggle: use gamepad axes for flippers/nudge/plunger

    // Page 4 — Outputs (discovery tool)
    pub(super) output_discovery: DiscoveryState,

    // Page 3 — Tilt
    tilt: TiltConfig,

    // Page 4 — Audio
    audio: AudioConfig,
    audio_cmd_tx: Option<crossbeam_channel::Sender<AudioCommand>>,
    /// JoinHandle for the audio thread — same shutdown discipline as
    /// the joystick thread: drop the sender to signal shutdown, then
    /// `.join()` before launching VPX so the SDL3 audio device is
    /// fully released.
    audio_thread: Option<std::thread::JoinHandle<()>>,

    // Launcher preview audio. When `selected_table` changes we debounce
    // ~700ms before firing PreviewStart so quick scrolling doesn't spam
    // the audio thread. `preview_last_idx` is the table whose audio is
    // currently playing or queued; `preview_due_at` is the deadline for
    // the next PreviewStart.
    preview_last_idx: Option<usize>,
    preview_due_at: Option<std::time::Instant>,
    preview_playing: bool,

    // Page 5 — Tables dir
    tables_dir: String,

    // Launcher
    tables: Vec<TableEntry>,
    table_filter: String,
    table_filter_lower: String, // cached lowercase version of table_filter
    selected_table: usize,
    scroll_to_selected: bool, // set by joystick navigation to trigger scroll
    last_scroll_target: Option<f32>, // last forced vertical scroll offset — skip reset when the target hasn't moved
    launcher_cols: usize,            // number of columns in the grid (computed in render)
    images_preloaded: bool,
    // User theme preference, cycled from the topbar/wizard toggle. Purely
    // in-memory (starts at System every launch) — no persistence yet.
    theme_pref: egui::ThemePreference,
    // About window visibility, driven by the ℹ toolbar icon.
    about_open: bool,
    // One-shot: `Some((from, to))` when `main.rs` detected an X11↔Wayland
    // session change on this boot. Rendered as a centred modal on top of
    // the wizard; cleared when the user clicks OK.
    session_change_notice: Option<(String, String)>,
    // Two-step VPX launch state on Wayland: `Some((path, requested_at))`
    // when the user has clicked a table and PinReady sent
    // `ViewportCommand::RequestActivationToken` to the compositor but is
    // still waiting for the reply. The launcher polls this every frame
    // in `App::ui`: on `Event::ActivationTokenReceived` (or after a
    // 500 ms deadline) it drains the pending state and spawns VPX with
    // the token in `XDG_ACTIVATION_TOKEN` — the sealed serial is what
    // makes mutter/kwin actually grant focus to the freshly-created
    // window.
    pending_vpx_launch: Option<(std::path::PathBuf, std::time::Instant)>,
    // Viewport rotation for the root window. CW90 in cabinet mode, None
    // otherwise. Shadow copy of the value passed to the `RotationPlugin`
    // registered on the egui Context — kept here purely so layout code can
    // branch on rotation without going through the plugin every frame.
    rotation: egui_rotate::Rotation,
    // Kiosk mode: lock the cursor inside the PF window and center it on the grid.
    // Window placement itself is handled at creation via ViewportBuilder::with_monitor.
    // The software-cursor lock/scale themselves live on the plugin — accessed via
    // `App::with_software_cursor`.
    kiosk_cursor: bool, // signals that the plugin was built with a locked SoftwareCursor
    kiosk_cursor_warped: bool, // one-shot: warp cursor after window settles
    // Last virtual cursor position seen — used to restore the cursor where
    // it was after a spurious PointerGone (Mutter Wayland drops the
    // pointer-constraints lock during focus thrashing with secondary
    // viewports), so the user doesn't perceive it as a teleport-to-centre.
    kiosk_last_virtual_pos: Option<egui::Pos2>,
    // Launcher joystick nav auto-repeat: track which nav button is held
    nav_held: Option<(
        u8,
        launcher_input::LauncherAction,
        std::time::Instant,
        std::time::Instant,
    )>,
    // Background backglass extraction results. Sent by the thread spawned
    // in `scan_tables`: (table index, .vpx path relative to tables_dir,
    // JPEG bytes). The UI thread pulls these in `process_bg_extraction`,
    // stores the bytes in the SQLite cache, and updates the TableEntry +
    // egui image cache.
    bg_rx: Option<crossbeam_channel::Receiver<BgExtraction>>,

    // Scan generation counter. Bumped at every `scan_tables()`; carried
    // verbatim by every BgExtraction message so the UI can drop stale
    // results from a prior scan whose indices no longer line up with the
    // current `tables` Vec. Without this guard, dropping a new .vpx into
    // tables_dir + clicking Rescan while the previous scan's bg thread
    // was still running would shuffle thumbnails onto the wrong rows.
    scan_generation: u64,

    // VBS patch classification results — (rel_path, embedded_sha,
    // sidecar_sha, status, last_checked_mtime). Drained into
    // `vbs_patches` table. Silent on the UI side — we don't show per-
    // table badges; the log is the source of truth for what happened.
    vbs_rx: Option<crossbeam_channel::Receiver<VbsPatchRecord>>,

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

    // Desktop integration: app-menu shortcuts (PinReady + VPinballX) and
    // .vpx file association. Mirrors `autostart` — flipped from the wizard's
    // tables_dir page, applied in finalize_wizard.
    desktop_integration: bool,

    // Self-hosted mirror for the VBS catalog and VPin media DB. Empty
    // string = direct GitHub fetch (default). Edited from the System
    // wizard page; persisted to `Database::set_mirror_base_url` on
    // every change so subsequent runs pick it up immediately without
    // waiting for finalize_wizard.
    mirror_base_url: String,

    // Asset bundling ("merge") — optional import step that lives on the
    // Tables wizard page. The three source paths point at legacy
    // VPINMAME/PUPVIDEOS/Music dirs; the merge engine in `crate::merge`
    // walks each .vpx in tables_dir and places companion files into the
    // 10.8.1 folder-per-table layout.
    merge_src_vpinmame: String,
    merge_src_pupvideos: String,
    merge_src_music: String,
    merge_strategy: crate::merge::MergeStrategy,
    merge_progress_rx: Option<crossbeam_channel::Receiver<crate::merge::MergeEvent>>,
    merge_cancel: Option<Arc<AtomicBool>>,
    merge_log: Vec<crate::merge::MergeEvent>,
    merge_dry_run_report: Option<crate::merge::MergeReport>,
    merge_running: bool,
    merge_section_open: bool,

    // Opt-in: auto-patch VBS scripts at scan from
    // jsm174/vpx-standalone-scripts. Off by default — the catalog
    // sometimes introduces regressions on specific tables, so users
    // enable this deliberately from the Tables wizard page.
    jsm174_patching: bool,

    // Opt-in: VPSDB + VPinMediaDB enrichment at scan. Off by default
    // because the first sync downloads ~7 MB of JSON + per-table
    // media. Persisted in `config.catalog_enrichment_enabled`. Same
    // wizard page as the VBS patcher toggle.
    catalog_enrichment: bool,

    // Cancellation token for the catalog enrichment worker. Each
    // scan_tables() flips the previous token to `true` (signalling
    // the in-flight worker to bail out gracefully at its next
    // table-loop iteration) and replaces it with a fresh token for
    // the new worker. This keeps "click Rebuild 4× in a row" from
    // launching 4 concurrent workers that re-DL the same files
    // (which produced 4× duplicate "MediaDb installed" log lines on
    // initial sync). The newest scan always wins; older runs exit
    // cleanly without finishing their queue.
    catalog_cancel_token: Option<Arc<AtomicBool>>,

    // Deadline for sending `ViewportCommand::Close` after finalize_wizard.
    // Absolute wall-clock instant = knocker playback end + small buffer.
    // Compared with `Instant::now()` every frame; no ms hardcoding.
    close_at: Option<std::time::Instant>,

    // Deadline for resetting window level back to Normal after a focus-
    // raise from a second launch. `AlwaysOnTop` forces the compositor to
    // re-stack us on top (plain `Focus` is often refused by focus-stealing
    // prevention); we drop it a few frames later to avoid pinning.
    focus_reset_at: Option<std::time::Instant>,

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

    // PinReady self-update (separate from VPX update)
    pinready_latest_release: Option<ReleaseInfo>,
    pinready_update_check_rx: Option<crossbeam_channel::Receiver<anyhow::Result<ReleaseInfo>>>,
    pinready_update_progress_rx: Option<crossbeam_channel::Receiver<UpdateProgress>>,
    pinready_updating: bool,
    pinready_update_progress: (u64, u64),
    pinready_update_error: Option<String>,

    /// First run without a VPinballX.ini: VPX is launched once (no table) so
    /// it writes its pristine default ini, which then seeds every wizard page
    /// instead of PinReady's hardcoded defaults. `Some` while the generation
    /// thread runs; the deferred page advance happens when it reports back.
    ini_gen_rx: Option<crossbeam_channel::Receiver<bool>>,
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

        // Detect + install the user's language BEFORE anything that
        // calls `t!()` or `scancode_name()`, so localised labels (key
        // names, action names) end up in the right language from the
        // first render rather than staying frozen in English.
        let selected_language = Self::detect_language(&db);

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
        let (joystick_rx, joystick_running, joystick_thread) = inputs::spawn_joystick_thread();
        let (audio_cmd_tx, audio_thread) = audio::spawn_audio_thread();
        let update_check_rx = if vpx_install_mode == VpxInstallMode::Manual {
            None
        } else {
            Self::spawn_update_check(&vpx_fork_repo)
        };
        let jsm174_patching = db.jsm174_patching_enabled();
        let catalog_enrichment = db.catalog_enrichment_enabled();
        let merge_src_vpinmame = db.get_merge_source("vpinmame");
        let merge_src_pupvideos = db.get_merge_source("pupvideos");
        let merge_src_music = db.get_merge_source("music");
        let merge_strategy = crate::merge::MergeStrategy::from_db_str(&db.get_merge_strategy());
        let merge_section_open = merge_src_vpinmame.is_empty()
            && merge_src_pupvideos.is_empty()
            && merge_src_music.is_empty();
        let mirror_base_url = db.mirror_base_url().unwrap_or_default();

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
            auto_map_active: false,
            joystick_rx: Some(joystick_rx),
            joystick_running: Some(joystick_running),
            joystick_thread: Some(joystick_thread),
            pinscape_id: None,
            pinscape_profile,
            gamepad_id: None,
            use_gamepad: false,
            output_discovery: DiscoveryState::default(),
            tilt,
            audio,
            audio_cmd_tx: Some(audio_cmd_tx),
            audio_thread: Some(audio_thread),
            preview_last_idx: None,
            preview_due_at: None,
            preview_playing: false,
            tables_dir,
            tables: Vec::new(),
            table_filter: String::new(),
            table_filter_lower: String::new(),
            selected_table: 0,
            scroll_to_selected: false,
            last_scroll_target: None,
            launcher_cols: 1,
            images_preloaded: false,
            theme_pref: egui::ThemePreference::System,
            about_open: false,
            session_change_notice: take_session_change_notice(),
            pending_vpx_launch: None,
            rotation: egui_rotate::Rotation::None,
            kiosk_cursor: false,
            kiosk_cursor_warped: false,
            kiosk_last_virtual_pos: None,
            nav_held: None,
            bg_rx: None,
            scan_generation: 0,
            vbs_rx: None,
            vpx_running: Arc::new(AtomicBool::new(false)),
            vpx_status_rx: None,
            vpx_loading_msg: String::new(),
            vpx_loading_pct: None,
            vpx_hide_covers: false,
            vpx_error_log: None,
            autostart: is_autostart_enabled(),
            desktop_integration: is_desktop_integration_installed(),
            mirror_base_url,
            merge_src_vpinmame,
            merge_src_pupvideos,
            merge_src_music,
            merge_strategy,
            merge_progress_rx: None,
            merge_cancel: None,
            merge_log: Vec::new(),
            merge_dry_run_report: None,
            merge_running: false,
            merge_section_open,
            jsm174_patching,
            catalog_enrichment,
            catalog_cancel_token: None,
            close_at: None,
            focus_reset_at: None,
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
            pinready_latest_release: None,
            pinready_update_check_rx: Self::spawn_pinready_update_check(),
            pinready_update_progress_rx: None,
            pinready_updating: false,
            pinready_update_progress: (0, 0),
            pinready_update_error: None,
            ini_gen_rx: None,
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

    /// Set the viewport rotation for the root window. Called by `main` once,
    /// at App construction time, before `eframe::run_native`. The rotation
    /// drives the input/output hooks below — None is a no-op.
    pub fn set_rotation(&mut self, rotation: egui_rotate::Rotation) {
        self.rotation = rotation;
    }

    /// Enable kiosk cursor behavior: software-scaled cursor, locked inside the
    /// window, and warped to center once the window is mapped. The cursor
    /// itself is configured (scale, lock) at `RotationPlugin` registration in
    /// `main.rs` — this flag just tells the launcher runtime that the plugin
    /// was built with a `SoftwareCursor` and the kiosk warp/lock loop applies.
    /// Window placement is handled separately via `ViewportBuilder::with_monitor`.
    pub fn enable_kiosk_cursor(&mut self) {
        self.kiosk_cursor = true;
        self.kiosk_cursor_warped = false;
        log::info!("kiosk_cursor enabled — plugin owns scale/lock, warp on first frame");
    }

    /// Convenience: run `f` against the plugin's `SoftwareCursor` if the
    /// `RotationPlugin` was registered with one (kiosk mode). No-op otherwise.
    fn with_software_cursor<R>(
        ctx: &egui::Context,
        f: impl FnOnce(&mut egui_rotate::SoftwareCursor) -> R,
    ) -> Option<R> {
        ctx.with_plugin::<egui_rotate::RotationPlugin, _>(|p| p.software_cursor_mut().map(f))
            .flatten()
    }

    /// Two frame-less icon buttons for the wizard/launcher headers:
    /// - Theme cycle: System (🌗) → Light (☀) → Dark (🌙) → System.
    /// - Rotation cycle: None → CW90 → CW180 → CW270 → None. Drives the
    ///   `RotationPlugin` so the root viewport rotates live — makes the
    ///   wizard readable when the operator is standing off-axis from a
    ///   pincab playfield.
    ///
    /// `icon_size` is the RichText size; the whole widget is roughly
    /// `2 * icon_size + item_spacing.x` wide.
    pub(super) fn toolbar_toggles(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        icon_size: f32,
    ) {
        let (theme_glyph, theme_hint, next_theme) = match self.theme_pref {
            egui::ThemePreference::System => {
                ("🌗", t!("toolbar_theme_auto"), egui::ThemePreference::Light)
            }
            egui::ThemePreference::Light => {
                ("☀", t!("toolbar_theme_light"), egui::ThemePreference::Dark)
            }
            egui::ThemePreference::Dark => (
                "🌙",
                t!("toolbar_theme_dark"),
                egui::ThemePreference::System,
            ),
        };
        let theme_resp = ui
            .add(egui::Button::new(egui::RichText::new(theme_glyph).size(icon_size)).frame(false))
            .on_hover_text(theme_hint.to_string());
        if theme_resp.clicked() {
            self.theme_pref = next_theme;
            ctx.set_theme(self.theme_pref);
        }

        let next_rotation = self.rotation.next_cw();
        let rot_deg = match next_rotation {
            egui_rotate::Rotation::None => "0°",
            egui_rotate::Rotation::CW90 => "90°",
            egui_rotate::Rotation::CW180 => "180°",
            egui_rotate::Rotation::CW270 => "270°",
        };
        let rot_resp = ui
            .add(egui::Button::new(egui::RichText::new("↻").size(icon_size)).frame(false))
            .on_hover_text(t!("toolbar_rotate_next", deg = rot_deg).to_string());
        if rot_resp.clicked() {
            self.rotation = next_rotation;
            ctx.with_plugin::<egui_rotate::RotationPlugin, _>(|p| {
                p.set_rotation(next_rotation);
            });
            ctx.request_repaint();
        }

        // ℹ — About box. Also the only place the launcher exposes the
        // version number now that the header label was dropped.
        let info_resp = ui
            .add(egui::Button::new(egui::RichText::new("ℹ").size(icon_size)).frame(false))
            .on_hover_text(t!("toolbar_about").to_string());
        if info_resp.clicked() {
            self.about_open = true;
        }
    }

    /// Modal-ish About window: version, homepage, license, credits. Opens
    /// from the ℹ icon in the topbar; also serves as the version display
    /// since the header no longer prints it.
    pub(super) fn render_about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = self.about_open;
        egui::Window::new(t!("about_title").to_string())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(460.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading(format!("PinReady v{}", env!("CARGO_PKG_VERSION")));
                    ui.add_space(4.0);
                    ui.label(t!("about_tagline"));
                    ui.add_space(12.0);
                    ui.hyperlink_to(t!("about_homepage").to_string(), env!("CARGO_PKG_HOMEPAGE"));
                    // GPL is copyleft — showing © would clash with the
                    // spirit the user picked the license for. Just plain
                    // authorship + license spdx.
                    ui.label(format!("Sylvain Gargasson — {}", env!("CARGO_PKG_LICENSE")));
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.strong(t!("about_thanks_crates_title"));
                        ui.add_space(4.0);
                        // Comma-joined prose — reads like a real
                        // acknowledgement, wraps naturally to the
                        // window width, and doesn't turn the About
                        // into a giant bullet-list.
                        ui.label(ABOUT_CRATE_THANKS.join(", "));
                        ui.add_space(12.0);
                        ui.strong(t!("about_thanks_people_title"));
                        ui.add_space(4.0);
                        // 3-column invisible grid, manually centred: the
                        // grid takes the width of its content, and we
                        // pre-pad the row so the whole block sits on the
                        // horizontal axis of the About window.
                        let col_w = 140.0;
                        let cols = 3;
                        let block_w = (col_w * cols as f32)
                            + (ui.spacing().item_spacing.x * (cols as f32 - 1.0));
                        let pad = ((ui.available_width() - block_w) * 0.5).max(0.0);
                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            egui::Grid::new("about_people_grid")
                                .num_columns(cols)
                                .min_col_width(col_w)
                                .show(ui, |ui| {
                                    // 9 names fit exactly in 3 rows,
                                    // so no dangling row to fill.
                                    for (i, name) in ABOUT_PEOPLE_THANKS.iter().enumerate() {
                                        ui.label(format!("• {name}"));
                                        if i % cols == cols - 1 {
                                            ui.end_row();
                                        }
                                    }
                                });
                        });
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(t!("about_thanks_testers")).italics());
                    });
            });
        self.about_open = open;
    }

    /// Show the "you switched from X11 to Wayland (or vice versa),
    /// that's why the wizard reopened" modal. Fires only when
    /// `session_change_notice` is `Some`; user dismisses via OK.
    /// No-op on non-Linux since `session::detect()` never signals a
    /// change there.
    pub(super) fn render_session_change_notice(&mut self, ctx: &egui::Context) {
        let Some((from, to)) = self.session_change_notice.clone() else {
            return;
        };
        let from_label = crate::session::label(&from);
        let to_label = crate::session::label(&to);
        let mut dismiss = false;
        egui::Window::new(t!("session_change_title").to_string())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(t!("session_change_body", from = from_label, to = to_label));
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    if ui.button(t!("session_change_ok").to_string()).clicked() {
                        dismiss = true;
                    }
                });
            });
        if dismiss {
            self.session_change_notice = None;
        }
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
                // VPX alternatives (`a | b`) fill the two independent slots:
                // the first `Key;N` part lands in `keyboard`, the first
                // device part in `joystick`. `&` combos: keep the first leg,
                // like before.
                for part in mapping_str.split('|') {
                    let part = part.split('&').next().unwrap_or("").trim();
                    if let Some(sc_str) = part.strip_prefix("Key;") {
                        if action.keyboard.is_none() {
                            if let Ok(sc_val) = sc_str.parse::<i32>() {
                                let scancode = sdl3_sys::everything::SDL_Scancode(sc_val);
                                action.keyboard = Some(CapturedInput::Keyboard {
                                    scancode,
                                    name: inputs::scancode_name(scancode),
                                });
                            }
                        }
                    } else if let Some(pos) = part.find(';') {
                        if action.joystick.is_none() {
                            let device_id = part[..pos].to_string();
                            let rest = &part[pos + 1..];
                            if let Ok(button) = rest.split(';').next().unwrap_or("").parse::<u8>() {
                                action.joystick = Some(CapturedInput::JoystickButton {
                                    device_id: device_id.clone(),
                                    button,
                                    name: format!("{} Button {}", device_id, button),
                                });
                            }
                        }
                    }
                }
                // A `Key;` part identical to the action's default is the
                // fallback older PinReady versions appended automatically —
                // treat it as "no custom key" so the UI keeps showing
                // "(default)" instead of a phantom customization.
                if let Some(CapturedInput::Keyboard { scancode, .. }) = &action.keyboard {
                    if *scancode == action.default_scancode {
                        action.keyboard = None;
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

            // For manual installs, always query the executable version at startup.
            // Do NOT cache this to the database — only the auto-download flow writes tags to DB.
            if vpx_install_mode == VpxInstallMode::Manual {
                if let Some(version) = crate::updater::query_vpx_version(&vpx_exe_path) {
                    log::info!("Detected VPX version from executable: {}", version);
                    vpx_installed_tag = version;
                } else {
                    log::debug!(
                        "Could not query VPX version from executable at {}",
                        vpx_exe_path
                    );
                }
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

    /// Spawn a background thread that queries the PinReady repo for the
    /// latest release and returns it via crossbeam channel.
    fn spawn_pinready_update_check(
    ) -> Option<crossbeam_channel::Receiver<anyhow::Result<ReleaseInfo>>> {
        log::info!(
            "Checking for PinReady updates from {}...",
            updater::PINREADY_REPO
        );
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let result = updater::check_pinready_release();
            let _ = tx.send(result);
        });
        Some(rx)
    }

    fn next_page(&mut self) {
        self.leave_page_hooks();
        // First run: no VPinballX.ini on disk yet. Before PinReady writes its
        // first (sparse) ini, launch VPX once without a table — it generates
        // its complete default ini — so rendering / inputs / tilt / audio all
        // start from VPX's *real* defaults instead of hardcoded guesses. The
        // page advance is deferred until the generation thread reports back
        // (modal overlay in `render_wizard`, which also reloads the config).
        if self.page == WizardPage::Screens
            && self.ini_gen_rx.is_none()
            && !self.config.path().exists()
        {
            let exe = updater::resolve_vpx_exe(std::path::Path::new(&self.vpx_exe_path));
            if exe.is_file() {
                self.ini_gen_rx = Some(Self::spawn_ini_generation(
                    exe,
                    self.config.path().to_path_buf(),
                ));
                return;
            }
        }
        self.advance_page();
    }

    /// The unconditional part of [`Self::next_page`]: persist the current
    /// page and move forward.
    fn advance_page(&mut self) {
        let next = self.page.index() + 1;
        if let Some(page) = WizardPage::from_index(next) {
            self.save_current_page();
            self.page = page;
        }
    }

    /// Run `VPX -h` once so it writes its pristine default ini. The help path
    /// initialises (and saves) the settings without opening the player window
    /// — on Windows a bare launch would pop the full window — then exits by
    /// itself. Reports `true` when the ini exists afterwards. The stable-size
    /// watch + kill remain as a belt in case a VPX build lingers anyway.
    fn spawn_ini_generation(
        exe: std::path::PathBuf,
        ini: std::path::PathBuf,
    ) -> crossbeam_channel::Receiver<bool> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            log::info!(
                "No VPinballX.ini yet — running `{} -h` once to generate defaults",
                exe.display()
            );
            let mut child = match std::process::Command::new(&exe)
                .arg("-h")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Could not launch VPX for ini generation: {e}");
                    let _ = tx.send(false);
                    return;
                }
            };

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let mut last_size: Option<u64> = None;
            loop {
                if let Ok(Some(status)) = child.try_wait() {
                    log::info!("VPX exited by itself during ini generation ({status})");
                    break;
                }
                if std::time::Instant::now() > deadline {
                    log::warn!("VPX ini generation timed out after 20s — killing");
                    break;
                }
                if let Ok(meta) = std::fs::metadata(&ini) {
                    let size = meta.len();
                    if size > 0 && last_size == Some(size) {
                        // Two stable polls: VPX has finished writing.
                        break;
                    }
                    last_size = Some(size);
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            let _ = child.kill();
            let _ = child.wait();
            // Exit-time write: give the file one more look after teardown.
            let ok = ini.exists();
            log::info!(
                "VPX default ini generation {}",
                if ok { "succeeded" } else { "failed" }
            );
            let _ = tx.send(ok);
        });
        rx
    }

    /// Adopt the ini VPX just generated: reload it from disk and re-derive
    /// the wizard state of the pages the user has *not* configured yet
    /// (rendering, inputs, tilt, audio). Screens/cabinet UI state is left
    /// alone — `advance_page` saves it on top right after.
    fn adopt_generated_ini(&mut self) {
        let path = self.config.path().to_path_buf();
        match VpxConfig::load(Some(&path)) {
            Ok(config) => self.config = config,
            Err(e) => {
                log::warn!("Could not reload generated ini: {e}");
                return;
            }
        }
        let (aa_factor, msaa, fxaa, sharpen, pf_reflection, max_tex_dim, sync_mode, max_framerate) =
            Self::load_rendering_config(&self.config);
        self.aa_factor = aa_factor;
        self.msaa = msaa;
        self.fxaa = fxaa;
        self.sharpen = sharpen;
        self.pf_reflection = pf_reflection;
        self.max_tex_dim = max_tex_dim;
        self.sync_mode = sync_mode;
        self.max_framerate = max_framerate;
        self.actions = Self::load_input_mappings(&self.config);
        self.tilt.load_from_config(&self.config);
        self.audio.load_from_config(&self.config);
        log::info!("Adopted VPX-generated defaults for rendering/inputs/tilt/audio");
    }

    fn prev_page(&mut self) {
        self.leave_page_hooks();
        if self.page.index() > 0 {
            if let Some(page) = WizardPage::from_index(self.page.index() - 1) {
                self.page = page;
            }
        }
    }

    /// Called just before the wizard switches away from the current page.
    /// Ensures any background activity (pulse loops, etc.) is halted so it
    /// doesn't keep running invisibly on another page.
    fn leave_page_hooks(&mut self) {
        if self.page == WizardPage::Outputs && self.output_discovery.loop_running {
            self.output_discovery.stop_loop();
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
                self.output_discovery.stop_session();
                self.output_discovery = DiscoveryState::default();
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
            WizardPage::System => {
                self.autostart = false;
                self.desktop_integration = false;
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
                            // Fills the joystick slot only — the keyboard
                            // binding of the action is kept alongside.
                            self.actions[idx].joystick = Some(CapturedInput::JoystickButton {
                                device_id: device_id.clone(),
                                button: *button,
                                name: name.clone(),
                            });
                        }
                        // In auto-map mode, advance to the next action;
                        // otherwise just go Idle. Mirrors the keyboard-
                        // capture path in inputs_page.rs.
                        self.advance_capture_or_finish();
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
                if action.joystick.is_none() {
                    action.joystick = Some(CapturedInput::JoystickButton {
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
mod desktop_integration;
mod inputs_page;
mod launcher;
mod launcher_input;
mod launcher_ui;
mod outputs_page;
mod rendering_page;
mod save;
mod screens_page;
mod system_page;
mod tables_dir_page;
mod tilt_page;

use autostart::{is_autostart_enabled, set_autostart};
use desktop_integration::{is_desktop_integration_installed, set_desktop_integration};

impl eframe::App for App {
    /// The `RotationPlugin` (registered in `main.rs`) owns input rotation,
    /// primitive rotation, cursor drawing and OS-cursor hiding — none of
    /// which lives in this file anymore.
    ///
    /// The one bit that still needs a hook is a Wayland kiosk workaround:
    /// under `CursorGrab::Locked` the OS delivers only raw `MouseMoved`
    /// deltas, no `WindowEvent::CursorMoved`. egui's hover hit-test relies
    /// on `PointerMoved` to know where the pointer is, so we inject a
    /// synthetic one every frame the software cursor is captured. The
    /// value is a dummy: the plugin's own `input_hook` (which runs right
    /// after this) rewrites it to the current `virtual_pos` before egui
    /// consumes it.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if raw_input.viewport_id != egui::ViewportId::ROOT || self.rotation.is_none() {
            return;
        }
        let has_virtual_cursor = ctx
            .with_plugin::<egui_rotate::RotationPlugin, _>(|p| {
                p.software_cursor().and_then(|c| c.virtual_pos()).is_some()
            })
            .unwrap_or(false);
        if has_virtual_cursor {
            raw_input
                .events
                .push(egui::Event::PointerMoved(egui::Pos2::ZERO));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Two-step VPX launch: when the launcher clicks a table it stashes
        // the path in `pending_vpx_launch` and asks the compositor for a
        // fresh xdg-activation-v1 token via `RequestActivationToken`. The
        // reply lands here as an `Event::ActivationTokenReceived` on the
        // next frame; a 500 ms deadline bails out if the compositor never
        // replies so the launch isn't held forever.
        if let Some((path, requested_at)) = self.pending_vpx_launch.clone() {
            let token = ui.ctx().input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::ActivationTokenReceived { token, .. } => Some(token.clone()),
                    _ => None,
                })
            });
            let deadline_exceeded = requested_at.elapsed() >= std::time::Duration::from_millis(500);
            if token.is_some() || deadline_exceeded {
                self.pending_vpx_launch = None;
                if token.is_none() {
                    log::warn!(
                        "xdg-activation reply missed 500 ms deadline; launching VPX without token"
                    );
                }
                self.launch_table(&path, token);
            } else {
                ui.ctx().request_repaint();
            }
        }

        // Kiosk cursor: scale + lock + virtual-pos bootstrap. Disabled while
        // VPX is running so it can take over keyboard/mouse input cleanly.
        let vpx_running = self.vpx_running.load(Ordering::Relaxed);
        if self.kiosk_cursor && !vpx_running {
            let ctx = ui.ctx();
            // Force a repaint every frame. Mutter Wayland skips presenting
            // surfaces that don't have user-focus even when we submit fresh
            // frames; without this the cursor visibly freezes although
            // virtual_pos is updating internally each frame.
            ctx.request_repaint();

            // The SoftwareCursor lock and the OS-level pointer grab are owned
            // by the egui-rotate plugin since 1.1: the lock is set once at
            // construction (nothing toggles it anymore — VPX launch/resume
            // release and re-capture the *whole* cursor instead), and the
            // grab is sent on capture/release transitions with the right
            // per-platform mode (see `os_grab` in `main.rs`).
            //
            // `CursorVisible(false)` stays: the plugin only hides the OS
            // pointer while the software cursor is captured, and there is a
            // captureless gap right after VPX exits (until the warp latch
            // below re-seeds the cursor) where the OS pointer would flicker
            // on top of the playfield.
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));

            // Reclaim focus only when the compositor will honour it. On
            // Wayland, `ViewportCommand::Focus` is a no-op without an
            // `xdg_activation_v1` token (egui#8142) and may trigger Mutter's
            // anti-focus-stealing protection, demoting the playfield. Skip
            // the reclaim entirely there; on X11 it works normally.
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
            if !focused && std::env::var("WAYLAND_DISPLAY").is_err() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }

            // Read the plugin's cursor state once: track the most recent
            // virtual position so we can restore it after a spurious
            // capture loss (Mutter Wayland drops the pointer-constraints
            // lock during focus thrash), and detect said capture loss to
            // reset the warp latch.
            let (virtual_pos, is_captured) = ctx
                .with_plugin::<egui_rotate::RotationPlugin, _>(|p| {
                    let c = p.software_cursor();
                    (
                        c.and_then(|c| c.virtual_pos()),
                        c.is_some_and(|c| c.is_captured()),
                    )
                })
                .unwrap_or((None, false));
            if let Some(p) = virtual_pos {
                self.kiosk_last_virtual_pos = Some(p);
            }
            if self.kiosk_cursor_warped && !is_captured {
                self.kiosk_cursor_warped = false;
            }

            if !self.kiosk_cursor_warped {
                // Bootstrap the SoftwareCursor at the last known position
                // (or the viewport centre on first run).
                //
                // We use `ctx.viewport_rect()` rather than
                // `viewport().inner_rect` because the latter relies on
                // `winit::Window::inner_position()`, which returns None
                // under Wayland (the protocol doesn't expose absolute
                // window positions to clients).
                //
                // Reject the egui placeholder default
                // `Rect::from_min_size(0, vec2(10_000, 10_000))` — surfaced
                // as ~8333×8333 after `round_ui` — by detecting a square
                // viewport_rect with both dimensions ≥ 4000 logical points.
                // No real cabinet display is perfectly square at that size.
                let vr = ctx.viewport_rect();
                let placeholder =
                    (vr.width() == vr.height() && vr.width() > 4000.0) || vr.area() <= 1.0;
                if !placeholder {
                    // Clamp the restore position inside a safe inner rect.
                    // The cursor arrow is anchored at its tip and extends
                    // ~60 logical points down-right (scale 3 × 16-20 base);
                    // under CW90 the inverse rotation maps "down-right" to
                    // "up-left" physically, so a virtual_pos within ~60
                    // points of the right or bottom edge clips half the
                    // arrow off-screen and the cursor disappears visually.
                    let safe_margin = 64.0;
                    let safe = vr.shrink(safe_margin);
                    let target = self
                        .kiosk_last_virtual_pos
                        .map(|p| {
                            egui::pos2(
                                p.x.clamp(safe.min.x, safe.max.x),
                                p.y.clamp(safe.min.y, safe.max.y),
                            )
                        })
                        .unwrap_or_else(|| vr.center());
                    Self::with_software_cursor(ctx, |c| c.set_virtual_pos(target));
                    self.kiosk_cursor_warped = true;
                }
                ctx.request_repaint();
            }
        }

        // Scheduled close (fires once the knocker sound has finished playing
        // on the audio thread — deadline set by `finalize_wizard` from the
        // decoded PCM length, not a hardcoded timeout).
        if let Some(deadline) = self.close_at {
            if std::time::Instant::now() >= deadline {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            ui.ctx().request_repaint();
        }

        // Another launch asked us to raise our window. We now actually
        // reach this branch immediately because `register_egui_ctx` lets
        // the socket listener call `request_repaint()` on wake.
        //
        // `Focus` alone is often refused by focus-stealing prevention
        // (Mutter X11 & Wayland, KWin…). The AlwaysOnTop toggle forces a
        // z-order bump even when the WM blocks direct raise; we drop back
        // to Normal 300ms later so the window isn't permanently pinned.
        if crate::pidlock::take_focus_request() {
            log::info!("Focus request from second launch — raising window");
            let ctx = ui.ctx();
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop,
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
            self.focus_reset_at =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(300));
            ctx.request_repaint();
        }
        if let Some(deadline) = self.focus_reset_at {
            if std::time::Instant::now() >= deadline {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                        egui::WindowLevel::Normal,
                    ));
                self.focus_reset_at = None;
            } else {
                ui.ctx().request_repaint();
            }
        }

        // Route based on mode — joystick events are handled per-mode
        if self.mode == AppMode::Launcher {
            self.render_launcher(ui);
            return;
        }

        self.process_wizard_joystick_events();

        // === Wizard mode ===

        self.render_about_window(ui.ctx());
        // Session-change notice (X11↔Wayland) — modal that explains why
        // the wizard reopened when the user was expecting the launcher.
        self.render_session_change_notice(ui.ctx());

        // Push the scrollbar flush to the window edge — default bar_outer_margin
        // leaves a small gap on the right that looks awkward on this layout.
        ui.style_mut().spacing.scroll.bar_outer_margin = 0.0;

        // Header
        // First-run ini generation in progress: modal + deferred page advance.
        if let Some(rx) = &self.ini_gen_rx {
            match rx.try_recv() {
                Ok(ok) => {
                    self.ini_gen_rx = None;
                    if ok {
                        self.adopt_generated_ini();
                    }
                    // Either way, move on — on failure the pages simply keep
                    // PinReady's built-in defaults, as before.
                    self.advance_page();
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    egui::Modal::new(egui::Id::new("ini_gen_modal")).show(ui.ctx(), |ui| {
                        ui.set_width(360.0);
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label(t!("wizard_generating_ini"));
                        });
                    });
                    ui.ctx().request_repaint();
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.ini_gen_rx = None;
                    self.advance_page();
                }
            }
        }

        egui::Panel::top("wizard_header").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                // Two-icon toolbar (theme + rotation) at the very start of the
                // wizard header — reachable from any wizard page. Rotation is
                // especially useful during config from a bench off-axis from
                // the playfield.
                let toolbar_size = (ui.spacing().interact_size.y - 4.0).max(14.0);
                let ctx = ui.ctx().clone();
                self.toolbar_toggles(&ctx, ui, toolbar_size);
                ui.separator();
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
                    // Tables dir is mandatory before leaving the Tables page —
                    // skipping it would land the user in a launcher that has
                    // nothing to scan and shows an empty grid.
                    let on_tables_page = self.page == WizardPage::TablesDir;
                    let tables_dir_missing = on_tables_page
                        && (self.tables_dir.is_empty()
                            || !std::path::Path::new(&self.tables_dir).is_dir());
                    let blocked = downloading || vpx_missing || tables_dir_missing;

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
                    if tables_dir_missing {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 180, 50),
                            t!("tables_path_required_inline"),
                        );
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
                        self.process_pinready_update_check(ui.ctx());

                        match self.page {
                            WizardPage::Screens => self.render_screens_page(ui),
                            WizardPage::Rendering => self.render_rendering_page(ui),
                            WizardPage::Inputs => self.render_inputs_page(ui),
                            WizardPage::Outputs => self.render_outputs_page(ui),
                            WizardPage::Tilt => self.render_tilt_page(ui),
                            WizardPage::Audio => self.render_audio_page(ui),
                            WizardPage::TablesDir => self.render_tables_dir_page(ui),
                            WizardPage::System => self.render_system_page(ui),
                        }
                    });
            });
    }
}
