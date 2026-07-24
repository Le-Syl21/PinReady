// PinReady — Visual Pinball configuration wizard & table launcher
// Copyright (C) 2026 — Licensed under GPLv3+
// See https://www.gnu.org/licenses/gpl-3.0.html

//! PinReady — Visual Pinball configuration wizard & table launcher.
//!
//! # Community & support
//!
//! Questions, bugs, beta testing — join the Discord: <https://discord.gg/T37DYHmt2j>
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

rust_i18n::i18n!("locales", fallback = "en");

mod app;
mod assets;
mod audio;
mod config;
mod db;
mod display_id;
mod display_reconcile;
mod i18n;
mod inputs;
mod mediadb;
mod merge;
mod outputs_hid;
mod pidlock;
mod scan_worker;
mod screens;
mod session;
mod system_info;
mod tilt;
mod updater;
mod vbs_patches;
mod vpsdb;
mod wayland_caps;

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
    //
    // We previously had `egui_winit=error` here to silence the per-frame
    // `WARN egui_winit] CursorGrab(Locked): the requested operation is
    // not supported by Winit` on Wayland. Moot since egui-rotate 1.1: the
    // plugin owns the grab, sends it only on capture/release transitions,
    // and uses a per-platform mode that winit actually supports (see
    // `os_grab` below) — egui_winit warns at default `warn` level again.
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
    // Align both SDL3 (used by screens.rs / audio.rs / inputs.rs) and
    // winit/eframe with the session's native display server. winit's
    // Linux auto-detection picks X11 in mixed-backend sessions even when
    // WAYLAND_DISPLAY is set, routing the whole launcher through
    // XWayland for no benefit — verified on Ubuntu 24.04 GNOME by
    // probing /proc/<pid>/fd vs gnome-shell/Xwayland socket owners.
    // Pinning SDL_VIDEODRIVER=wayland early has the observed side-effect
    // of flipping winit too (SDL initialises before eframe and opens a
    // Wayland connection first, which winit then aligns with). Respect
    // an explicit override if the user has already set the variable.
    #[cfg(target_os = "linux")]
    if std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("SDL_VIDEODRIVER").is_err() {
        // SAFETY: called before any thread that might read this env var
        // (SDL3, eframe, winit) has been spawned. Single-threaded at this
        // point in main().
        unsafe {
            std::env::set_var("SDL_VIDEODRIVER", "wayland");
        }
    }

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
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--print-paths") {
        return run_print_paths_cli();
    }
    if args.iter().any(|a| a == "--list-tables") {
        return run_list_tables_cli();
    }
    // Short-lived helper: enumerate displays under a specific SDL video driver
    // so PinReady can learn the identifiers VPX will see under that driver
    // (wayland vs x11 name differently). Run as a subprocess to keep the driver
    // isolated from the main UI's winit/wayland stack.
    if let Some(pos) = args.iter().position(|a| a == "--enumerate-displays") {
        let driver = args.get(pos + 1).cloned().unwrap_or_default();
        return run_enumerate_displays_cli(&driver);
    }
    if args.iter().any(|a| a == "--probe-display") {
        return run_probe_display_cli();
    }
    if args.iter().any(|a| a == "--merge-dry-run") {
        return run_merge_cli(&args, merge::MergeMode::DryRun);
    }
    if args.iter().any(|a| a == "--merge") {
        return run_merge_cli(&args, merge::MergeMode::Commit);
    }
    if args.iter().any(|a| a == "--reset-wizard") {
        return run_reset_wizard_cli();
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
    // After a self-update the previous instance is *still alive* for the
    // brief window between `self_replace::self_replace` and its `exit(0)`.
    // The freshly-spawned new instance receives `--from-update` so it can
    // retry acquiring the flock instead of bailing on the first
    // AlreadyRunning the way a normal second launch should.
    let from_update = args.iter().any(|a| a == "--from-update");
    if from_update {
        eprintln!(
            "PinReady starting with --from-update (PID {})",
            std::process::id()
        );
    }
    let mut from_update_retries = 0u32;
    let _pid_lock = {
        const MAX_RETRIES: u32 = 50; // 50 × 100 ms = 5 s
        loop {
            match pidlock::PidLock::acquire_in(&lock_dir) {
                Ok(lock) => break lock,
                Err(pidlock::PidLockError::AlreadyRunning { path, pid }) => {
                    if from_update && from_update_retries < MAX_RETRIES {
                        from_update_retries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                    if from_update {
                        eprintln!(
                            "PinReady --from-update: gave up waiting for previous instance to \
                             release the lock after {} retries (PID {})",
                            from_update_retries,
                            pid.map(|p| p.to_string())
                                .unwrap_or_else(|| "?".to_string())
                        );
                    }
                    // Another PinReady is live. Politely ask it to bring
                    // its window to the front (via the focus-on-relaunch
                    // Unix socket) and exit cleanly. Only stderr here —
                    // init_logging isn't set up yet, which is intentional:
                    // leaves the running instance's log intact.
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
            }
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
    if from_update {
        log::info!(
            "Started via --from-update; acquired PID lock after {} retries ({} ms)",
            from_update_retries,
            from_update_retries * 100
        );
    }

    // SDL3 is initialized on demand by each consumer:
    //   - VIDEO: lazy-init inside `screens::enumerate_displays`.
    //   - JOYSTICK: owned by the joystick polling thread.
    //   - AUDIO: owned by the audio playback thread.
    // Teardown is centralized: `App::shutdown_sdl_threads` joins both
    // worker threads and calls `SDL_Quit()` before each VPX spawn,
    // wiping every subsystem + open device in one shot. After VPX
    // exits, the threads are respawned and re-init their subsystems
    // from scratch.

    // Belt-and-suspenders atexit: on a clean PinReady shutdown (user
    // closes the launcher, wizard finishes without launching a table,
    // panic that unwinds main, etc.) the Drop guard runs `SDL_Quit()`
    // on the main thread one last time. SDL_Quit is documented as
    // safe to call when SDL is already torn down ("safe to call this
    // function even in the case of errors in initialization") so it
    // overlaps cleanly with `App::shutdown_sdl_threads`'s mid-session
    // SDL_Quit before VPX spawn. The wiki recommends this pattern via
    // atexit() but warns against it in libraries — PinReady is a
    // standalone binary so the warning doesn't apply.
    // https://wiki.libsdl.org/SDL3/SDL_Quit
    struct SdlQuitGuard;
    impl Drop for SdlQuitGuard {
        fn drop(&mut self) {
            unsafe { sdl3_sys::everything::SDL_Quit() };
            log::info!("SDL_Quit() called from atexit guard");
        }
    }
    let _sdl_quit_guard = SdlQuitGuard;

    // Determine the initial mode once from CLI + DB, then loop: each mode
    // owns its own eframe session with a viewport built at window creation
    // time. Switching modes = close current session, main reads the signal
    // set by `crate::app::request_mode_switch`, restart with fresh state.
    // This avoids any live-viewport mutation and its stale-compositor
    // artifacts (double rotation, half-rendered frames).
    let initial_mode = {
        let db = db::Database::open(None)?;
        let configured = db.get_config("wizard_completed").as_deref() == Some("true");

        // A Wayland↔X11 session change no longer needs special handling: at
        // launcher startup `choose_driver_and_reconcile` re-resolves the
        // `*Display=` names to the current driver, and the resolvability check
        // below is anchor-aware. So the switch is transparent — no wizard, no
        // notice modal.

        // The wizard writes VPinballX.ini at completion. If the user (or a
        // cleanup tool) deleted it later, the wizard_completed flag in the
        // DB is stale — the launcher would start with no VPX config and
        // crash or misbehave. Re-run the wizard to regenerate the .ini.
        let ini_present = config::default_ini_path().is_file();
        if force_config || !configured || !ini_present {
            if configured && !ini_present {
                log::warn!(
                    "wizard_completed=true but VPinballX.ini not found at {} — re-running wizard",
                    config::default_ini_path().display()
                );
            }
            app::AppMode::Wizard
        } else {
            // Sanity-check the persisted *Display= names in VPinballX.ini
            // against currently-connected displays. Two cases trigger a
            // mismatch and force the wizard:
            //   1. the user switched between X11 and Wayland sessions
            //      (SDL_GetDisplayName builds the name differently)
            //   2. a configured monitor is no longer connected.
            // Cable swaps and port changes are *not* mismatches: the SDL
            // name follows the EDID, not the connector.
            match config::VpxConfig::load(None) {
                Ok(cfg) => {
                    let connected = screens::enumerate_displays();
                    // Anchor-aware: a role resolves if its stored anchor (layout
                    // position or EDID fingerprint) matches a connected display,
                    // regardless of the ini's current `*Display=` name. So an
                    // X11↔Wayland transition (or a launch that reconciled names
                    // to the other driver) no longer forces the wizard — only a
                    // genuinely absent monitor does.
                    let missing =
                        display_reconcile::unresolvable_assigned_displays(&cfg, &db, &connected);
                    if missing.is_empty() {
                        app::AppMode::Launcher
                    } else {
                        log::warn!(
                            "Configured display(s) not resolvable to a connected monitor: \
                             {missing:?} — unplugged screen, re-running wizard"
                        );
                        app::AppMode::Wizard
                    }
                }
                Err(e) => {
                    log::error!("Failed to read VPinballX.ini for display sanity check: {e} — re-running wizard");
                    app::AppMode::Wizard
                }
            }
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

fn print_help() {
    println!("PinReady v{VERSION} — Visual Pinball configurator & launcher");
    println!();
    println!("Usage: pinready [OPTIONS]");
    println!();
    println!("Run with no options to launch the wizard or table launcher.");
    println!();
    println!("Wizard / launcher control");
    println!("  --config, -c                    Force configuration wizard mode");
    println!("  --reset-wizard                  Mark wizard as not completed and exit");
    println!("                                  (next launch goes back to the wizard).");
    println!();
    println!("Asset merge (legacy folder import)");
    println!("  --merge-dry-run TABLES VPINMAME PUPVIDEOS MUSIC [--strategy MODE]");
    println!("                                  Detect what would be placed; touch nothing.");
    println!("  --merge TABLES VPINMAME PUPVIDEOS MUSIC [--strategy MODE] [--yes]");
    println!("                                  Same, but actually apply. --yes skips the");
    println!("                                  interactive confirmation. MODE is one of");
    println!("                                  copy (default), move, symlink.");
    println!("                                  Use \"\" to skip a source root.");
    println!();
    println!("Diagnostics");
    println!("  --print-paths                   Print resolved DB / log / ini / tables /");
    println!("                                  VPX-binary paths and exit.");
    println!("  --list-tables                   Print one line per detected table folder");
    println!("                                  (folder name + .vpx filename if present).");
    println!();
    println!("Internal");
    println!("  --from-update                   Relaunch from auto-update (retries the PID");
    println!("                                  lock for 5 s while the previous instance");
    println!("                                  finishes exiting). Set automatically by the");
    println!("                                  updater — you should not need it.");
    println!();
    println!("General");
    println!("  --version, -v                   Show version and license");
    println!("  --help, -h                      Show this help");
}

/// Print resolved paths PinReady operates on.
fn run_print_paths_cli() -> Result<()> {
    let db_path = db::default_db_path();
    let log_dir = db_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let log_path = log_dir.join("PinReady.log");
    let ini_path = config::default_ini_path();
    let pid_path = log_dir.join("PinReady.pid");

    let (tables_dir, vpx_exe_path) = match db::Database::open(None) {
        Ok(db) => (
            db.get_tables_dir().unwrap_or_default(),
            db.get_config("vpx_exe_path").unwrap_or_default(),
        ),
        Err(_) => (String::new(), String::new()),
    };
    let bin = std::env::current_exe().unwrap_or_default();

    println!("binary           = {}", bin.display());
    println!("database         = {}", db_path.display());
    println!("log              = {}", log_path.display());
    println!("pid lock         = {}", pid_path.display());
    println!("VPinballX.ini    = {}", ini_path.display());
    println!("tables_dir       = {tables_dir}");
    println!("vpx_exe_path     = {vpx_exe_path}");
    Ok(())
}

/// Enumerate displays under `driver` and print one TSV row per display:
/// `name<TAB>x<TAB>y<TAB>w<TAB>h<TAB>refresh<TAB>width_mm<TAB>height_mm`.
///
/// PinReady spawns this as a subprocess (`--enumerate-displays x11|wayland`)
/// so SDL's video driver is isolated from the main process's winit/Wayland
/// stack: the identifiers printed are exactly what VPX will match when it runs
/// under the same driver.
fn run_enumerate_displays_cli(driver: &str) -> Result<()> {
    if !matches!(driver, "wayland" | "x11") {
        eprintln!("usage: pinready --enumerate-displays <wayland|x11>");
        std::process::exit(2);
    }
    // Override any inherited/forced value before SDL initialises its video
    // subsystem inside `enumerate_displays`. Safe: single-threaded CLI path,
    // no SDL/winit thread started yet.
    unsafe {
        std::env::set_var("SDL_VIDEODRIVER", driver);
    }
    for d in screens::enumerate_displays() {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            d.name, d.x, d.y, d.width, d.height, d.refresh_rate, d.width_mm, d.height_mm
        );
    }
    Ok(())
}

/// Human diagnostic for the display/driver decision: session type, whether the
/// compositor advertises `wp_fifo_v1`, the resulting VPX driver choice, the
/// kernel DRM monitors (driver-independent EDID identity), and the SDL displays
/// as seen under the current driver.
fn run_probe_display_cli() -> Result<()> {
    let session = session::detect();
    println!("session         : {}", session.unwrap_or("(none)"));
    println!("wp_fifo_v1      : {}", wayland_caps::supports_fifo_v1());
    println!(
        "VPX driver      : {}",
        wayland_caps::preferred_vpx_driver(session).unwrap_or("(SDL default)")
    );
    println!("\nDRM monitors (kernel EDID — driver-independent):");
    for m in display_id::read_drm_monitors() {
        println!(
            "  {:<12} {:<18} serial={:<10} fp={}…",
            m.connector,
            m.id.label(),
            m.id.serial,
            &m.id.fingerprint[..12]
        );
    }
    println!("\nSDL displays (current driver):");
    for d in screens::enumerate_displays() {
        println!(
            "  {:<28} {}x{}@{:.2} +{}+{}  {}x{}mm",
            d.name, d.width, d.height, d.refresh_rate, d.x, d.y, d.width_mm, d.height_mm
        );
    }
    Ok(())
}

/// Print one line per table detected under the configured `tables_dir`.
fn run_list_tables_cli() -> Result<()> {
    let db = db::Database::open(None)?;
    let tables_dir = db.get_tables_dir().unwrap_or_default();
    if tables_dir.is_empty() {
        eprintln!("tables_dir is not configured. Run `pinready --config` first.");
        std::process::exit(1);
    }
    let root = std::path::Path::new(&tables_dir);
    if !root.is_dir() {
        eprintln!("tables_dir does not exist: {tables_dir}");
        std::process::exit(1);
    }
    let mut entries: Vec<_> = std::fs::read_dir(root)?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let folder = e.file_name().to_string_lossy().into_owned();
        let vpx = std::fs::read_dir(e.path())
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .find(|f| f.path().extension().and_then(|s| s.to_str()) == Some("vpx"))
            .map(|f| f.file_name().to_string_lossy().into_owned())
            .unwrap_or_else(|| "(no .vpx)".into());
        println!("{folder}\t{vpx}");
    }
    Ok(())
}

/// Mark the wizard as not completed so the next normal launch lands
/// back in the wizard.
fn run_reset_wizard_cli() -> Result<()> {
    let db = db::Database::open(None)?;
    db.set_config("wizard_completed", "false")?;
    println!("wizard_completed = false. Next launch will start the wizard.");
    Ok(())
}

/// Headless merge runner — dry-run or commit. Skips PID lock, SDL, eframe.
/// Positional args: TABLES VPINMAME PUPVIDEOS MUSIC. Optional `--strategy
/// copy|move|symlink` (default copy). `--yes` skips the commit-mode
/// confirmation prompt.
fn run_merge_cli(args: &[String], mode: merge::MergeMode) -> Result<()> {
    let flag = match mode {
        merge::MergeMode::DryRun => "--merge-dry-run",
        merge::MergeMode::Commit => "--merge",
    };
    let pos = args.iter().position(|a| a == flag).unwrap();

    let mut positional = Vec::with_capacity(4);
    let mut i = pos + 1;
    while positional.len() < 4 {
        let Some(arg) = args.get(i) else {
            break;
        };
        if arg.starts_with("--") {
            break;
        }
        positional.push(arg.clone());
        i += 1;
    }
    if positional.len() < 4 {
        anyhow::bail!(
            "{flag}: expected 4 positional args (TABLES VPINMAME PUPVIDEOS MUSIC), got {}",
            positional.len()
        );
    }

    let strategy = match args
        .iter()
        .position(|a| a == "--strategy")
        .and_then(|p| args.get(p + 1))
        .map(|s| s.as_str())
    {
        None | Some("copy") => merge::MergeStrategy::Copy,
        Some("move") => merge::MergeStrategy::Move,
        Some("symlink") => merge::MergeStrategy::Symlink,
        Some(other) => {
            anyhow::bail!("--strategy: unknown value '{other}' (expected copy, move, or symlink)")
        }
    };
    let assume_yes = args.iter().any(|a| a == "--yes" || a == "-y");

    let opt = |s: &str| -> Option<std::path::PathBuf> {
        if s.trim().is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(s))
        }
    };
    let tables = std::path::PathBuf::from(&positional[0]);
    let sources = merge::MergeSources {
        vpinmame: opt(&positional[1]),
        pupvideos: opt(&positional[2]),
        music: opt(&positional[3]),
    };

    let mode_label = match mode {
        merge::MergeMode::DryRun => "dry-run",
        merge::MergeMode::Commit => "commit",
    };
    println!("[merge {mode_label}] tables    = {}", tables.display());
    println!(
        "[merge {mode_label}] sources   = vpinmame={:?} pupvideos={:?} music={:?}",
        sources.vpinmame, sources.pupvideos, sources.music
    );
    println!("[merge {mode_label}] strategy  = {}", strategy.as_db_str());

    if matches!(mode, merge::MergeMode::Commit) && !assume_yes {
        eprint!(
            "About to {} files into table folders. Continue? [y/N] ",
            strategy.as_db_str()
        );
        use std::io::Write as _;
        let _ = std::io::stderr().flush();
        let mut buf = String::new();
        std::io::stdin().read_line(&mut buf)?;
        let answer = buf.trim().to_ascii_lowercase();
        if answer != "y" && answer != "yes" {
            eprintln!("Aborted.");
            return Ok(());
        }
    }

    let (rx, _cancel, handle) = merge::spawn(tables, sources, strategy, mode);

    use merge::MergeEvent::*;
    let mut errored = 0usize;
    while let Ok(ev) = rx.recv() {
        match ev {
            TableStarted { name } => println!("▸ {name}"),
            AssetFound { kind, src, dst } => println!(
                "  + {} : {} -> {}",
                kind.label(),
                src.display(),
                dst.display()
            ),
            AssetSkipped { kind, reason } => {
                println!("  · {} ({})", kind.label(), reason.label())
            }
            AssetApplied { kind, dst } => {
                println!("  ✓ {} -> {}", kind.label(), dst.display())
            }
            AssetError { kind, msg } => {
                println!("  ! {} : {msg}", kind.label());
                errored += 1;
            }
            TableDone { .. } => {}
            Done(report) => {
                println!(
                    "[merge {mode_label}] done — tables={} found={} applied={} skipped={} errors={}",
                    report.tables_processed,
                    report.assets_found,
                    report.assets_applied,
                    report.assets_skipped,
                    report.assets_errored
                );
            }
        }
    }
    let _ = handle.join();
    if errored > 0 {
        std::process::exit(2);
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

    let (viewport, want_kiosk_cursor, rotation) = build_viewport(&displays, mode, &vpx_config, &db);
    let start_in_wizard = matches!(mode, app::AppMode::Wizard);
    let mut app = app::App::new(vpx_config, db, start_in_wizard, displays);
    app.set_rotation(rotation);
    if want_kiosk_cursor {
        app.enable_kiosk_cursor();
    }

    let mut options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    // Glow renderer instead of wgpu — wgpu's Vulkan backend on NVIDIA
    // Wayland has a known freeze where presents are throttled to a single
    // initial frame regardless of present_mode hint. Glow uses EGL +
    // glutin which has different presentation pacing.
    options.glow_options.vsync = false;

    eframe::run_native(
        "PinReady",
        options,
        Box::new(move |cc| {
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
            // Register the egui-rotate plugin unconditionally. It owns
            // per-viewport input rotation, output rotation and the
            // software cursor. Registered even when the initial rotation
            // is `None` — the topbar `↻` button flips rotation live and
            // needs the plugin already on the Context to take effect.
            // Passthrough cost is negligible with `Rotation::None`.
            //
            // The `SoftwareCursor` is also attached unconditionally: the
            // plugin only activates it when a viewport is rotated, so
            // wizard/desktop launcher run with the OS cursor until the
            // user clicks ↻. From that click on, the cursor rotates
            // along with the UI — the OS pointer is upright and would
            // fight the visible axes otherwise.
            //
            // Kiosk cabinets keep the previous 3× lock+scale for far-
            // viewing playfields; wizard/desktop match the OS cursor
            // size at 1.0× so the user can be precise on small widgets
            // (radio buttons, sliders, ini keys). The wizard cursor is
            // not hard-locked so the user can still leave the window
            // (open a file dialog, focus another app…) — since
            // egui-rotate 1.1 it soft-locks instead: the edge resists
            // casual contact but a deliberate push still exits.
            let cursor_scale = if want_kiosk_cursor { 3.0 } else { 1.0 };
            // The plugin owns the OS pointer grab (egui-rotate 1.1): it sends
            // `CursorGrab` on capture/release transitions, so the old manual
            // per-frame grab loop and its Linux opt-out are gone. Mode per
            // platform (winit support matrix): `Confined` exists on Wayland,
            // X11 and Windows; macOS only implements `Locked`.
            let os_grab = if cfg!(target_os = "macos") {
                egui::viewport::CursorGrab::Locked
            } else {
                egui::viewport::CursorGrab::Confined
            };
            let plugin = egui_rotate::RotationPlugin::new(rotation).with_software_cursor(
                egui_rotate::SoftwareCursor::new()
                    .with_scale(cursor_scale)
                    .with_lock(want_kiosk_cursor)
                    .with_os_grab(Some(os_grab))
                    // Launcher only: keyboard navigation parks the cursor
                    // (dissolve + hover cleared) so a pointer resting on a
                    // card can't override flipper-key selection; the joystick
                    // path does the same via `set_dormant` in
                    // `handle_launcher_joystick`. Any mouse use reforms it in
                    // place. The wizard keeps the cursor visible while typing
                    // — it's a form UI, hiding on every keystroke would be
                    // distracting.
                    .with_dormant_on_keys(!start_in_wizard),
            );
            cc.egui_ctx.add_plugin(plugin);
            // Funnel keyboard input from the BG/DMD/Topper cover viewports to the
            // playfield (ROOT). Under Mutter Wayland a freshly-mapped cover
            // viewport can steal keyboard focus despite `with_active(false)`;
            // this keeps flipper keys and launcher navigation reaching the
            // playfield. Mouse follows the focused window; joystick is read via
            // SDL, independent of focus. (Replaces the old eframe kiosk routing.)
            cc.egui_ctx.add_plugin(egui_keyfunnel::KeyFunnel::new());
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
    db: &db::Database,
) -> (egui::ViewportBuilder, bool, egui_rotate::Rotation) {
    let primary_idx = displays.iter().position(|d| d.is_primary).unwrap_or(0);

    let cabinet_mode =
        matches!(mode, app::AppMode::Launcher) && vpx_config.get_i32("Player", "BGSet") == Some(1);
    let playfield_name = vpx_config.get("Player", "PlayfieldDisplay");
    // Find the Playfield monitor for `with_monitor` (borderless-fullscreen
    // targeting). This runs *before* the launch-time name reconciliation, so
    // the ini's `PlayfieldDisplay` may still hold a name from another driver
    // that won't match the current SDL enumeration (e.g. an X11 name under a
    // Wayland session) — in which case the window would open non-fullscreen and
    // leave the GNOME panels showing. Fall back to the driver-independent EDID
    // geometry anchor so the fullscreen target resolves either way.
    let playfield_idx = if cabinet_mode {
        playfield_name
            .as_ref()
            .and_then(|name| displays.iter().position(|d| &d.name == name))
            .or_else(|| {
                display_reconcile::anchored_display_index(
                    db,
                    screens::DisplayRole::Playfield,
                    displays,
                )
            })
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
    let mut rotation = egui_rotate::Rotation::None;
    if matches!(mode, app::AppMode::Launcher) {
        viewport = viewport.with_decorations(false);
        if cabinet_mode {
            rotation = egui_rotate::Rotation::CW90;
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
    (viewport, want_kiosk_cursor, rotation)
}
