// PinReady — VPinballX configuration wizard & table launcher
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

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
        println!("PinReady v{VERSION} — VPinballX configurator & launcher");
        println!();
        println!("Usage: pinready [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --config, -c    Force configuration wizard mode");
        println!("  --version, -v   Show version and license");
        println!("  --help, -h      Show this help");
        return Ok(());
    }

    env_logger::init();
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
    // - No VPX ini file → wizard (first run)
    // - Otherwise → launcher
    let ini_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".local/share/VPinballX/10.8/VPinballX.ini");
    let start_in_wizard = force_config || !ini_path.exists();
    if start_in_wizard {
        log::info!("Starting in configuration wizard mode");
    } else {
        log::info!("Starting in launcher mode");
    }

    // Create app (starts joystick + audio threads internally)
    let app = app::App::new(vpx_config, db, start_in_wizard);

    // Launch eframe
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("PinReady v{VERSION}"))
            .with_inner_size([1000.0, 1000.0]),
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
