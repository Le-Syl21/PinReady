use sdl3_sys::everything::*;
use std::ffi::CStr;

/// Display role in a pinball cabinet
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRole {
    Playfield,
    Backglass,
    Dmd,
    Topper,
    Unused,
}

impl DisplayRole {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Playfield => "Playfield",
            Self::Backglass => "Backglass",
            Self::Dmd => "DMD",
            Self::Topper => "Topper",
            Self::Unused => "Non utilisé",
        }
    }

    /// All roles as a slice for UI dropdowns.
    pub fn all() -> &'static [DisplayRole] {
        &[
            Self::Playfield,
            Self::Backglass,
            Self::Dmd,
            Self::Topper,
            Self::Unused,
        ]
    }
}

/// Information about a connected display
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DisplayInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub refresh_rate: f32,
    pub is_primary: bool,
    pub total_pixels: u64,
    pub size_inches: Option<u32>,
    /// Physical size in use: what detection found, unless a human corrected
    /// it — see [`crate::display_reconcile::apply_saved_sizes`].
    pub width_mm: i32,
    pub height_mm: i32,
    /// What detection actually reported, kept beside the value in use so an
    /// override can be told from an agreement.
    ///
    /// Only the difference is worth remembering. Persisting every size would
    /// freeze today's detection for everyone, including the people it gets
    /// right, and would mask any later improvement to it behind a value
    /// nobody chose.
    pub detected_width_mm: i32,
    pub detected_height_mm: i32,
    pub role: DisplayRole,
    /// This screen's index in winit's `available_monitors()`, which is what
    /// `ViewportBuilder::with_monitor` takes. Equal to its index here unless
    /// the two libraries list screens in a different order (Windows, plain
    /// X11, sometimes macOS): see [`crate::monitor_order`].
    pub monitor_index: usize,
    /// SDL's content scale for this screen: what a size in pixels has to be
    /// divided by to become the logical points egui sizes windows in.
    pub content_scale: f32,
    /// The OS handle SDL gives for this screen, where it gives one: the
    /// `HMONITOR` on Windows, which winit reports for the same screen. Lets
    /// the two orders be joined exactly instead of by rectangle.
    pub native_handle: Option<usize>,
}

/// The `HMONITOR` SDL reports for a display, as an address.
#[cfg(target_os = "windows")]
unsafe fn native_handle(id: SDL_DisplayID) -> Option<usize> {
    // SAFETY: plain property lookups on a display id SDL just listed.
    let props = unsafe { SDL_GetDisplayProperties(id) };
    if props == 0 {
        return None;
    }
    let h = unsafe {
        SDL_GetPointerProperty(
            props,
            SDL_PROP_DISPLAY_WINDOWS_HMONITOR_POINTER,
            std::ptr::null_mut(),
        )
    };
    (!h.is_null()).then_some(h as usize)
}

#[cfg(not(target_os = "windows"))]
unsafe fn native_handle(_id: SDL_DisplayID) -> Option<usize> {
    None
}

/// Parse inches from SDL3 display name (e.g. "PL4380UH 42\"" -> 42)
fn parse_inches_from_name(name: &str) -> Option<u32> {
    let trimmed = name.trim().trim_end_matches('"');
    trimmed.rsplit(' ').next()?.parse().ok()
}

/// Physical size for each SDL display, from the panel's own EDID.
///
/// `display-info` used to answer this. On Windows it called
/// `GetDeviceCaps(HORZSIZE/VERTSIZE)`, which returns whatever the display
/// driver invents — a 42-inch panel came back as 1600x900 mm, i.e. 72 inches,
/// and its owner corrected it by hand on every launch. The EDID is the panel's
/// own account of itself and is the same everywhere.
///
/// The mapping is [`crate::display_id::correlate`]'s, which scores an SDL display
/// against an EDID on the fields SDL itself built that name from.
fn physical_sizes(displays: &[DisplayInfo]) -> Vec<(String, (i32, i32), Option<u32>)> {
    let sdl: Vec<crate::display_id::SdlDisplay> = displays
        .iter()
        .map(|d| crate::display_id::SdlDisplay {
            name: d.name.clone(),
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
            width_mm: 0,
            height_mm: 0,
        })
        .collect();
    crate::display_id::correlate(&sdl, &crate::display_id::read_drm_monitors())
        .into_iter()
        .filter_map(|(name, id)| {
            let (w, h) = id.phys_mm?;
            let inches = id.diagonal_inches();
            Some((
                name,
                (i32::try_from(w).unwrap_or(0), i32::try_from(h).unwrap_or(0)),
                inches,
            ))
        })
        .collect()
}

