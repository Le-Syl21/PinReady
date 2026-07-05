//! Display server session type (Linux only).
//!
//! Used as a fallback signal for the SDL video driver: newer SDL3 builds
//! shipped with VPX prefer XWayland when a Wayland compositor is around,
//! which routes VPX through the X11 protocol and re-introduces the
//! display-placement bugs PinReady exists to avoid. The primary driver
//! decision lives in `wayland_caps::preferred_vpx_driver` (which also
//! accounts for `wp_fifo_v1`); this is the plain session read it builds on.
//!
//! (There is no longer any X11↔Wayland "session changed, re-run the wizard"
//! logic — display roles are re-resolved by geometry/EDID anchor in
//! `display_reconcile`, so a session switch is transparent.)
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

