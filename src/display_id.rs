//! Driver-independent monitor identity from EDID.
//!
//! The X11 ↔ Wayland display-name divergence (and the framerate escape hatch
//! that runs VPX under XWayland while PinReady stays native Wayland) means the
//! *same physical monitor* is named differently depending on which SDL video
//! driver enumerated it. To reconcile a role→display binding across drivers we
//! need an anchor that does not come from the display server at all.
//!
//! That anchor is the **EDID**, read from the kernel DRM layer
//! (`/sys/class/drm/*/edid`). The kernel reads it straight from the monitor's
//! ROM, so it is byte-identical whether the session is Wayland or X11 — and it
//! survives cable/port swaps too (it follows the panel, not the connector).
//!
//! [`MonitorId::fingerprint`] is a SHA-256 of the 128-byte EDID base block: a
//! stable key to persist as `role → fingerprint`. The parsed fields
//! ([`manufacturer`](MonitorId::manufacturer), [`model_name`](MonitorId::model_name),
//! …) are for display and for tie-breaking two identical panels whose EDID
//! carries no unique serial (fingerprints then collide by design — fall back to
//! the DRM connector or the SDL geometry).
//!
//! Foundational layer: the parser + DRM reader land first, the launch-time
//! correlation (SDL-per-driver ↔ EDID ↔ role) is wired on top next.
#![allow(dead_code)]

use sha2::{Digest, Sha256};

/// The immutable identity of a monitor, derived from its EDID base block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorId {
    /// SHA-256 (hex) of the 128-byte EDID base block. Identical across
    /// Wayland/X11 and across cable swaps; the persistent key.
    pub fingerprint: String,
    /// Three-letter PnP manufacturer id (e.g. `"IVM"` for Iiyama).
    pub manufacturer: String,
    /// Product code (vendor-assigned model number).
    pub product: u16,
    /// Serial number. `0` when the panel encodes none (common on no-name
    /// panels) — such units are indistinguishable by EDID alone.
    pub serial: u32,
    /// Year of manufacture (already offset from the EDID's 1990 base).
    pub year: u16,
    /// Week of manufacture (`1..=54`, or `0`/`255` when unspecified).
    pub week: u8,
    /// Model name from the descriptor block, if present (e.g. `"PL4380UH"`).
    pub model_name: Option<String>,
    /// Serial string from the descriptor block, if present.
    pub serial_string: Option<String>,
    /// Physical image size in mm `(w, h)` from the EDID basic parameters, or
    /// `None` when the panel declares it undefined (e.g. many DMD panels).
    pub phys_mm: Option<(u32, u32)>,
    /// Preferred (native) resolution `(w, h)` from the first detailed timing.
    pub preferred_mode: Option<(u32, u32)>,
}

impl MonitorId {
    /// Two ids describe the *same model* (same vendor + product code), even if
    /// they are physically distinct units.
    pub fn same_model(&self, other: &Self) -> bool {
        self.manufacturer == other.manufacturer && self.product == other.product
    }

    /// A short human label, e.g. `"IVM PL4380UH"` or `"XXX #0012"`.
    pub fn label(&self) -> String {
        match &self.model_name {
            Some(name) => format!("{} {name}", self.manufacturer),
            None => format!("{} #{:04x}", self.manufacturer, self.product),
        }
    }
}

const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const BASE_BLOCK: usize = 128;