/// Whether the size detection gave for this screen is likely made up.
///
/// TVs put a placeholder in their EDID (1600x900 mm, a 72-inch screen whatever
/// the real one is), and bare panels on generic driver boards carry the
/// board's EDID, named "Monitor" and sized for some other panel. Neither can be
/// fixed from here, but the owner can look the model up and type the size in.
/// A size someone already corrected is trusted.
pub fn size_looks_wrong(d: &DisplayInfo) -> bool {
    if d.width_mm != d.detected_width_mm || d.height_mm != d.detected_height_mm {
        return false;
    }
    let (w, h) = (d.detected_width_mm, d.detected_height_mm);
    // 900 mm on the short side is a 72-inch 16:9 screen: wider than any
    // cabinet, and exactly the 1600x900 placeholder TVs report.
    if w <= 0 || h <= 0 || w.min(h) >= 900 {
        return true;
    }
    let generic = matches!(
        d.name.trim().to_ascii_lowercase().as_str(),
        "" | "monitor" | "generic monitor" | "default monitor" | "unknown"
    );
    // Pixels and millimetres must agree on the shape, whatever the rotation.
    let ratio = |a: f64, b: f64| a.max(b) / a.min(b);
    let px = ratio(d.width as f64, d.height as f64);
    let mm = ratio(w as f64, h as f64);
    generic || (px - mm).abs() / px > 0.1
}

