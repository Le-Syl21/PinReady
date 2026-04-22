// PinReady — Visual Pinball configuration wizard & table launcher
// Copyright (C) 2026 — Licensed under GPLv3+
// See https://www.gnu.org/licenses/gpl-3.0.html
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

rust_i18n::i18n!("locales");

mod app;
mod assets;
mod audio;
mod config;
mod db;
mod i18n;
mod inputs;
mod outputs_hid;
mod pidlock;
mod screens;
mod tilt;
mod updater;
mod vbs_patches;

use anyhow::Result;
use std::io::Write;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize logging to both stderr and a log file next to the database.
/// The log file is rotated at startup: PinReady.log → PinReady.log.1
fn init_logging() {
    let log_dir = db::default_db_path()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("PinReady.log");
    let log_prev = log_dir.join("PinReady.log.1");

    // Rotate: keep one previous log
    if log_path.exists() {
        let _ = std::fs::rename(&log_path, &log_prev);
    }

    let log_file = std::fs::File::create(&log_path).ok();

    // Panic hook — panics bypass the log crate, so without this the actual
    // panic message (wgpu errors, etc.) only hits stderr and is lost if the
    // user runs detached from a terminal.
    if let Some(file) = log_file.as_ref().and_then(|f| f.try_clone().ok()) {
        std::panic::set_hook(Box::new(move |info| {
            use std::io::Write as _;
            let msg = format!(
                "\n!!! PANIC: {}\n{:?}\n",
                info,
                std::backtrace::Backtrace::capture()
            );
            let _ = (&file).write_all(msg.as_bytes());
            eprintln!("{msg}");
        }));
    }

    // Default to `info` so launcher diagnostics (kiosk bounds, display roles, etc.)
    // land in the log file. RUST_LOG env var still overrides if set.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(move |buf, record| {
            let ts = time::OffsetDateTime::now_local()
                .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
                .format(&time::macros::format_description!(
                    "[year]-[month]-[day] [hour]:[minute]:[second]"
                ))
                .unwrap();
            let line = format!(
                "[{ts} {level} {target}] {msg}\n",
                level = record.level(),
                target = record.target(),
                msg = record.args(),
            );
            // Write to stderr (default behavior)
            let _ = buf.write_all(line.as_bytes());
            // Write to log file
            if let Some(ref file) = log_file {
                use std::io::Write as _;
                let _ = (&*file).write_all(line.as_bytes());
            }
            Ok(())
        })
        .init();

    eprintln!("Log file: {}", log_path.display());
}

/// On Windows release builds the process is a GUI subsystem app (no console).
/// Attach to the parent terminal's console before printing CLI output so that
/// `println!` is visible. Must be called before the first write to stdout.
#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn attach_windows_console() {
    extern "system" {
        fn AttachConsole(dwProcessId: u32) -> i32;
        fn AllocConsole() -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    // SAFETY: Win32 console API; no invariants to uphold beyond calling convention.
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            // Not launched from a console (e.g. double-click) — open a new one.
            AllocConsole();
        }
    }
}

