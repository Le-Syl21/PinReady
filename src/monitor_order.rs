//! The order winit lists monitors in, and which SDL display is which.
//!
//! SDL enumerates the displays (and drives VPX); winit opens our windows, and
//! `ViewportBuilder::with_monitor(i)` takes an index into winit's
//! `available_monitors()`. The two libraries do not list screens in the same
//! order:
//!
//! - **Windows.** Both walk `EnumDisplayMonitors`, but SDL walks it twice,
//!   primary first, then the rest (`WIN_AddDisplays` in SDL 3.4); winit keeps
//!   the raw order (`windows/monitor.rs` in winit 0.30), which puts the
//!   primary wherever Windows does. Both report `rcMonitor` from
//!   `GetMonitorInfoW`, under the same per-monitor-v2 DPI awareness that both
//!   set on the process, so the rects are identical.
//! - **macOS.** SDL walks `CGGetOnlineDisplayList`, main display first and
//!   mirrors skipped; winit walks `CGGetActiveDisplayList` as is. Apple puts
//!   the main display first in the active list, so they usually agree, but a
//!   sleeping or mirrored screen can still shift the indices.
//! - **Plain X11.** SDL lists the primary output first, winit lists RandR
//!   CRTCs in order (see [`crate::x11_display`]).
//!
//! Each platform below reports the monitors' rects in winit's order;
//! [`monitor_indices`] then ties every SDL display to the winit monitor at the
//! same place.

/// Top-left corner and size of every monitor winit will report, in the order
/// `available_monitors()` returns them, in the same coordinates SDL's display
/// bounds use. `None` when the platform could not be asked.
#[cfg(target_os = "linux")]
pub fn winit_monitor_rects() -> Option<Vec<(i32, i32, u32, u32)>> {
    crate::x11_display::winit_monitor_rects()
}

/// Same walk as winit's `available_monitors()` on Windows: one
/// `EnumDisplayMonitors` pass, each monitor's `rcMonitor`.
#[cfg(target_os = "windows")]
pub fn winit_monitor_rects() -> Option<Vec<(i32, i32, u32, u32)>> {
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        // SAFETY: `data` is the &mut Vec passed below, alive for the whole
        // enumeration.
        let out = unsafe { &mut *(data.0 as *mut Vec<Option<(i32, i32, u32, u32)>>) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `info` is a correctly sized MONITORINFO.
        let rect = if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let r = info.rcMonitor;
            Some((
                r.left,
                r.top,
                (r.right - r.left).max(0) as u32,
                (r.bottom - r.top).max(0) as u32,
            ))
        } else {
            None
        };
        // A monitor whose rect cannot be read still holds its slot: winit
        // counts it all the same.
        out.push(rect);
        true.into()
    }

    let mut out: Vec<Option<(i32, i32, u32, u32)>> = Vec::new();
    // SAFETY: standard Win32 enumeration; `out` outlives the callback, which
    // only pushes through the pointer we hand it.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(std::ptr::from_mut(&mut out) as isize),
        )
    };
    if !ok.as_bool() || out.is_empty() {
        return None;
    }
    // An unreadable slot becomes an empty rect that matches no display.
    Some(
        out.into_iter()
            .map(|r| r.unwrap_or((i32::MIN, i32::MIN, 0, 0)))
            .collect(),
    )
}

/// Same walk as winit's `available_monitors()` on macOS:
/// `CGGetActiveDisplayList`, each display's `CGDisplayBounds` (points, as
/// SDL's bounds are).
#[cfg(target_os = "macos")]
pub fn winit_monitor_rects() -> Option<Vec<(i32, i32, u32, u32)>> {
    use objc2_core_graphics::{CGDisplayBounds, CGError, CGGetActiveDisplayList};

    let mut count: u32 = 0;
    // SAFETY: the null buffer form is the documented "how many?" call.
    let sized = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
    if sized != CGError::Success || count == 0 {
        return None;
    }
    let mut ids = vec![0u32; count as usize];
    // SAFETY: `ids` holds exactly the count the call above reported.
    let filled = unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) };
    if filled != CGError::Success {
        return None;
    }
    ids.truncate(count as usize);
    Some(
        ids.into_iter()
            .map(|id| {
                let b = CGDisplayBounds(id);
                (
                    b.origin.x as i32,
                    b.origin.y as i32,
                    b.size.width.max(0.0) as u32,
                    b.size.height.max(0.0) as u32,
                )
            })
            .collect(),
    )
}