/// Parse an EDID blob into a [`MonitorId`]. Returns `None` when the base block
/// is too short or the fixed header is absent (not a valid EDID).
pub fn parse_edid(bytes: &[u8]) -> Option<MonitorId> {
    if bytes.len() < BASE_BLOCK || bytes[..8] != EDID_HEADER {
        return None;
    }
    let base = &bytes[..BASE_BLOCK];

    let fingerprint = {
        let mut h = Sha256::new();
        h.update(base);
        let digest = h.finalize();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    };

    // Manufacturer: bytes 8-9, big-endian; three 5-bit letters, A=1.
    let m = ((base[8] as u16) << 8) | base[9] as u16;
    let letter = |shift: u16| -> char { (b'A' - 1 + ((m >> shift) & 0x1f) as u8) as char };
    let manufacturer: String = [letter(10), letter(5), letter(0)].iter().collect();

    let product = u16::from_le_bytes([base[10], base[11]]);
    let serial = u32::from_le_bytes([base[12], base[13], base[14], base[15]]);
    let week = base[16];
    let year = 1990 + base[17] as u16;

    // Basic display params: max image size in cm (0 = undefined / projector).
    let phys_mm = match (base[21], base[22]) {
        (0, _) | (_, 0) => None,
        (w, h) => Some((w as u32 * 10, h as u32 * 10)),
    };

    // First detailed timing descriptor (bytes 54..): active pixels are 12-bit,
    // low byte + high nibble. A 0 pixel clock (bytes 54-55) means it's really a
    // monitor descriptor, not a timing → no preferred mode.
    let preferred_mode = if base[54] == 0 && base[55] == 0 {
        None
    } else {
        let hactive = base[56] as u32 | (((base[58] as u32) & 0xF0) << 4);
        let vactive = base[59] as u32 | (((base[61] as u32) & 0xF0) << 4);
        (hactive > 0 && vactive > 0).then_some((hactive, vactive))
    };

    // Four 18-byte descriptors at 54/72/90/108. A block whose first three bytes
    // are 0 and whose type tag (byte 3) is 0xFC = monitor name, 0xFF = serial.
    let mut model_name = None;
    let mut serial_string = None;
    for offset in [54usize, 72, 90, 108] {
        let d = &base[offset..offset + 18];
        if d[0] == 0 && d[1] == 0 && d[2] == 0 {
            let text = descriptor_text(&d[5..18]);
            match d[3] {
                0xFC => model_name = text,
                0xFF => serial_string = text,
                _ => {}
            }
        }
    }

    Some(MonitorId {
        fingerprint,
        manufacturer,
        product,
        serial,
        year,
        week,
        model_name,
        serial_string,
        phys_mm,
        preferred_mode,
    })
}

/// Decode a descriptor text payload: ASCII terminated by `0x0A`, trailing
/// spaces trimmed. Returns `None` if empty.
fn descriptor_text(payload: &[u8]) -> Option<String> {
    let end = payload
        .iter()
        .position(|&b| b == 0x0A)
        .unwrap_or(payload.len());
    let text: String = payload[..end]
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .trim_end()
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// A monitor discovered through the kernel DRM layer.
#[derive(Clone, Debug)]
pub struct DrmMonitor {
    /// Connector name without the card prefix, e.g. `"DP-1"`, `"HDMI-A-1"`.
    /// Driver-independent; changes only on a physical port change.
    pub connector: String,
    pub id: MonitorId,
}

/// Enumerate connected monitors and their [`MonitorId`] from
/// `/sys/class/drm/*/edid`. Linux only; the EDID here is the same regardless of
/// the active session type (Wayland or X11), which is the whole point.
///
/// Connectors with no readable EDID (disconnected, or not exposed by the
/// driver) are skipped. Note `stat()` reports size 0 for these sysfs binary
/// attributes, so we read them unconditionally rather than trusting the size.
#[cfg(target_os = "linux")]
pub fn read_drm_monitors() -> Vec<DrmMonitor> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let edid_path = dir.join("edid");
        let Ok(bytes) = std::fs::read(&edid_path) else {
            continue;
        };
        let Some(id) = parse_edid(&bytes) else {
            continue;
        };
        // Dir name is "cardN-<connector>"; strip the card prefix.
        let name = entry.file_name().to_string_lossy().into_owned();
        let connector = name
            .split_once('-')
            .map(|(_, c)| c)
            .unwrap_or(&name)
            .to_string();
        out.push(DrmMonitor { connector, id });
    }
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}

#[cfg(not(target_os = "linux"))]
pub fn read_drm_monitors() -> Vec<DrmMonitor> {
    Vec::new()
}

/// A display as SDL reports it under some video driver — the identity that VPX
/// writes to `*Display=`. The `name` differs between wayland and x11 for the
/// same monitor; the geometry (position + size) does not.
#[derive(Clone, Debug)]
pub struct SdlDisplay {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub width_mm: i32,
    pub height_mm: i32,
}

/// What the wizard persists for a role, to re-resolve the SDL display name at
/// every launch regardless of the video driver.
///
/// Primary key is the **layout position** (+ size): identical across
/// Wayland/X11 for the same physical setup, so it bridges the driver switch
/// perfectly. The optional EDID **fingerprint** is the fallback that also
/// survives a physical re-arrangement (position changes, EDID does not) — but
/// only for monitors whose EDID is usable (some cheap panels declare no serial,
/// no physical size and a native mode unrelated to how they're driven, and can
/// only be tracked by position).
#[derive(Clone, Debug)]
pub struct DisplayAnchor {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub fingerprint: Option<String>,
}