fn main() -> Result<()> {
    // Parse arguments
    let args: Vec<String> = std::env::args().collect();

    // On Windows release builds stdout is detached from the terminal.
    // Attach to the parent console before the first println! so Rust's lazy
    // stdout initialisation picks up the valid handle.
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    {
        let is_cli = args
            .iter()
            .any(|a| matches!(a.as_str(), "--version" | "-v" | "--help" | "-h"));
        if is_cli {
            attach_windows_console();
        }
    }

    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("PinReady v{VERSION}");
        println!("License: GPLv3+ — https://www.gnu.org/licenses/gpl-3.0.html");
        return Ok(());
    }
    let force_config = args.iter().any(|a| a == "--config" || a == "-c");
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("PinReady v{VERSION} — Visual Pinball configurator & launcher");
        println!();
        println!("Usage: pinready [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --config, -c    Force configuration wizard mode");
        println!("  --version, -v   Show version and license");
        println!("  --help, -h      Show this help");
        return Ok(());
    }

    // Acquire the single-instance PID lock BEFORE init_logging: the log
    // rotation that init_logging does (rename PinReady.log →
    // PinReady.log.1) would otherwise clobber the running instance's
    // diagnostics every time a second launch happens. Only the instance
    // that wins the lock should touch the log files.
    let lock_dir = db::default_db_path()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let _pid_lock = match pidlock::PidLock::acquire_in(&lock_dir) {
        Ok(lock) => lock,
        Err(pidlock::PidLockError::AlreadyRunning { path, pid }) => {
            // Another PinReady is live. Politely ask it to bring its window
            // to the front (via the focus-on-relaunch Unix socket) and exit
            // cleanly. Only stderr here — init_logging isn't set up yet,
            // which is intentional: leaves the running instance's log
            // intact.
            if pidlock::try_notify_focus(&lock_dir) {
                eprintln!("PinReady already running — asked it to focus and exiting.");
                return Ok(());
            }
            let pid_display = pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".to_string());
            eprintln!(
                "PinReady is already running (PID {pid_display}).\n\
                 Lock file: {}\n\
                 To stop the running instance: kill {pid_display}",
                path.display()
            );
            std::process::exit(1);
        }
        Err(pidlock::PidLockError::OpenFailed(e)) => {
            eprintln!(
                "PID lock file could not be opened ({e}). \
                 This typically means a permissions problem on {}. \
                 Aborting to avoid running two PinReady in parallel.",
                lock_dir.display()
            );
            std::process::exit(1);
        }
    };

    // Only now initialize logging — we've confirmed we're the sole instance,
    // so rotating the log is safe and desired.
    init_logging();
    log::info!("PinReady v{VERSION} starting...");
    log::info!(
        "PID lock held at {}",
        lock_dir.join("PinReady.pid").display()
    );

    // Initialize SDL3 for display enumeration only (joystick + audio handled in their threads)
    unsafe {
        use sdl3_sys::everything::*;
        if !SDL_Init(SDL_INIT_VIDEO) {
            let err = std::ffi::CStr::from_ptr(SDL_GetError()).to_string_lossy();
            anyhow::bail!("Failed to init SDL3: {err}");
        }
    }
    log::info!("SDL3 initialized (video for display enumeration)");

    // Determine the initial mode once from CLI + DB, then loop: each mode
    // owns its own eframe session with a viewport built at window creation
    // time. Switching modes = close current session, main reads the signal
    // set by `crate::app::request_mode_switch`, restart with fresh state.
    // This avoids any live-viewport mutation and its stale-compositor
    // artifacts (double rotation, half-rendered frames).
    let initial_mode = {
        let db = db::Database::open(None)?;
        let configured = db.get_config("wizard_completed").as_deref() == Some("true");
        if force_config || !configured {
            app::AppMode::Wizard
        } else {
            app::AppMode::Launcher
        }
    };
    let mut current_mode = initial_mode;
    log::info!("Starting in {:?} mode", current_mode);

    loop {
        run_eframe_for_mode(current_mode)?;

        // After the eframe session closed, decide what's next:
        //   Some(mode) → a UI handler requested a switch; relaunch as that.
        //   None       → user closed the window without switching; quit.
        match app::take_next_mode() {
            Some(next) => {
                log::info!("Relaunching eframe in {:?} mode", next);
                current_mode = next;
            }
            None => {
                log::info!("No next-mode signal — user quit");
                break;
            }
        }
    }

    Ok(())
}

