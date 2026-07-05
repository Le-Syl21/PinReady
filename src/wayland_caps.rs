//! Wayland compositor capability probe.
//!
//! The one capability that matters for PinReady is **`wp_fifo_v1`**: without
//! it, SDL3 on native Wayland cannot delegate FIFO (vsync) timing to the
//! compositor and falls back to frame-callback presentation, which Mutter
//! throttles to a fraction of the refresh rate (the "capped at 30 fps on a
//! 120 Hz playfield" symptom). The protocol landed in Mutter 48 / GNOME 48;
//! older compositors do not advertise it.
//!
//! When it is absent, the launch path can run VPX under XWayland (X11) instead,
//! which does not suffer the same throttle — the framerate escape hatch.
//!
//! Detection is a plain Wayland registry enumeration: connect, list the
//! advertised globals, look for `wp_fifo_manager_v1`. No rendering, no window.

/// Whether the current Wayland compositor advertises `wp_fifo_manager_v1`.
///
/// Returns `false` when not on Wayland, when the connection fails, or when the
/// global is absent — i.e. "assume no proper FIFO" on any uncertainty, which is
/// the safe default for the framerate decision.
#[cfg(target_os = "linux")]
pub fn supports_fifo_v1() -> bool {
    use wayland_client::{
        globals::{registry_queue_init, GlobalListContents},
        protocol::wl_registry,
        Connection, Dispatch, QueueHandle,
    };

    // The registry events are fully handled by `GlobalListContents`; our state
    // only has to satisfy the `Dispatch` bound with an empty handler.
    struct Probe;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    let Ok(conn) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _queue)) = registry_queue_init::<Probe>(&conn) else {
        return false;
    };
    globals
        .contents()
        .with_list(|list| list.iter().any(|g| g.interface == "wp_fifo_manager_v1"))
}

#[cfg(not(target_os = "linux"))]
pub fn supports_fifo_v1() -> bool {
    false
}

/// The SDL video driver VPX should be launched under, given the session type
/// and compositor capabilities:
///
/// - Wayland **with** `wp_fifo_v1` → `"wayland"` (native, correct FIFO).
/// - Wayland **without** it → `"x11"` (XWayland escape hatch, avoids the
///   frame-callback throttle at the cost of going through the X11 protocol).
/// - Native X11 session → `"x11"`.
/// - Anything else (non-Linux, headless) → `None` (let SDL decide).
///
/// This is the single decision point; display enumeration and the VPX launch
/// env are both derived from it so they can never disagree.
pub fn preferred_vpx_driver(session: Option<&str>) -> Option<&'static str> {
    match session {
        Some("wayland") if supports_fifo_v1() => Some("wayland"),
        Some("wayland") => Some("x11"),
        Some("x11") => Some("x11"),
        _ => None,
    }
}