/// Re-resolve a role's [`DisplayAnchor`] to the SDL display name under the
/// current driver: try the exact layout position+size first (handles the
/// driver switch), then the EDID fingerprint via `correlation` (handles a
/// re-arrangement). `None` → the monitor is gone, caller falls back to the
/// wizard.
pub fn resolve_anchor(
    anchor: &DisplayAnchor,
    sdl: &[SdlDisplay],
    correlation: &[(String, MonitorId)],
) -> Option<String> {
    if let Some(d) = sdl.iter().find(|d| {
        d.x == anchor.x && d.y == anchor.y && d.width == anchor.width && d.height == anchor.height
    }) {
        return Some(d.name.clone());
    }
    anchor
        .fingerprint
        .as_deref()
        .and_then(|fp| resolve_display_name(fp, correlation))
}

/// Correlate the SDL displays of *one* driver to the kernel DRM monitors,
/// yielding `(sdl_name → MonitorId)`. This is the bridge: SDL gives the
/// driver-specific name + physical characteristics, DRM gives the stable EDID
/// identity, and matching them by resolution + physical size links the two.
///
/// Matching is greedy on a score (native resolution match dominates, physical
/// size breaks ties) with each side used at most once — enough for real
/// cabinets where monitors differ in model or size. Truly identical twin
/// panels (same resolution *and* size *and* EDID) are indistinguishable here
/// and would need connector/position data to separate.
pub fn correlate(sdl: &[SdlDisplay], drm: &[DrmMonitor]) -> Vec<(String, MonitorId)> {
    fn score(s: &SdlDisplay, id: &MonitorId) -> i32 {
        let mut pts = 0;
        if let Some((w, h)) = id.preferred_mode {
            if w as i32 == s.width && h as i32 == s.height {
                pts += 100;
            }
        }
        if let (Some((w, h)), true) = (id.phys_mm, s.width_mm > 0 && s.height_mm > 0) {
            // ±5 mm tolerance: EDID stores cm, SDL/EDID rounding differs.
            if (w as i32 - s.width_mm).abs() <= 5 && (h as i32 - s.height_mm).abs() <= 5 {
                pts += 10;
            }
        }
        pts
    }

    // All (sdl, drm) pairs with a positive score, best first, greedily assigned.
    let mut pairs: Vec<(i32, usize, usize)> = Vec::new();
    for (si, s) in sdl.iter().enumerate() {
        for (di, d) in drm.iter().enumerate() {
            let pts = score(s, &d.id);
            if pts > 0 {
                pairs.push((pts, si, di));
            }
        }
    }
    pairs.sort_by_key(|&(score, ..)| std::cmp::Reverse(score));

    let mut used_sdl = vec![false; sdl.len()];
    let mut used_drm = vec![false; drm.len()];
    let mut out = Vec::new();
    for (_, si, di) in pairs {
        if used_sdl[si] || used_drm[di] {
            continue;
        }
        used_sdl[si] = true;
        used_drm[di] = true;
        out.push((sdl[si].name.clone(), drm[di].id.clone()));
    }
    out
}