/// Single eframe session for one mode. Builds the viewport with the right
/// rotation / monitor / decorations for the mode at creation time, runs
/// until the App signals Close (mode switch or quit), returns.
fn run_eframe_for_mode(mode: app::AppMode) -> Result<()> {
    let db = db::Database::open(None)?;
    let vpx_config = config::VpxConfig::load(None)?;
    let displays = screens::enumerate_displays();

    let (viewport, want_kiosk_cursor) = build_viewport(&displays, mode, &vpx_config);
    let start_in_wizard = matches!(mode, app::AppMode::Wizard);
    let mut app = app::App::new(vpx_config, db, start_in_wizard, displays);
    if want_kiosk_cursor {
        app.enable_kiosk_cursor();
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "PinReady",
        options,
        Box::new(|cc| {
            // Load Noto fonts for non-Latin scripts (Arabic, CJK, Devanagari, Thai, etc.)
            let noto_fonts = noto_fonts_dl::load_fonts();
            if !noto_fonts.is_empty() {
                let font_count = noto_fonts.len();
                let mut font_defs = egui::FontDefinitions::default();
                for (name, data) in noto_fonts {
                    font_defs
                        .families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .push(name.clone());
                    font_defs.font_data.insert(
                        name.clone(),
                        std::sync::Arc::new(egui::FontData::from_owned(data.clone())),
                    );
                }
                cc.egui_ctx.set_fonts(font_defs);
                log::info!("Loaded {} Noto font(s) for non-Latin scripts", font_count);
            }
            // Bump the UI a bit beyond egui's default sizing. Composes with
            // the OS-reported DPI scale (native_pixels_per_point), so HiDPI
            // is still honored — this is an additional user-level zoom.
            cc.egui_ctx.set_zoom_factor(1.20);
            // Register egui context with pidlock so the socket listener
            // can wake egui up on focus requests (otherwise the atomic
            // never gets consumed while the window is unfocused).
            pidlock::register_egui_ctx(cc.egui_ctx.clone());
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}

/// Build the initial `ViewportBuilder` for `mode`:
///   - Wizard: windowed square at 80% of the primary monitor's smaller axis.
///   - Launcher desktop: borderless fullscreen on the primary monitor.
///   - Launcher cabinet: borderless fullscreen + CW90 on the Playfield monitor.
///
/// Returns `(viewport, want_kiosk_cursor)`. The kiosk-cursor flag lets the
/// caller enable cursor lock + focus-reclaim on the App instance only when
/// we've successfully set up cabinet mode.
fn build_viewport(
    displays: &[screens::DisplayInfo],
    mode: app::AppMode,
    vpx_config: &config::VpxConfig,
) -> (egui::ViewportBuilder, bool) {
    let primary_idx = displays.iter().position(|d| d.is_primary).unwrap_or(0);

    let cabinet_mode =
        matches!(mode, app::AppMode::Launcher) && vpx_config.get_i32("Player", "BGSet") == Some(1);
    let playfield_name = vpx_config.get("Player", "PlayfieldDisplay");
    let playfield_idx = if cabinet_mode {
        playfield_name
            .as_ref()
            .and_then(|name| displays.iter().position(|d| &d.name == name))
    } else {
        None
    };

    // Initial size:
    //  - Wizard: square window, 80% of primary's smaller axis
    //  - Launcher: full target monitor (with_monitor promotes to borderless FS)
    let initial_size: [f32; 2] = if matches!(mode, app::AppMode::Wizard) {
        displays
            .iter()
            .find(|d| d.is_primary)
            .or_else(|| displays.first())
            .map(|d| {
                let side = 0.80 * (d.width.min(d.height)) as f32;
                [side, side]
            })
            .unwrap_or([864.0, 864.0])
    } else {
        let target_idx = if cabinet_mode {
            playfield_idx.unwrap_or(primary_idx)
        } else {
            primary_idx
        };
        displays
            .get(target_idx)
            .map(|d| [d.width as f32, d.height as f32])
            .unwrap_or([1920.0, 1080.0])
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("PinReady v{VERSION}"))
        .with_inner_size(initial_size);

    let mut want_kiosk_cursor = false;
    if matches!(mode, app::AppMode::Launcher) {
        viewport = viewport.with_decorations(false);
        if cabinet_mode {
            viewport = viewport.with_rotation(eframe::emath::ViewportRotation::CW90);
            if let Some(idx) = playfield_idx {
                log::info!("Cabinet mode: rotating launcher CW90 on monitor index {idx}");
                viewport = viewport.with_monitor(idx);
                want_kiosk_cursor = true;
            } else {
                log::warn!(
                    "Cabinet mode: Playfield display not found, rotation applied without repositioning"
                );
            }
        } else {
            log::info!(
                "Launcher desktop mode: borderless fullscreen on monitor index {primary_idx}"
            );
            viewport = viewport.with_monitor(primary_idx);
        }
    }
    (viewport, want_kiosk_cursor)
}
