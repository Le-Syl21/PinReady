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
    pub width_mm: i32,
    pub height_mm: i32,
    pub role: DisplayRole,
}

/// Parse inches from SDL3 display name (e.g. "PL4380UH 42\"" -> 42)
fn parse_inches_from_name(name: &str) -> Option<u32> {
    let trimmed = name.trim().trim_end_matches('"');
    trimmed.rsplit(' ').next()?.parse().ok()
}

/// Get physical size using display-info crate, matched by resolution + position.
/// Returns (width_mm, height_mm, diagonal_inches)
fn get_display_physical(x: i32, y: i32, width: i32, height: i32) -> (i32, i32, Option<u32>) {
    if let Ok(displays) = display_info::DisplayInfo::all() {
        for d in &displays {
            if d.x == x
                && d.y == y
                && d.width as i32 == width
                && d.height as i32 == height
                && d.width_mm > 0
                && d.height_mm > 0
            {
                let diag_mm = ((d.width_mm as f64).powi(2) + (d.height_mm as f64).powi(2)).sqrt();
                let inches = (diag_mm / 25.4).round() as u32;
                return (d.width_mm, d.height_mm, Some(inches));
            }
        }
    }
    (0, 0, None)
}

/// Enumerate all connected displays using SDL3.
/// Must be called from the SDL3 thread (or before eframe takes over the main thread).
pub fn enumerate_displays() -> Vec<DisplayInfo> {
    let mut displays = Vec::new();

    unsafe {
        // SDL3 already initialized globally in main

        let mut count: i32 = 0;
        let display_ids = SDL_GetDisplays(&mut count);
        if display_ids.is_null() || count == 0 {
            log::warn!("No displays found");
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

            // Get physical size: display-info crate, fallback to parsing SDL3 name
            let (width_mm, height_mm, inches) =
                get_display_physical(bounds.x, bounds.y, bounds.w, bounds.h);
            let size_inches = inches.or_else(|| parse_inches_from_name(&name));

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
                width_mm,
                height_mm,
                role: DisplayRole::Unused,
            });
        }

        SDL_free(display_ids as *mut _);
        // Do NOT call SDL_QuitSubSystem here — other subsystems (audio, joystick) need SDL alive
    }

    // Sort by total pixel count descending for auto-assignment
    displays.sort_by(|a, b| b.total_pixels.cmp(&a.total_pixels));

    // Auto-assign roles by size
    auto_assign_roles(&mut displays);

    for d in &displays {
        log::info!(
            "Display: {} | {}x{} @ ({},{}) | {:?}",
            d.name,
            d.width,
            d.height,
            d.x,
            d.y,
            d.role
        );
    }

    displays
}

/// Assign roles heuristically based on total pixel count (descending order).
/// Largest → Playfield, 2nd → Backglass, 3rd → DMD, 4th → Topper
fn auto_assign_roles(displays: &mut [DisplayInfo]) {
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
