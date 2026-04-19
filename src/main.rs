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
mod screens;
mod tilt;
mod updater;

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
    if let Some(file) = log_file.as_ref().map(|f| f.try_clone().ok()).flatten() {
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
            let ts = buf.timestamp_seconds();
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

fn main() -> Result<()> {
    // Parse arguments
    let args: Vec<String> = std::env::args().collect();
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

    init_logging();
    log::info!("PinReady v{VERSION} starting...");

    // Initialize SDL3 for display enumeration only (joystick + audio handled in their threads)
    unsafe {
        use sdl3_sys::everything::*;
        if !SDL_Init(SDL_INIT_VIDEO) {
            let err = std::ffi::CStr::from_ptr(SDL_GetError()).to_string_lossy();
            anyhow::bail!("Failed to init SDL3: {err}");
        }
    }
    log::info!("SDL3 initialized (video for display enumeration)");

    // Open database
    let db = db::Database::open(None)?;

    // Load VPX config (pre-fill wizard if ini exists)
    let vpx_config = config::VpxConfig::load(None)?;

    // Determine start mode:
    // - --config flag → wizard
    // - DB not marked as configured (first run, wiped DB, etc.) → wizard
    // - Otherwise → launcher
    let configured = db.get_config("wizard_completed").as_deref() == Some("true");
    let start_in_wizard = force_config || !configured;
    if start_in_wizard {
        log::info!("Starting in configuration wizard mode");
    } else {
        log::info!("Starting in launcher mode");
    }

    // Enumerate displays once — shared between kiosk lookup and App state.
    let displays = screens::enumerate_displays();

    // Detect Cabinet mode (BGSet=1) → rotate viewport and place on Playfield.
    // Only applies in launcher mode — the wizard must always run in a standard,
    // non-rotated window since it's where the user configures cabinet mode.
    let cabinet_mode = !start_in_wizard && vpx_config.get_i32("Player", "BGSet") == Some(1);
    let playfield_name = vpx_config.get("Player", "PlayfieldDisplay");
    let playfield_idx = if cabinet_mode {
        playfield_name
            .as_ref()
            .and_then(|name| displays.iter().position(|d| &d.name == name))
    } else {
        None
    };

    // Create app (starts joystick + audio threads internally)
    let mut app = app::App::new(vpx_config, db, start_in_wizard, displays);

    // Launch eframe. Wizard UI is style-scaled x2 so we start with a window
    // large enough to show all breadcrumbs + content comfortably.
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("PinReady v{VERSION}"))
        .with_inner_size([1800.0, 1100.0]);
    if cabinet_mode {
        viewport = viewport
            .with_rotation(eframe::emath::ViewportRotation::CW90)
            .with_decorations(false);
        if let Some(idx) = playfield_idx {
            log::info!(
                "Cabinet mode: rotating launcher CW90 on monitor index {}",
                idx
            );
            viewport = viewport.with_monitor(idx);
            // kiosk_bounds drives cursor lock + warp to grid center after the
            // window is mapped. Placement itself is handled by with_monitor.
            app.enable_kiosk_cursor();
        } else {
            log::warn!(
                "Cabinet mode: Playfield display not found, rotation applied without repositioning"
            );
        }
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
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}