/// For each display, in `displays` order, the index winit gives the same
/// screen.
///
/// A display whose rect is exactly one of winit's takes that index. Whatever
/// is left over (one side in logical points, the other in pixels, should the
/// two ever disagree on DPI awareness) is paired by layout: scaling moves
/// and resizes screens but keeps them in the same left-to-right,
/// top-to-bottom order. A display that still matches nothing keeps its own
/// index.
pub fn monitor_indices(
    displays: &[(i32, i32, i32, i32)],
    winit: &[(i32, i32, u32, u32)],
) -> Vec<usize> {
    let mut out: Vec<Option<usize>> = displays
        .iter()
        .map(|&(x, y, w, h)| {
            winit.iter().position(|&(wx, wy, ww, wh)| {
                wx == x && wy == y && i64::from(ww) == i64::from(w) && i64::from(wh) == i64::from(h)
            })
        })
        .collect();

    let mut left: Vec<usize> = (0..displays.len()).filter(|&i| out[i].is_none()).collect();
    let mut free: Vec<usize> = (0..winit.len())
        .filter(|j| !out.contains(&Some(*j)))
        .collect();
    if !left.is_empty() && left.len() == free.len() {
        left.sort_by_key(|&i| (displays[i].0, displays[i].1));
        free.sort_by_key(|&j| (winit[j].0, winit[j].1));
        for (i, j) in left.into_iter().zip(free) {
            out[i] = Some(j);
        }
    }

    out.into_iter()
        .enumerate()
        .map(|(i, j)| j.unwrap_or(i))
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

    /// gonzonia's cabinet on Windows (PinReady 0.22.0): SDL lists the
    /// primary TV first, Windows enumerates it last. The launcher opened on
    /// winit #0, the DMD screen.
    #[test]
    fn windows_primary_enumerated_last() {
        let sdl = [
            (0, 0, 3840, 2160),    // LG TV SSCR2, playfield, primary
            (5760, 0, 1920, 1080), // Monitor
            (3840, 0, 1920, 1080), // DELL S2721QS
        ];
        let winit = [
            (5760, 0, 1920, 1080),
            (3840, 0, 1920, 1080),
            (0, 0, 3840, 2160),
        ];
        assert_eq!(monitor_indices(&sdl, &winit), vec![2, 0, 1]);
    }

    /// Same cabinet with the TV at 150 %, SDL reporting logical points and
    /// winit pixels: no rect matches, the layout still does.
    #[test]
    fn scaled_layout_pairs_by_position() {
        let sdl = [
            (0, 0, 2560, 1440),
            (4480, 0, 1920, 1080),
            (2560, 0, 1920, 1080),
        ];
        let winit = [
            (5760, 0, 1920, 1080),
            (3840, 0, 1920, 1080),
            (0, 0, 3840, 2160),
        ];
        assert_eq!(monitor_indices(&sdl, &winit), vec![2, 0, 1]);
    }

    /// Only the scaled screen differs; the two at 100 % match exactly and
    /// the leftover pair is tied up by elimination.
    #[test]
    fn one_scaled_screen_among_exact_ones() {
        let sdl = [
            (0, 0, 2560, 1440),
            (5760, 0, 1920, 1080),
            (3840, 0, 1920, 1080),
        ];
        let winit = [
            (5760, 0, 1920, 1080),
            (3840, 0, 1920, 1080),
            (0, 0, 3840, 2160),
        ];
        assert_eq!(monitor_indices(&sdl, &winit), vec![2, 0, 1]);
    }

    /// A screen stacked above the playfield sorts by height once the
    /// columns tie.
    #[test]
    fn scaled_vertical_stack() {
        let sdl = [(0, 0, 1280, 720), (0, -720, 1280, 720)];
        let winit = [(0, -1080, 1920, 1080), (0, 0, 1920, 1080)];
        assert_eq!(monitor_indices(&sdl, &winit), vec![1, 0]);
    }

    /// Unequal leftovers are not guessed at.
    #[test]
    fn leftover_count_mismatch_keeps_indices() {
        let sdl = [(0, 0, 2560, 1440), (2560, 0, 1280, 720)];
        let winit = [(0, 0, 3840, 2160)];
        assert_eq!(monitor_indices(&sdl, &winit), vec![0, 1]);
    }
}