/// Enumerate all connected displays using SDL3. Initializes
/// `SDL_INIT_VIDEO` on first call (no-op refcount bump on subsequent
/// calls); never quits the subsystem manually. `App::shutdown_sdl_threads`
/// runs `SDL_Quit()` before each VPX spawn, which tears VIDEO down
/// alongside AUDIO + JOYSTICK in a single shot.
/// Must be called from the SDL3 thread (or before eframe takes over the main thread).
pub fn enumerate_displays() -> Vec<DisplayInfo> {
    let mut displays = Vec::new();

    unsafe {
        if !SDL_InitSubSystem(SDL_INIT_VIDEO) {
            let err = CStr::from_ptr(SDL_GetError()).to_string_lossy();
            log::warn!("SDL_InitSubSystem(VIDEO) failed: {err}");
            return displays;
        }

        let mut count: i32 = 0;
        let display_ids = SDL_GetDisplays(&mut count);
        if display_ids.is_null() || count <= 0 {
            log::warn!("No displays found (count={count})");
            return displays;
        }

        let primary_id = SDL_GetPrimaryDisplay();

        for i in 0..count as usize {
            let id = *display_ids.add(i);

            // Get display name (includes EDID name + inches on X11)
            let name_ptr = SDL_GetDisplayName(id);
            let name = if !name_ptr.is_null() {
                CStr::from_ptr(name_ptr).to_string_lossy().into_owned()
            } else {
                format!("Display {}", i + 1)
            };

            // Get display bounds (position + size)
            let mut bounds = SDL_Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            };
            if !SDL_GetDisplayBounds(id, &mut bounds) {
                continue;
            }

            // Get refresh rate from desktop mode
            let mode = SDL_GetDesktopDisplayMode(id);
            let refresh_rate = if !mode.is_null() {
                (*mode).refresh_rate
            } else {
                0.0
            };

            let total_pixels = bounds.w as u64 * bounds.h as u64;
            let content_scale = match SDL_GetDisplayContentScale(id) {
                s if s > 0.0 => s,
                _ => 1.0,
            };

            // Physical size is filled in below, once every display is known:
            // the EDID has to be matched against the whole set, not one at a
            // time. The inches in the SDL name are the fallback — SDL derives
            // them from the EDID too, just less precisely.
            let size_inches = parse_inches_from_name(&name);

            displays.push(DisplayInfo {
                name,
                x: bounds.x,
                y: bounds.y,
                width: bounds.w,
                height: bounds.h,
                refresh_rate,
                is_primary: id == primary_id,
                total_pixels,
                size_inches,
                width_mm: 0,
                height_mm: 0,
                detected_width_mm: 0,
                detected_height_mm: 0,
                role: DisplayRole::Unused,
                monitor_index: displays.len(),
                content_scale,
                native_handle: native_handle(id),
            });
        }

        SDL_free(display_ids as *mut _);

        #[cfg(target_os = "linux")]
        {
            let driver_ptr = SDL_GetCurrentVideoDriver();
            let is_x11 = !driver_ptr.is_null() && CStr::from_ptr(driver_ptr).to_bytes() == b"x11";
            if is_x11 && crate::x11_display::is_plain_x11() {
                match_winit_order(&mut displays);
            }
        }
        // Asked while SDL's video subsystem is up: it has already set the
        // process DPI awareness winit will run under, so `rcMonitor` reads
        // the same pixels for both.
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        match_winit_order(&mut displays);
        // No SDL_QuitSubSystem here — the VIDEO subsystem stays init
        // until the next SDL_Quit() (fired by `shutdown_sdl_threads`
        // before each VPX spawn).
    }

    // Physical size from each panel's own EDID, matched against the full set.
    // A display with no readable EDID keeps 0x0 and the inches parsed out of
    // its SDL name, which is what the wizard lets a human correct.
    for (name, (w, h), inches) in physical_sizes(&displays) {
        if let Some(d) = displays.iter_mut().find(|d| d.name == name) {
            d.width_mm = w;
            d.height_mm = h;
            d.detected_width_mm = w;
            d.detected_height_mm = h;
            if let Some(i) = inches {
                d.size_inches = Some(i);
            }
        }
    }

    // Auto-assign roles by pixel count WITHOUT reordering the list: the index
    // in `displays` must match winit's `available_monitors()` order so that
    // `ViewportBuilder::with_monitor(idx)` targets the correct screen.
    let mut sorted_indices: Vec<usize> = (0..displays.len()).collect();
    sorted_indices.sort_by_key(|&i| std::cmp::Reverse(displays[i].total_pixels));
    let roles = [
        DisplayRole::Playfield,
        DisplayRole::Backglass,
        DisplayRole::Dmd,
        DisplayRole::Topper,
    ];
    for (rank, &idx) in sorted_indices.iter().enumerate() {
        displays[idx].role = roles.get(rank).copied().unwrap_or(DisplayRole::Unused);
    }

    for d in &displays {
        log::info!(
            "Display: {} | {}x{} @ ({},{}) | {:?} | winit #{} | scale {}",
            d.name,
            d.width,
            d.height,
            d.x,
            d.y,
            d.role,
            d.monitor_index,
            d.content_scale
        );
    }

    displays
}

/// Share of a screen the wizard's restored-down window may cover, per axis.
/// Monitor sizes are whole screens, not work areas: the height margin leaves
/// room for a taskbar or panel plus the title bar, which is not part of the
/// inner size.
const WIZARD_FIT_WIDTH: f32 = 0.90;
const WIZARD_FIT_HEIGHT: f32 = 0.85;

/// Restored-down size of the wizard window, in logical points.
///
/// The wizard is an ordinary window opened without `with_monitor`, so the OS
/// decides which screen it lands on — on Windows the primary, whatever SDL
/// thinks that is, and SDL's and winit's notion of "which screen" can
/// disagree. Sizing it from one particular display therefore cannot work: the
/// old square at 80 % of the primary's short side came out as a tall window
/// on a 1080p backglass whenever the "primary" was a 4K playfield in portrait,
/// and eframe only clamps an initial size to the *largest* monitor.
///
/// So the window is sized for whichever screen it could end up on: per axis,
/// the smallest logical extent among all displays (pixels divided by that
/// display's own content scale, which is how winit will lay the logical size
/// out there), with a margin. It is still opened maximised; this is the size
/// it restores down to, and the one it gets where maximising is not honoured.
/// `fit_within_monitor` checks it again against the monitor egui reports once
/// the window exists.
pub fn wizard_window_size(displays: &[DisplayInfo]) -> [f32; 2] {
    let logical = |px: i32, scale: f32| {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        px as f32 / scale
    };
    let smallest = displays
        .iter()
        .filter(|d| d.width > 0 && d.height > 0)
        .map(|d| {
            [
                logical(d.width, d.content_scale),
                logical(d.height, d.content_scale),
            ]
        })
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1])]);
    match smallest {
        Some([w, h]) => [
            (w * WIZARD_FIT_WIDTH).round(),
            (h * WIZARD_FIT_HEIGHT).round(),
        ],
        // Nothing enumerated: something that fits a 1280x800 laptop.
        None => [1152.0, 680.0],
    }
}

