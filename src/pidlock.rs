//! Single-instance PID lock + raise-existing-window signal.
//!
//! Two guarantees rolled into one:
//!
//! 1. **Single instance.** Only one PinReady can hold the flock on
//!    `PinReady.pid`. A second launch observes the lock is held and bails.
//!
//! 2. **Focus-on-relaunch.** Instead of the second launch dying silently,
//!    it connects to a Unix socket next to the PID file and sends
//!    `"focus\n"`. The running instance's socket thread sets an atomic,
//!    the egui loop picks it up and sends `Focus + Minimized(false)` to
//!    its own viewport so the existing window comes to the front.
//!
//! Held for the whole process lifetime. The flock is released by the OS
//! on `kill -9` (no stale lock), and the socket file is unlinked by Drop
//! on clean shutdown.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Filename of the lock. Same directory as `PinReady.log` / the DB.
const LOCK_FILENAME: &str = "PinReady.pid";
/// Unix socket filename used by a second-launch to raise the running one.
#[cfg(unix)]
const SOCKET_FILENAME: &str = "PinReady.sock";

/// Set by the socket listener whenever a second-launch asked us to raise
/// our window. `App::ui` polls and consumes this each frame; when true it
/// issues `Focus` + `Minimized(false)` viewport commands.
static FOCUS_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Registered by eframe's creation closure once per session. Lets the
/// socket-listener thread wake up egui when a focus request arrives —
/// without this, egui's event loop stays idle when the window doesn't
/// have focus, so the atomic never gets consumed.
///
/// Re-registered on every eframe restart (wizard↔launcher mode switch).
static EGUI_CTX: Mutex<Option<eframe::egui::Context>> = Mutex::new(None);

/// Register the egui context so the socket listener can call
/// `request_repaint` on it. Call this once from the eframe creation
/// closure, passing `cc.egui_ctx.clone()`.
pub fn register_egui_ctx(ctx: eframe::egui::Context) {
    *EGUI_CTX.lock().unwrap() = Some(ctx);
}

/// `App::ui` calls this each frame. Returns true (and clears the flag) when
/// another instance recently asked us to raise our window.
pub fn take_focus_request() -> bool {
    FOCUS_REQUESTED.swap(false, Ordering::Relaxed)
}

pub struct PidLock {
    /// Kept alive to hold the flock; released on Drop.
    _file: File,
    path: PathBuf,
    #[cfg(unix)]
    socket_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum PidLockError {
    /// Couldn't create/open the lock file.
    OpenFailed(std::io::Error),
    /// Another instance is running. The file contents (if readable) give
    /// the PID so the user can kill it without `ps`-digging.
    AlreadyRunning { path: PathBuf, pid: Option<u32> },
}

impl PidLock {
    /// Try to acquire the lock. The file lives in `dir`.
    pub fn acquire_in(dir: &Path) -> Result<Self, PidLockError> {
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join(LOCK_FILENAME);

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(PidLockError::OpenFailed)?;

        match file.try_lock() {
            Ok(()) => {
                // We own the lock. Overwrite with our PID.
                let _ = file.set_len(0);
                let _ = file.seek(std::io::SeekFrom::Start(0));
                let _ = writeln!(file, "{}", std::process::id());

                // Start the focus-on-relaunch socket listener. Failures here
                // are logged and ignored — the lock itself still works.
                #[cfg(unix)]
                let socket_path = start_focus_listener(dir);

                Ok(PidLock {
                    _file: file,
                    path,
                    #[cfg(unix)]
                    socket_path,
                })
            }
            Err(_) => {
                // Another live process holds the flock. Read its PID so
                // the error message is actionable.
                let mut content = String::new();
                let pid = File::open(&path)
                    .ok()
                    .and_then(|mut f| f.read_to_string(&mut content).ok().map(|_| content))
                    .and_then(|s| s.trim().parse::<u32>().ok());
                Err(PidLockError::AlreadyRunning { path, pid })
            }
        }
    }
}

/// Try to tell the currently-running instance to focus its window. Returns
/// `true` if the message was sent successfully. Called by a failed-to-lock
/// second launch before giving up — lets a double-click on the app icon
/// raise the existing window instead of just bailing silently.
#[cfg(unix)]
pub fn try_notify_focus(dir: &Path) -> bool {
    use std::os::unix::net::UnixStream;
    let sock_path = dir.join(SOCKET_FILENAME);
    match UnixStream::connect(&sock_path) {
        Ok(mut stream) => {
            let _ = stream.write_all(b"focus\n");
            log::info!(
                "Sent focus request to running instance at {}",
                sock_path.display()
            );
            true
        }
        Err(e) => {
            log::info!("No running instance socket at {}: {e}", sock_path.display());
            false
        }
    }
}

#[cfg(not(unix))]
pub fn try_notify_focus(_dir: &Path) -> bool {
    // Windows/other: unimplemented. Caller falls back to the standard
    // "already running" exit.
    false
}

/// Bind the Unix socket and spawn a listener thread that translates any
/// connection into a `FOCUS_REQUESTED` flag. Returns the socket path on
/// success so `Drop` can unlink it. Any failure is logged and the
/// function returns `None` — the PID lock still works either way.
#[cfg(unix)]
fn start_focus_listener(dir: &Path) -> Option<PathBuf> {
    use std::io::BufRead;
    use std::os::unix::net::UnixListener;

    let sock_path = dir.join(SOCKET_FILENAME);
    // Remove any stale socket left by a previous crashed instance. Fresh
    // binds fail with EADDRINUSE otherwise.
    let _ = std::fs::remove_file(&sock_path);

    let listener = match UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!(
                "Focus-on-relaunch socket bind failed at {}: {e}. Raising existing window from another launch won't work.",
                sock_path.display()
            );
            return None;
        }
    };
    log::info!(
        "Focus-on-relaunch socket listening at {}",
        sock_path.display()
    );

    std::thread::Builder::new()
        .name("pinready-focus-listener".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let reader = std::io::BufReader::new(stream);
                // We don't actually require the message content — any
                // connection is a focus request. Reading a line just drains
                // the socket cleanly.
                if let Some(Ok(line)) = reader.lines().next() {
                    log::info!("Focus request received: {line:?}");
                }
                FOCUS_REQUESTED.store(true, Ordering::Relaxed);

                // Wake egui up. Without this, the main thread stays idle
                // until a real input event arrives (mouse enter, keyboard,
                // timer) — which on an unfocused window might not happen
                // for a long time. request_repaint forces one frame so
                // `take_focus_request` runs and the viewport commands fire.
                if let Some(ctx) = EGUI_CTX.lock().unwrap().as_ref() {
                    ctx.request_repaint();
                }
            }
        })
        .map_err(|e| log::warn!("Focus listener thread spawn failed: {e}"))
        .ok();

    Some(sock_path)
}

impl Drop for PidLock {
    fn drop(&mut self) {
        // Release the flock (implicit on File drop) and remove both the
        // PID file and the socket so a clean shutdown leaves no trace. On
        // `kill -9` Drop doesn't run — the files stay but the flock is
        // released by the OS, and a stale socket is re-bound fresh on the
        // next launch.
        let _ = std::fs::remove_file(&self.path);
        #[cfg(unix)]
        if let Some(sock) = &self.socket_path {
            let _ = std::fs::remove_file(sock);
        }
    }
}
