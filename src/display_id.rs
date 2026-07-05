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
    })
}

/// Decode a descriptor text payload: ASCII terminated by `0x0A`, trailing
/// spaces trimmed. Returns `None` if empty.
fn descriptor_text(payload: &[u8]) -> Option<String> {
    let end = payload.iter().position(|&b| b == 0x0A).unwrap_or(payload.len());
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
        let connector = name.split_once('-').map(|(_, c)| c).unwrap_or(&name).to_string();
        out.push(DrmMonitor { connector, id });
    }
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}

#[cfg(not(target_os = "linux"))]
pub fn read_drm_monitors() -> Vec<DrmMonitor> {
    Vec::new()
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
}
