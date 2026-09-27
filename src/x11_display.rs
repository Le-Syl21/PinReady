//! What winit will see on a plain X11 server, asked before it starts.
//!
//! Two things differ between SDL (which enumerates the displays and drives
//! VPX) and winit (which opens our windows) once no desktop session sits in
//! between, e.g. a bare `X :3` + window manager launched from a script:
//!
//! - **Scale.** Without `Xft.dpi` in the X resource database, winit derives
//!   the scale factor from the pixel density the panel claims in millimetres,
//!   while SDL stays at 1.0. A cabinet TV then gets a launcher zoomed well
//!   past its screen, next to a VPX that looks right.
//! - **Order.** SDL lists the primary output first, then the others in RandR
//!   output order. winit lists CRTCs in RandR order and never moves the
//!   primary. `with_monitor(i)` takes winit's index, so an SDL index can land
//!   the launcher on the backglass.
//!
//! Both answers come from the same RandR and resource-manager requests winit
//! makes itself (`x11/monitor.rs`, `x11/util/randr.rs` in winit 0.30).

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;

/// True when this process will talk to an X server directly: `DISPLAY` set
/// and no Wayland socket for winit or SDL to prefer.
pub fn is_plain_x11() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some()
}

/// Whether the X resource database holds `Xft.dpi`. `None` when the server
/// cannot be reached, which leaves every decision to winit as before.
pub fn has_xft_dpi() -> Option<bool> {
    let (conn, _) = x11rb::connect(None).ok()?;
    let db = x11rb::resource_manager::new_from_default(&conn).ok()?;
    Some(db.get_string("Xft.dpi", "").is_some())
}

/// Top-left corner and size of every monitor winit will report, in the order
/// `available_monitors()` returns them: CRTCs that are lit and drive an
/// output, in RandR's CRTC order.
pub fn winit_monitor_rects() -> Option<Vec<(i32, i32, u32, u32)>> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let version = conn.randr_query_version(1, 3).ok()?.reply().ok()?;
    // Same choice as winit: the cheap "current" query from RandR 1.3 on.
    let crtcs = if (version.major_version, version.minor_version) >= (1, 3) {
        conn.randr_get_screen_resources_current(root)
            .ok()?
            .reply()
            .ok()?
            .crtcs
    } else {
        conn.randr_get_screen_resources(root)
            .ok()?
            .reply()
            .ok()?
            .crtcs
    };
    let mut rects = Vec::with_capacity(crtcs.len());
    for crtc in crtcs {
        let info = conn
            .randr_get_crtc_info(crtc, x11rb::CURRENT_TIME)
            .ok()?
            .reply()
            .ok()?;
        if info.width == 0 || info.height == 0 || info.outputs.is_empty() {
            continue;
        }
        rects.push((
            i32::from(info.x),
            i32::from(info.y),
            u32::from(info.width),
            u32::from(info.height),
        ));
    }
    Some(rects)
}

/// For each display, in `displays` order, the index winit gives the same
/// screen. A display that matches no CRTC keeps its own index.
pub fn monitor_indices(
    displays: &[(i32, i32, i32, i32)],
    winit: &[(i32, i32, u32, u32)],
) -> Vec<usize> {
    displays
        .iter()
        .enumerate()
        .map(|(i, &(x, y, w, h))| {
            winit
                .iter()
                .position(|&(wx, wy, ww, wh)| {
                    wx == x
                        && wy == y
                        && i64::from(ww) == i64::from(w)
                        && i64::from(wh) == i64::from(h)
                })
                .unwrap_or(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::monitor_indices;

    #[test]
    fn follows_winit_order_not_sdl_order() {
        // SDL put the primary (the backglass, at x=1920) first; winit lists
        // CRTCs in RandR order, playfield first.
        let sdl = [(1920, 0, 1920, 1080), (0, 0, 1920, 1080)];
        let winit = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)];
        assert_eq!(monitor_indices(&sdl, &winit), vec![1, 0]);
    }

    #[test]
    fn same_order_is_identity() {
        let sdl = [
            (0, 0, 3840, 2160),
            (3840, 0, 1920, 1080),
            (5760, 0, 1280, 390),
        ];
        let winit = [
            (0, 0, 3840, 2160),
            (3840, 0, 1920, 1080),
            (5760, 0, 1280, 390),
        ];
        assert_eq!(monitor_indices(&sdl, &winit), vec![0, 1, 2]);
    }

    #[test]
    fn unmatched_display_keeps_its_index() {
        let sdl = [(0, 0, 1920, 1080), (1920, 0, 1024, 768)];
        let winit = [(0, 0, 1920, 1080)];
        assert_eq!(monitor_indices(&sdl, &winit), vec![0, 1]);
    }
}
