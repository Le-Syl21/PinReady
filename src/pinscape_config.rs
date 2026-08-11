//! Read a Pinscape KL25Z's own configuration over HID.
//!
//! The firmware answers `65 9 <variable>` with a "configuration variable
//! report" — a header of `0x9800`, the variable ID, then its value. That is
//! how its Config Tool works, and it means the board can be *asked* rather
//! than the user interrogated: the accelerometer's range and orientation, and
//! whether a plunger sensor exists at all, are facts the firmware already
//! knows.
//!
//! Protocol reference: mjrgh/Pinscape_Controller, `USBProtocol.h` (MIT).
//! Sections 2D (configuration variable report) and CONFIGURATION VARIABLES.

use hidapi::{HidApi, HidDevice};

/// Private VID/PID of a Pinscape board running in native mode. The LedWiz
/// emulation mode uses another pair and does not speak this protocol.
const PINSCAPE_VID: u16 = 0x1209;
const PINSCAPE_PID: u16 = 0xEAEA;

const CMD_EXTENDED: u8 = 65;
const CMD_QUERY_CONFIG_VAR: u8 = 9;

const VAR_ACCELEROMETER: u8 = 4;
const VAR_PLUNGER_TYPE: u8 = 5;

/// Reply header for a configuration variable report (little-endian 0x9800).
const REPLY_CONFIG_VAR: [u8; 2] = [0x00, 0x98];

/// How long to wait for the board to answer. It interleaves replies with its
/// ordinary joystick reports, so a few of those arrive first; 50 ms is far
/// more than the 1 ms USB polling period needs.
const REPLY_TIMEOUT_MS: i32 = 50;
/// Reports to sift through before giving up on one query.
const MAX_REPORTS: usize = 40;

/// What the board says about itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinscapeConfig {
    /// Accelerometer full-scale range in g: 1, 2, 4 or 8.
    pub accel_range_g: f32,
    /// Board orientation in the cabinet, which decides how its X/Y map to
    /// side/front: 0 = USB ports at front, 1 = left, 2 = right, 3 = rear.
    pub accel_orientation: u8,
    /// Auto-centring: 0 = on with a 5 s timer, 1..60 = on with that delay in
    /// seconds, 255 = off (manual centring only).
    pub accel_autocenter: u8,
    /// Plunger sensor type, 0 when no plunger is wired at all.
    pub plunger_type: u8,
}

impl PinscapeConfig {
    /// Human-readable orientation, for the UI.
    pub fn orientation_label(&self) -> &'static str {
        match self.accel_orientation {
            0 => "USB ports at front",
            1 => "USB ports at left",
            2 => "USB ports at right",
            3 => "USB ports at rear",
            _ => "unknown",
        }
    }

    pub fn has_plunger(&self) -> bool {
        self.plunger_type != 0
    }
}

/// Ask the first Pinscape board on the bus for its configuration.
///
/// `None` when no board is connected, when it runs in LedWiz emulation mode
/// (different VID/PID, no config protocol), or when it stays silent — all of
/// which simply mean "we cannot know", never an error worth surfacing.
pub fn read() -> Option<PinscapeConfig> {
    let api = HidApi::new()
        .inspect_err(|e| log::debug!("Pinscape config: HidApi init failed: {e}"))
        .ok()?;
    let device = api
        .open(PINSCAPE_VID, PINSCAPE_PID)
        .inspect_err(|e| log::debug!("Pinscape config: no board to query ({e})"))
        .ok()?;

    let accel = query(&device, VAR_ACCELEROMETER, 0)?;
    // byte 3 = orientation, byte 4 = dynamic range, byte 5 = auto-centring.
    let accel_range_g = match accel[4] {
        1 => 2.0,
        2 => 4.0,
        3 => 8.0,
        // 0 is "±1G (2G hardware mode, rescaled to a 1G range)"; anything
        // else is a firmware newer than this code, and 1 g is both the
        // firmware default and the value mjr recommends for a cabinet.
        _ => 1.0,
    };

    // A board with no plunger wired answers here too — type 0.
    let plunger_type = query(&device, VAR_PLUNGER_TYPE, 0).map_or(0, |v| v[3]);

    let config = PinscapeConfig {
        accel_range_g,
        accel_orientation: accel[3],
        accel_autocenter: accel[5],
        plunger_type,
    };
    log::info!(
        "Pinscape board: accelerometer ±{} g, {}, plunger type {}",
        config.accel_range_g,
        config.orientation_label(),
        config.plunger_type
    );
    Some(config)
}

/// Query one configuration variable, returning its 8-byte report.
fn query(device: &HidDevice, variable: u8, index: u8) -> Option<[u8; 8]> {
    // Leading 0x00 is the report ID hidapi expects; the board's own messages
    // are 8 bytes.
    let request = [
        0x00,
        CMD_EXTENDED,
        CMD_QUERY_CONFIG_VAR,
        variable,
        index,
        0,
        0,
        0,
        0,
    ];
    device
        .write(&request)
        .inspect_err(|e| log::debug!("Pinscape config: query {variable} failed: {e}"))
        .ok()?;

    // Replies share the endpoint with ordinary joystick reports, so read
    // until one carries our header and variable ID.
    let mut buffer = [0u8; 64];
    for _ in 0..MAX_REPORTS {
        let read = device.read_timeout(&mut buffer, REPLY_TIMEOUT_MS).ok()?;
        if read < 8 {
            continue;
        }
        if buffer[0..2] == REPLY_CONFIG_VAR && buffer[2] == variable {
            let mut value = [0u8; 8];
            value.copy_from_slice(&buffer[0..8]);
            return Some(value);
        }
    }
    log::debug!("Pinscape config: no reply for variable {variable}");
    None
}

/// Read the board in the background: HID open + query can take tens of
/// milliseconds, and startup should not wait on a peripheral.
pub fn spawn_read() -> crossbeam_channel::Receiver<Option<PinscapeConfig>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("pinready-pinscape-cfg".into())
        .spawn(move || {
            let _ = tx.send(read());
        })
        .expect("spawn pinscape config thread");
    rx
}