/// The size a window of `inner` points should shrink to so it fits a monitor
/// of `monitor` points with the same margins as [`wizard_window_size`], or
/// `None` when it already fits. Both sizes must be in the same unit — egui's
/// `ViewportInfo::inner_rect` and `monitor_size` are, and so is the
/// `ViewportCommand::InnerSize` the result goes to.
pub fn fit_within_monitor(inner: [f32; 2], monitor: [f32; 2]) -> Option<[f32; 2]> {
    if !(monitor[0] > 0.0 && monitor[1] > 0.0) {
        return None;
    }
    let max = [
        (monitor[0] * WIZARD_FIT_WIDTH).floor(),
        (monitor[1] * WIZARD_FIT_HEIGHT).floor(),
    ];
    // Half a point of slack: a window already sized by this function reads
    // back with rounding noise and must not be resized again.
    if inner[0] <= max[0] + 0.5 && inner[1] <= max[1] + 0.5 {
        return None;
    }
    Some([inner[0].min(max[0]), inner[1].min(max[1])])
}

// Display-resolvability at startup now lives in `display_reconcile`
// (`unresolvable_assigned_displays`): it is anchor-aware, so an X11↔Wayland
// name divergence no longer forces the wizard — only a genuinely absent
// monitor does. The old raw-name check was removed with that move.

/// Assign roles positionally: first entry → Playfield, etc. Assumes the slice
/// is already sorted by desired priority. Kept for tests; production code in
/// `enumerate_displays` assigns roles without reordering the list.
#[cfg(test)]
pub(crate) fn auto_assign_roles(displays: &mut [DisplayInfo]) {
    let roles = [
        DisplayRole::Playfield,
        DisplayRole::Backglass,
        DisplayRole::Dmd,
        DisplayRole::Topper,
    ];
    for (i, display) in displays.iter_mut().enumerate() {
        display.role = roles.get(i).copied().unwrap_or(DisplayRole::Unused);
    }
}

