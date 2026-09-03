//! Where a display's EDID comes from, on each platform.
//!
//! EDID is the display's own account of itself: who made it, which model, what
//! serial, and how big the panel physically is. Every other source we tried
//! was a paraphrase of it at best. `display-info`'s Windows backend called
//! `GetDeviceCaps(HORZSIZE/VERTSIZE)`, which returns whatever the display
//! driver feels like declaring — a 42-inch panel came back as 1600×900 mm,
//! i.e. 72 inches, and the owner had to correct it by hand on every launch.
//!
//! So this module does the one thing that was missing: hand over the raw
//! bytes. Parsing them is [`piaf`]'s job, and matching them to a display SDL
//! knows about is the caller's.
//!
//! The matching key differs per platform, and that is deliberate — each one
//! has an exact answer, so none of this needs the resolution-and-size scoring
//! heuristic it replaces:
//!
//! * **Linux** — the DRM connector name (`DP-2`), which is also the name SDL
//!   reports under both X11 and Wayland.
//! * **Windows** — the `HMONITOR`, which SDL hands over directly in
//!   `SDL_PROP_DISPLAY_WINDOWS_HMONITOR_POINTER`.
//! * **macOS** — the `CGDirectDisplayID`, matched to SDL by screen bounds
//!   since SDL exposes no macOS display property.

/// One display's EDID, plus what identifies it on this platform.
#[derive(Debug, Clone)]
pub struct DisplayEdid {
    pub key: MatchKey,
    pub bytes: Vec<u8>,
}

/// How to tie an EDID back to the display SDL enumerated.
// Two variants have no producer yet: the Windows and macOS sources are the
// next step, and the enum is what they will fill.
#[allow(dead_code, reason = "Windows and macOS sources land next")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchKey {
    /// DRM connector name, e.g. `"DP-2"`.
    Connector(String),
    /// Win32 `HMONITOR`, as an address.
    Monitor(usize),
    /// `CGDirectDisplayID`.
    CoreGraphics(u32),
}

/// Every EDID this machine will give us.
///
/// A display whose EDID cannot be read is simply absent from the result: an
/// unreadable EDID is not an error worth failing a launch over, it just means
/// the caller falls back to whatever it did before.
#[must_use]
pub fn read_all() -> Vec<DisplayEdid> {
    platform::read_all()
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{DisplayEdid, MatchKey};

    /// `/sys/class/drm/<card>-<connector>/edid`.
    ///
    /// `stat()` reports size 0 for these sysfs binary attributes, so they are
    /// read unconditionally rather than filtered on length first.
    pub fn read_all() -> Vec<DisplayEdid> {
        let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let Ok(bytes) = std::fs::read(entry.path().join("edid")) else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            // Directory is `cardN-<connector>`; SDL reports `<connector>`.
            let dir = entry.file_name().to_string_lossy().into_owned();
            let connector = dir
                .split_once('-')
                .map_or(dir.clone(), |(_, c)| c.to_owned());
            out.push(DisplayEdid {
                key: MatchKey::Connector(connector),
                bytes,
            });
        }
        out.sort_by(|a, b| format!("{:?}", a.key).cmp(&format!("{:?}", b.key)));
        out
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::DisplayEdid;

    pub fn read_all() -> Vec<DisplayEdid> {
        Vec::new()
    }
}
