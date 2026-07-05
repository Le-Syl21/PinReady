//! Display server session type (Linux only).
//!
//! Two uses:
//!   1. Force VPX's SDL backend to match PinReady's actual session (env
//!      var `SDL_VIDEODRIVER`). Newer SDL3 builds shipped with VPX
//!      prefer XWayland when a Wayland compositor is around, which
//!      routes VPX through the X11 protocol and re-introduces the
//!      display-placement bugs PinReady exists to avoid.
//!   2. Detect when the user switched between X11 and Wayland between
//!      two PinReady runs, so the wizard can re-open with a clear
//!      "here's why" instead of a silent reconfiguration.
//!
//! Non-Linux platforms always report `None` — macOS/Windows have a
//! single native session type and nothing to force.

/// Current session's display server, in a form suitable for
/// `SDL_VIDEODRIVER`. Returns `None` outside Linux, or on Linux if
/// neither `WAYLAND_DISPLAY` nor `DISPLAY` is set (e.g. headless
/// systemd unit, container without display forwarding).
pub fn detect() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            return Some("wayland");
        }
        if std::env::var("DISPLAY").is_ok() {
            return Some("x11");
        }
    }
    None
}