/// Give each display the index winit will know it by. SDL puts the primary
/// screen first, winit keeps the platform's own order (raw
/// `EnumDisplayMonitors` on Windows, RandR CRTCs on a bare X server): when
/// the two disagree, `with_monitor` opens the launcher on the wrong screen.
/// See [`crate::monitor_order`].
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn match_winit_order(displays: &mut [DisplayInfo]) {
    let rects: Vec<_> = displays
        .iter()
        .map(|d| (d.x, d.y, d.width, d.height))
        .collect();
    let Some(winit) = crate::monitor_order::winit_monitor_rects() else {
        log::warn!("Monitor list unavailable: assuming winit lists screens in SDL's order");
        return;
    };
    // Both orders, so a log from a cabinet shows what each library saw.
    log::info!("SDL monitor order: {rects:?}");
    log::info!("winit monitor order: {winit:?}");
    // Where the OS gives both libraries the same handle (Windows), join on
    // it: exact whatever the scaling. Otherwise, by rectangle.
    let handles: Option<Vec<usize>> = displays.iter().map(|d| d.native_handle).collect();
    let indices = match (handles, crate::monitor_order::winit_monitor_handles()) {
        (Some(sdl), Some(winit_handles)) => {
            log::info!("joining SDL and winit monitors on their HMONITOR");
            crate::monitor_order::handle_indices(&sdl, &winit_handles)
        }
        _ => crate::monitor_order::monitor_indices(&rects, &winit),
    };
    for (d, idx) in displays.iter_mut().zip(indices) {
        if d.monitor_index != idx {
            log::info!(
                "Display {} is winit monitor {idx} (SDL lists it at {})",
                d.name,
                d.monitor_index
            );
        }
        d.monitor_index = idx;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_display(name: &str, w: i32, h: i32) -> DisplayInfo {
        DisplayInfo {
            name: name.to_string(),
            x: 0,
            y: 0,
            width: w,
            height: h,
            refresh_rate: 60.0,
            is_primary: false,
            total_pixels: w as u64 * h as u64,
            size_inches: None,
            width_mm: 0,
            height_mm: 0,
            detected_width_mm: 0,
            detected_height_mm: 0,
            role: DisplayRole::Unused,
            monitor_index: 0,
            content_scale: 1.0,
            native_handle: None,
        }
    }

    // --- Wizard window size ---

    fn scaled(w: i32, h: i32, scale: f32) -> DisplayInfo {
        DisplayInfo {
            content_scale: scale,
            ..make_display("x", w, h)
        }
    }

    #[test]
    fn wizard_fits_the_smallest_screen_of_a_cabinet() {
        // The tester's cabinet: a 4K TV in portrait at 150 %, a 1080p
        // backglass and a 1080p DMD. The window opens on the backglass.
        let displays = [
            scaled(2160, 3840, 1.5),
            scaled(1920, 1080, 1.0),
            scaled(1920, 1080, 1.0),
        ];
        let [w, h] = wizard_window_size(&displays);
        // The portrait TV is the narrowest (1440 pt), the backglass the
        // shortest (1080 pt): the window fits on either.
        assert_eq!([w, h], [1296.0, 918.0]);
        assert!(w <= 1920.0 && h <= 1080.0);
        assert!(w <= 1440.0 && h <= 2560.0);
    }

    #[test]
    fn wizard_size_respects_each_screen_scale() {
        // A 4K panel at 200 % is a 1920x1080 point screen.
        let [w, h] = wizard_window_size(&[scaled(3840, 2160, 2.0)]);
        assert_eq!([w, h], [1728.0, 918.0]);
        // A bogus scale is treated as 1.0, not as a division by zero.
        let [w, h] = wizard_window_size(&[scaled(1920, 1080, 0.0)]);
        assert_eq!([w, h], [1728.0, 918.0]);
    }

    #[test]
    fn wizard_size_without_displays_is_laptop_sized() {
        let [w, h] = wizard_window_size(&[]);
        assert!(w <= 1280.0 && h <= 800.0);
    }

    #[test]
    fn fit_within_monitor_shrinks_only_what_overflows() {
        // A window as tall as a 4K portrait screen, on a 1080p monitor.
        assert_eq!(
            fit_within_monitor([1152.0, 1728.0], [1920.0, 1080.0]),
            Some([1152.0, 918.0])
        );
        // Too wide only.
        assert_eq!(
            fit_within_monitor([2000.0, 600.0], [1920.0, 1080.0]),
            Some([1728.0, 600.0])
        );
        // Fits: left alone, including our own output read back.
        assert_eq!(fit_within_monitor([1000.0, 700.0], [1920.0, 1080.0]), None);
        assert_eq!(fit_within_monitor([1728.0, 918.0], [1920.0, 1080.0]), None);
        // No monitor reported yet.
        assert_eq!(fit_within_monitor([5000.0, 5000.0], [0.0, 0.0]), None);
    }

    // --- DisplayRole ---

    #[test]
    fn display_role_labels() {
        assert_eq!(DisplayRole::Playfield.label(), "Playfield");
        assert_eq!(DisplayRole::Backglass.label(), "Backglass");
        assert_eq!(DisplayRole::Dmd.label(), "DMD");
        assert_eq!(DisplayRole::Topper.label(), "Topper");
        assert_eq!(DisplayRole::Unused.label(), "Non utilisé");
    }

    #[test]
    fn display_role_all_has_5_entries() {
        assert_eq!(DisplayRole::all().len(), 5);
    }

    // --- parse_inches_from_name ---

    #[test]
    fn parse_inches_standard() {
        assert_eq!(parse_inches_from_name("PL4380UH 42\""), Some(42));
    }

    #[test]
    fn parse_inches_with_trailing_quote() {
        assert_eq!(parse_inches_from_name("Samsung U28E590 43\""), Some(43));
    }

    #[test]
    fn parse_inches_no_size() {
        assert_eq!(parse_inches_from_name("Generic Monitor"), None);
    }

    #[test]
    fn parse_inches_empty() {
        assert_eq!(parse_inches_from_name(""), None);
    }

    #[test]
    fn parse_inches_only_number() {
        assert_eq!(parse_inches_from_name("27\""), Some(27));
    }

    // --- auto_assign_roles ---

    #[test]
    fn auto_assign_single_display() {
        let mut displays = vec![make_display("Main", 1920, 1080)];
        auto_assign_roles(&mut displays);
        assert_eq!(displays[0].role, DisplayRole::Playfield);
    }

    #[test]
    fn auto_assign_two_displays() {
        let mut displays = vec![
            make_display("Big", 3840, 2160),
            make_display("Small", 1920, 1080),
        ];
        auto_assign_roles(&mut displays);
        assert_eq!(displays[0].role, DisplayRole::Playfield);
        assert_eq!(displays[1].role, DisplayRole::Backglass);
    }

    #[test]
    fn auto_assign_three_displays() {
        let mut displays = vec![
            make_display("4K", 3840, 2160),
            make_display("QHD", 2560, 1440),
            make_display("FHD", 1920, 1080),
        ];
        auto_assign_roles(&mut displays);
        assert_eq!(displays[0].role, DisplayRole::Playfield);
        assert_eq!(displays[1].role, DisplayRole::Backglass);
        assert_eq!(displays[2].role, DisplayRole::Dmd);
    }

    #[test]
    fn auto_assign_four_displays() {
        let mut displays = vec![
            make_display("4K", 3840, 2160),
            make_display("QHD", 2560, 1440),
            make_display("FHD", 1920, 1080),
            make_display("HD", 1280, 720),
        ];
        auto_assign_roles(&mut displays);
        assert_eq!(displays[0].role, DisplayRole::Playfield);
        assert_eq!(displays[1].role, DisplayRole::Backglass);
        assert_eq!(displays[2].role, DisplayRole::Dmd);
        assert_eq!(displays[3].role, DisplayRole::Topper);
    }

    #[test]
    fn auto_assign_five_displays_extra_is_unused() {
        let mut displays = vec![
            make_display("A", 3840, 2160),
            make_display("B", 2560, 1440),
            make_display("C", 1920, 1080),
            make_display("D", 1280, 720),
            make_display("E", 800, 600),
        ];
        auto_assign_roles(&mut displays);
        assert_eq!(displays[4].role, DisplayRole::Unused);
    }

    #[test]
    fn auto_assign_empty() {
        let mut displays: Vec<DisplayInfo> = vec![];
        auto_assign_roles(&mut displays);
        assert!(displays.is_empty());
    }

    #[test]
    fn tv_placeholder_size_looks_wrong() {
        let mut d = make_display("LG TV SSCR2", 3840, 2160);
        (d.width_mm, d.height_mm) = (1600, 900);
        (d.detected_width_mm, d.detected_height_mm) = (1600, 900);
        assert!(size_looks_wrong(&d));
        // Any screen that tall is out of place in a cabinet, rotated or not.
        (d.width_mm, d.height_mm) = (920, 1640);
        (d.detected_width_mm, d.detected_height_mm) = (920, 1640);
        assert!(size_looks_wrong(&d));
        (d.detected_width_mm, d.detected_height_mm) = (1600, 900);
        // Once the owner typed the real size, it is trusted.
        (d.width_mm, d.height_mm) = (924, 520);
        assert!(!size_looks_wrong(&d));
    }

    #[test]
    fn generic_board_and_bad_shape_look_wrong() {
        let mut d = make_display("Monitor", 1920, 1080);
        (d.width_mm, d.height_mm) = (477, 268);
        (d.detected_width_mm, d.detected_height_mm) = (477, 268);
        assert!(size_looks_wrong(&d));
        d.name = "DELL S2721QS".into();
        assert!(!size_looks_wrong(&d));
        // A 4:3 size on a 16:9 mode, rotated or not.
        (d.width_mm, d.height_mm) = (400, 300);
        (d.detected_width_mm, d.detected_height_mm) = (400, 300);
        assert!(size_looks_wrong(&d));
        let mut r = make_display("DELL S2721QS", 1080, 1920);
        (r.width_mm, r.height_mm) = (597, 336);
        (r.detected_width_mm, r.detected_height_mm) = (597, 336);
        assert!(!size_looks_wrong(&r));
    }

    #[test]
    fn missing_size_looks_wrong() {
        assert!(size_looks_wrong(&make_display("DELL S2721QS", 1920, 1080)));
    }
}
