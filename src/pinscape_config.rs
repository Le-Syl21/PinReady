//! Ask a Pinscape board about its own configuration, over HID.
//!
//! Facts like the accelerometer's range and orientation, or whether a plunger
//! is wired at all, are things the firmware already knows — so they should be
//! read, not asked of the user.
//!
//! Both generations answer on VID/PID 1209:EAEA and are told apart by their
//! USB product string, but they share nothing else:
//!
//! - **KL25Z** — numbered configuration variables. `65 9 <var>` returns a
//!   "configuration variable report": header `0x9800`, the variable ID, then
//!   its value. Reference: mjrgh/Pinscape_Controller, `USBProtocol.h` (MIT),
//!   section 2D and CONFIGURATION VARIABLES.
//! - **Pico** — its settings live in a JSON file reachable only through a
//!   libusb vendor interface, which would cost a new system dependency. Its
//!   HID "feedback controller" is free, though, and answers a status query:
//!   whether a plunger is enabled and calibrated. Reference:
//!   mjrgh/PinscapePico, `LinuxAPI/FeedbackControllerInterface.cpp`.

use hidapi::{HidApi, HidDevice};

/// Private VID/PID of a Pinscape board running in native mode. The LedWiz
/// emulation mode uses another pair and does not speak this protocol.
const PINSCAPE_VID: u16 = 0x1209;
const PINSCAPE_PID: u16 = 0xEAEA;

/// Boards we can recognise on the bus but not question.
const OPAQUE_BOARDS: &[(u16, u16, &str)] = &[(0x2E8A, 0x106F, "DudesCab")];

const CMD_EXTENDED: u8 = 65;
const CMD_QUERY_CONFIG_VAR: u8 = 9;

const VAR_ACCELEROMETER: u8 = 4;
const VAR_PLUNGER_TYPE: u8 = 5;

/// Pico feedback-controller HID: request report ID, and the status query it
/// carries; the reply comes back under its own report ID.
const PICO_REPORT_ID_REQUEST: u8 = 0x04;
const PICO_REQ_QUERY_ID: u8 = 0x01;
const PICO_REQ_QUERY_STATUS: u8 = 0x02;
const PICO_REPORT_ID_ID: u8 = 0x01;
const PICO_REPORT_ID_STATUS: u8 = 0x02;

/// Reply header for a configuration variable report (little-endian 0x9800).
const REPLY_CONFIG_VAR: [u8; 2] = [0x00, 0x98];

/// How long to wait for the board to answer. It interleaves replies with its
/// ordinary joystick reports, so a few of those arrive first; 50 ms is far
/// more than the 1 ms USB polling period needs.
const REPLY_TIMEOUT_MS: i32 = 50;
/// Reports to sift through before giving up on one query.
const MAX_REPORTS: usize = 40;

/// Which board answered. They share VID/PID 1209:EAEA and are told apart by
/// their USB product string, but speak entirely different protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    /// Pinscape Controller on a FRDM-KL25Z — numbered configuration
    /// variables over HID.
    Kl25z,
    /// Pinscape Pico — a JSON configuration file behind a libusb vendor
    /// interface, plus a small HID "feedback controller" that answers a
    /// status query.
    Pico,
    /// A board we recognise but cannot interrogate: DudesCab and PinOne have
    /// closed firmware with no documented query protocol (libdof drives their
    /// *outputs* only, and PinOne talks over a serial port at that). Worth
    /// naming anyway — "your DudesCab does not publish this" is honest, where
    /// "no board detected" would be a lie.
    Opaque(&'static str),
}

/// What the board says about itself. Fields are optional because the two
/// boards do not expose the same facts over the channel we can reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinscapeConfig {
    pub board: Board,
    /// Accelerometer full-scale range in g: 1, 2, 4 or 8. KL25Z only — the
    /// Pico keeps this in its config file, behind libusb.
    pub accel_range_g: Option<f32>,
    /// Board orientation in the cabinet, which decides how its X/Y map to
    /// side/front: 0 = USB ports at front, 1 = left, 2 = right, 3 = rear.
    pub accel_orientation: Option<u8>,
    /// Auto-centring: 0 = on with a 5 s timer, 1..60 = on with that delay in
    /// seconds, 255 = off (manual centring only).
    pub accel_autocenter: Option<u8>,
    /// Plunger sensor type, 0 when no plunger is wired at all (KL25Z).
    pub plunger_type: Option<u8>,
    /// Whether a plunger is enabled, as the Pico reports it.
    pub plunger_enabled: Option<bool>,
    /// Whether the plunger is calibrated (Pico).
    pub plunger_calibrated: Option<bool>,
}