/// Given a role's stored EDID fingerprint and the current (driver-specific)
/// SDL↔EDID correlation, return the SDL display name to write to `*Display=`.
/// `None` when no current monitor carries that fingerprint (the panel is gone
/// → caller falls back to the wizard).
pub fn resolve_display_name(
    fingerprint: &str,
    correlation: &[(String, MonitorId)],
) -> Option<String> {
    correlation
        .iter()
        .find(|(_, id)| id.fingerprint == fingerprint)
        .map(|(name, _)| name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real EDIDs pulled from the pincab (`/sys/class/drm/card1-*/edid`).
    const DP_1: &str = "00ffffffffffff0026cd12003432313012210104b55e35783ba4a5ae4f47ab26105054b7cf00814081809500714f81c0b300d1c001014dd000a0f0703e8030203500ad113200001a6fc200a0a0a0555030203500ad113200001a000000fd003078fefe6e010a202020202020000000fc00504c3433383055480a2020202002b802033bf14f010203040e0f901112131d1e1f6061230907078301000065030c0010006d1a000002013078000000000000e305c000e60605017373208a6f80a0703840403020280cad113200001a6fc200a0a0a0555030203500ad113200001a000000000000000000000000000000000000000000000000000000000000000050";
    const DVI_D_1: &str = "00ffffffffffff006318000000000000061e0103800000780ad7a5a2594a9624145054a3080081c00101010101010101010101010101121b007b50201530302036003f4321000018023a801871382d40582c45003f432100001a000000fd001e4c1e5a1e000a202020202020000000fc004141410a2020202020202020200175020324715090050403070206011f141312161115202309070366030c0010000083010000011d007251d01e206e285500c48e2100001e011d8018711c1620582c2500c48e2100009e8c0ad08a20e02d10103e9600138e2100001800000000000000000000000000000000000000000000000000000000000000000000000000ee";
    const HDMI_A_1: &str = "00ffffffffffff0026cd2d76d100000002210103804627782a7e59a45554a121094e52254b00e100a940b3009500d100d1c0a9c001016a5e00a0a0a0295030203500bc862100001e000000ff0031323038353330323030323039000000fd00304b1b7821000a202020202020000000fc00504c33323934510a202020202001f9020329f14990010203111213041f230907078301000067030c001000b844e305c301e60605014b4b002a4480a070382740582c4500bc862100001e011d007251d01e206e285500bc862100001e662156aa51001e3030203500bc862100001ed97600a0a0a0345030203500bc862100001e00000000000000000000000000000e";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn parses_playfield_iiyama_42() {
        let id = parse_edid(&hex(DP_1)).expect("valid EDID");
        assert_eq!(id.manufacturer, "IVM"); // Iiyama
        assert_eq!(id.model_name.as_deref(), Some("PL4380UH"));
        assert_ne!(id.serial, 0);
        assert_eq!(id.year, 2023);
    }

    #[test]
    fn parses_dmd_noname_panel() {
        let id = parse_edid(&hex(DVI_D_1)).expect("valid EDID");
        assert_eq!(id.manufacturer, "XXX"); // no-name panel
        assert_eq!(id.model_name.as_deref(), Some("AAA"));
        assert_eq!(id.serial, 0, "this panel encodes no serial");
    }

    #[test]
    fn parses_backglass_iiyama_32() {
        let id = parse_edid(&hex(HDMI_A_1)).expect("valid EDID");
        assert_eq!(id.manufacturer, "IVM");
        assert_eq!(id.model_name.as_deref(), Some("PL3294Q"));
        assert_eq!(id.serial_string.as_deref(), Some("1208530200209"));
    }

    #[test]
    fn fingerprints_are_distinct_across_the_three_panels() {
        let a = parse_edid(&hex(DP_1)).unwrap().fingerprint;
        let b = parse_edid(&hex(DVI_D_1)).unwrap().fingerprint;
        let c = parse_edid(&hex(HDMI_A_1)).unwrap().fingerprint;
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64, "sha-256 hex is 64 chars");
    }

    #[test]
    fn fingerprint_is_stable_across_reparse() {
        let a = parse_edid(&hex(HDMI_A_1)).unwrap().fingerprint;
        let b = parse_edid(&hex(HDMI_A_1)).unwrap().fingerprint;
        assert_eq!(a, b);
    }

    #[test]
    fn same_model_detects_the_two_iiyamas_as_different_models() {
        // Both Iiyama (IVM) but different product codes → not the same model.
        let pf = parse_edid(&hex(DP_1)).unwrap();
        let bg = parse_edid(&hex(HDMI_A_1)).unwrap();
        assert_eq!(pf.manufacturer, bg.manufacturer);
        assert!(!pf.same_model(&bg));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_edid(&[0u8; 128]).is_none()); // bad header
        assert!(parse_edid(&[0xff; 8]).is_none()); // too short
    }

    #[test]
    fn extracts_physical_size_and_native_mode() {
        let pf = parse_edid(&hex(DP_1)).unwrap();
        assert_eq!(pf.phys_mm, Some((940, 530)));
        assert_eq!(pf.preferred_mode, Some((3840, 2160)));
        // The no-name DMD panel is a cautionary tale: no physical size, and its
        // EDID-native mode (1280x800) is unrelated to how it's actually driven
        // (1920x1080) — hence it can only be tracked by layout position.
        let dmd = parse_edid(&hex(DVI_D_1)).unwrap();
        assert_eq!(dmd.phys_mm, None);
        assert_eq!(dmd.preferred_mode, Some((1280, 800)));
    }

    /// The three DRM monitors, as `read_drm_monitors` would return them.
    fn cab_drm() -> Vec<DrmMonitor> {
        vec![
            DrmMonitor {
                connector: "DP-1".into(),
                id: parse_edid(&hex(DP_1)).unwrap(),
            },
            DrmMonitor {
                connector: "DVI-D-1".into(),
                id: parse_edid(&hex(DVI_D_1)).unwrap(),
            },
            DrmMonitor {
                connector: "HDMI-A-1".into(),
                id: parse_edid(&hex(HDMI_A_1)).unwrap(),
            },
        ]
    }

    // Real `--enumerate-displays` output captured from the cab (both drivers).
    fn sdl_wayland() -> Vec<SdlDisplay> {
        vec![
            SdlDisplay {
                name: "Iiyama North America 42\"".into(),
                x: 0,
                y: 0,
                width: 3840,
                height: 2160,
                width_mm: 940,
                height_mm: 530,
            },
            SdlDisplay {
                name: "XXX".into(),
                x: 3840,
                y: 0,
                width: 1920,
                height: 1080,
                width_mm: 0,
                height_mm: 0,
            },
            SdlDisplay {
                name: "Iiyama North America 32\"".into(),
                x: 5760,
                y: 0,
                width: 2560,
                height: 1440,
                width_mm: 700,
                height_mm: 390,
            },
        ]
    }
    fn sdl_x11() -> Vec<SdlDisplay> {
        vec![
            SdlDisplay {
                name: "DP-1 42\"".into(),
                x: 0,
                y: 0,
                width: 3840,
                height: 2160,
                width_mm: 940,
                height_mm: 530,
            },
            SdlDisplay {
                name: "DVI-D-1".into(),
                x: 3840,
                y: 0,
                width: 1920,
                height: 1080,
                width_mm: 508,
                height_mm: 286,
            },
            SdlDisplay {
                name: "HDMI-1 32\"".into(),
                x: 5760,
                y: 0,
                width: 2560,
                height: 1440,
                width_mm: 700,
                height_mm: 390,
            },
        ]
    }

    #[test]
    fn correlate_links_good_edid_monitors() {
        // The two Iiyamas (usable EDID: real physical size) correlate; the
        // no-name DMD (no mm, native mode ≠ driven mode) does not — by design,
        // it is tracked by position instead.
        let drm = cab_drm();
        let pf = parse_edid(&hex(DP_1)).unwrap().fingerprint;
        let bg = parse_edid(&hex(HDMI_A_1)).unwrap().fingerprint;

        for sdl in [sdl_wayland(), sdl_x11()] {
            let c = correlate(&sdl, &drm);
            assert!(resolve_display_name(&pf, &c).unwrap().contains("42"));
            assert!(resolve_display_name(&bg, &c).unwrap().contains("32"));
        }
    }

    /// A role anchored under Wayland resolves to the *x11* name after a driver
    /// switch — the whole point. Position bridges all three (incl. the DMD);
    /// the fingerprint is the rearrangement fallback.
    #[test]
    fn anchor_bridges_wayland_to_x11_by_position() {
        // Playfield anchored while enumerated under Wayland.
        let pf_id = parse_edid(&hex(DP_1)).unwrap();
        let anchor = DisplayAnchor {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
            fingerprint: Some(pf_id.fingerprint.clone()),
        };
        // Now the session runs x11: enumerate + correlate under x11.
        let x_corr = correlate(&sdl_x11(), &cab_drm());
        assert_eq!(
            resolve_anchor(&anchor, &sdl_x11(), &x_corr).as_deref(),
            Some("DP-1 42\"")
        );

        // The DMD, whose EDID can't correlate, still bridges by position alone.
        let dmd_anchor = DisplayAnchor {
            x: 3840,
            y: 0,
            width: 1920,
            height: 1080,
            fingerprint: None,
        };
        assert_eq!(
            resolve_anchor(&dmd_anchor, &sdl_x11(), &x_corr).as_deref(),
            Some("DVI-D-1")
        );
    }

    /// After a physical re-arrangement (positions change), the good-EDID
    /// monitors still resolve via their fingerprint.
    #[test]
    fn anchor_survives_rearrangement_via_fingerprint() {
        let bg_id = parse_edid(&hex(HDMI_A_1)).unwrap();
        // Backglass was at (5760,0) when anchored; user moved it to (0,0).
        let anchor = DisplayAnchor {
            x: 5760,
            y: 0,
            width: 2560,
            height: 1440,
            fingerprint: Some(bg_id.fingerprint.clone()),
        };
        let mut moved = sdl_x11();
        moved.iter_mut().find(|d| d.name.contains("32")).unwrap().x = 0;
        let corr = correlate(&moved, &cab_drm());
        // Position no longer matches → fingerprint fallback finds it anyway.
        assert_eq!(
            resolve_anchor(&anchor, &moved, &corr).as_deref(),
            Some("HDMI-1 32\"")
        );
    }
}