impl PinscapeConfig {
    /// Human-readable orientation, for the UI.
    pub fn orientation_label(&self) -> &'static str {
        match self.accel_orientation {
            Some(0) => "USB ports at front",
            Some(1) => "USB ports at left",
            Some(2) => "USB ports at right",
            Some(3) => "USB ports at rear",
            _ => "unknown orientation",
        }
    }

    /// Whether a plunger exists. `None` when the board did not say — the
    /// caller must then keep whatever it was doing before, not assume.
    pub fn has_plunger(&self) -> Option<bool> {
        match (self.plunger_type, self.plunger_enabled) {
            (Some(t), _) => Some(t != 0),
            (None, Some(enabled)) => Some(enabled),
            _ => None,
        }
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

    // Same VID/PID for both generations, so the product string decides which
    // protocol to speak. Sending KL25Z queries to a Pico would just time out,
    // slowly.
    let board = api
        .device_list()
        .find(|d| d.vendor_id() == PINSCAPE_VID && d.product_id() == PINSCAPE_PID)
        .map(|d| {
            let product = d.product_string().unwrap_or_default().to_lowercase();
            if product.contains("pico") {
                Board::Pico
            } else {
                Board::Kl25z
            }
        })
        .or_else(|| {
            api.device_list().find_map(|d| {
                OPAQUE_BOARDS
                    .iter()
                    .find(|(vid, pid, _)| d.vendor_id() == *vid && d.product_id() == *pid)
                    .map(|(_, _, name)| Board::Opaque(name))
            })
        })?;

    if let Board::Opaque(name) = board {
        log::info!("{name} detected — its firmware publishes no input settings");
        return Some(PinscapeConfig {
            board,
            accel_range_g: None,
            accel_orientation: None,
            accel_autocenter: None,
            plunger_type: None,
            plunger_enabled: None,
            plunger_calibrated: None,
        });
    }

    let device = api
        .open(PINSCAPE_VID, PINSCAPE_PID)
        .inspect_err(|e| log::debug!("Pinscape config: no board to query ({e})"))
        .ok()?;

    match board {
        Board::Pico => read_pico(&device),
        Board::Kl25z => read_kl25z(&device),
        // Returned above, before the device is even opened.
        Board::Opaque(_) => None,
    }
}

/// KL25Z: numbered configuration variables.
fn read_kl25z(device: &HidDevice) -> Option<PinscapeConfig> {
    let accel = query(device, VAR_ACCELEROMETER, 0)?;
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
    let plunger_type = query(device, VAR_PLUNGER_TYPE, 0).map(|v| v[3]);

    let config = PinscapeConfig {
        board: Board::Kl25z,
        accel_range_g: Some(accel_range_g),
        accel_orientation: Some(accel[3]),
        accel_autocenter: Some(accel[5]),
        plunger_type,
        plunger_enabled: None,
        plunger_calibrated: None,
    };
    log::info!(
        "Pinscape KL25Z: accelerometer ±{accel_range_g} g, {}, plunger type {}",
        config.orientation_label(),
        plunger_type.map_or("unknown".to_string(), |t| t.to_string())
    );
    Some(config)
}

/// Pico: the feedback-controller HID interface answers a status query. Its
/// accelerometer settings live in the config file, behind libusb, so they
/// stay unknown here — deliberately, rather than at the price of a new
/// system dependency and a udev rule.
fn read_pico(device: &HidDevice) -> Option<PinscapeConfig> {
    let mut request = [0u8; 64];
    request[0] = PICO_REPORT_ID_REQUEST;
    request[1] = PICO_REQ_QUERY_STATUS;
    device
        .write(&request)
        .inspect_err(|e| log::debug!("Pinscape Pico: status query failed: {e}"))
        .ok()?;

    let mut buffer = [0u8; 64];
    for _ in 0..MAX_REPORTS {
        let read = device.read_timeout(&mut buffer, REPLY_TIMEOUT_MS).ok()?;
        if read > 1 && buffer[0] == PICO_REPORT_ID_STATUS {
            let flags = buffer[1];
            // The identification report carries the plunger *type*, which the
            // status flags only summarise as enabled/disabled. Its layout:
            // <0x01> <UnitNumber:BYTE> <UnitName:CHAR[32]> <ProtocolVer:UINT16>
            // <HardwareID:BYTE[8]> <NumPorts:UINT16> <PlungerType:UINT16> …
            let plunger_type = query_pico_id(device);
            let config = PinscapeConfig {
                board: Board::Pico,
                accel_range_g: None,
                accel_orientation: None,
                accel_autocenter: None,
                plunger_type,
                plunger_enabled: Some(flags & 0x01 != 0),
                plunger_calibrated: Some(flags & 0x02 != 0),
            };
            log::info!(
                "Pinscape Pico: plunger {} ({})",
                if config.plunger_enabled == Some(true) {
                    "enabled"
                } else {
                    "disabled"
                },
                if config.plunger_calibrated == Some(true) {
                    "calibrated"
                } else {
                    "not calibrated"
                }
            );
            return Some(config);
        }
    }
    log::debug!("Pinscape Pico: no status reply");
    None
}

/// Plunger sensor type from the Pico's identification report, if it answers.
fn query_pico_id(device: &HidDevice) -> Option<u8> {
    let mut request = [0u8; 64];
    request[0] = PICO_REPORT_ID_REQUEST;
    request[1] = PICO_REQ_QUERY_ID;
    device.write(&request).ok()?;

    // Offset of PlungerType: 1 (report type) + 1 (unit) + 32 (name)
    // + 2 (protocol version) + 8 (hardware ID) + 2 (port count) = 46.
    const PLUNGER_TYPE_OFFSET: usize = 46;
    let mut buffer = [0u8; 64];
    for _ in 0..MAX_REPORTS {
        let read = device.read_timeout(&mut buffer, REPLY_TIMEOUT_MS).ok()?;
        if read > PLUNGER_TYPE_OFFSET + 1 && buffer[0] == PICO_REPORT_ID_ID {
            // UINT16 little-endian, but every defined code fits in a byte.
            return Some(buffer[PLUNGER_TYPE_OFFSET]);
        }
    }
    None
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
