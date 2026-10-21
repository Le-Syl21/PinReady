//! CSD PinOne output boards (Cleveland Software Design).
//!
//! Two generations speak the same 9-byte packet protocol over different
//! transports:
//!
//! - **PinOne** (original, ATmega32U4, USB 0E8F:9207): outputs over the
//!   board's USB CDC serial port. This is the only channel its firmware reads.
//! - **PinOne Ultra** (V3 firmware, ESP32-S3, USB 39F5:9208): outputs over a
//!   raw HID vendor channel (usage page 0xFF00, usage 0x01, report ID 6,
//!   63-byte reports). The board also keeps a CDC port with an optional
//!   "legacy serial DOF" path, but it only understands bank packets there,
//!   not the single-output and admin commands this tester needs.
//!
//! Packet (firmware `Communication.cpp`, both generations):
//!
//! ```text
//!   [0] = 0
//!   [1] = 200 + k  → outputs k*7 .. k*7+6 (0-based) = bytes [2..9]
//!         250      → admin command [2]: 255 = all outputs off, 11 = version
//!         251      → board answers "DEBUG,CSD Board Connected\r\n"
//!         252      → single output: [2] = 0-based index, [3] = value
//! ```
//!
//! Sources: philipellisis/PinOne (V3 firmware: `Communication.cpp`,
//! `Enums.h`, `UsbHid.cpp`), philipellisis/arduino-virtual-pinball-board
//! (original firmware), philipellisis/DirectOutputMain
//! (`PinOneHidCommunication.cs`, `PinOneAutoConfigurator.cs`) and jsm174/libdof
//! (`src/cab/out/pinone`, the DOF that ships inside VPX standalone).

use hidapi::{HidApi, HidDevice};
use serialport::{SerialPort, SerialPortType};
use std::ffi::CStr;
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

// === USB identification ===

/// Original PinOne. Its HID joystick and its CDC serial port are interfaces
/// of the same composite device, so both report this pair. (SDL's joystick
/// GUID spells the PID `0792` because it stores it little-endian.)
pub const LEGACY_VID: u16 = 0x0E8F;
pub const LEGACY_PID: u16 = 0x9207;

/// PinOne Ultra: Cleveland Software Design's own USB-IF vendor ID. The
/// firmware notes that the PID may still be bumped while its HID descriptor
/// settles, so a product string containing "PinOne" ("PinOne Ultra") is
/// accepted as well.
pub const ULTRA_VID: u16 = 0x39F5;
pub const ULTRA_PID: u16 = 0x9208;
/// The vendor collection is the only one of the Ultra's four HID collections
/// (gamepad, consumer control, keyboard, vendor) that accepts packets.
pub const ULTRA_USAGE_PAGE: u16 = 0xFF00;
pub const ULTRA_USAGE: u16 = 0x01;
/// arduino-esp32 `USBHIDVendor` uses `HID_REPORT_ID_VENDOR` = 6.
const ULTRA_REPORT_ID: u8 = 6;
/// Report ID + 63 payload bytes (`USBHIDVendor HidConfig(63, true)`). The
/// firmware queues every payload byte and parses 9 at a time; the zero
/// padding after the packet is discarded by its resync logic.
const ULTRA_REPORT_LEN: usize = 64;

/// Output count DOF declares for a PinOne (`SetNumberOfOutputs(63)`): 15 on
/// the board + up to three 16-output expansion boards.
pub const PINONE_OUTPUTS: u16 = 63;

// === Protocol ===

pub const PACKET_LEN: usize = 9;
/// Bank command base (`200 + k`); only DOF uses it, kept for parity tests.
#[cfg(test)]
const CMD_BANK_BASE: u8 = 200;
const CMD_ADMIN: u8 = 250;
const CMD_CONNECT: u8 = 251;
const CMD_SET_SINGLE: u8 = 252;
const ADMIN_VERSION: u8 = 11;
const ADMIN_OFF: u8 = 255;
const CONNECTED_REPLY: &str = "DEBUG,CSD Board Connected";

/// Serial speed libdof and DOF open the port at. CDC ACM ignores the value
/// for data transfer, but it must NEVER be 1200: opening then closing a
/// 32U4's CDC port at 1200 baud is the Arduino "1200 bps touch" that reboots
/// the board into its bootloader.
const SERIAL_BAUD: u32 = 2_000_000;
/// libdof `TestSerialPort` read/write timeout.
const SERIAL_TIMEOUT_MS: u64 = 100;
/// Extra wait for the version answer (one more main-loop pass on the board).
const VERSION_TIMEOUT_MS: u64 = 300;

/// `[0, 251, 0…]` — "are you a PinOne?".
pub fn connect_packet() -> [u8; PACKET_LEN] {
    [0, CMD_CONNECT, 0, 0, 0, 0, 0, 0, 0]
}

/// `[0, 250, 11, 0…]` — the board answers `V,x.y.z\r\n`.
pub fn version_packet() -> [u8; PACKET_LEN] {
    [0, CMD_ADMIN, ADMIN_VERSION, 0, 0, 0, 0, 0, 0]
}

/// `[0, 250, 255, 0…]` — every output off (`Outputs::turnOff`).
pub fn all_off_packet() -> [u8; PACKET_LEN] {
    [0, CMD_ADMIN, ADMIN_OFF, 0, 0, 0, 0, 0, 0]
}

/// `[0, 252, index, value, 0…]` for output `n` (1-based, as PinReady counts;
/// the firmware's `Outputs::updateOutput` index is 0-based). `None` when `n`
/// is outside 1..=63 — the firmware does not bound-check the index.
pub fn set_output_packet(n: u16, value: u8) -> Option<[u8; PACKET_LEN]> {
    if n == 0 || n > PINONE_OUTPUTS {
        return None;
    }
    Some([0, CMD_SET_SINGLE, (n - 1) as u8, value, 0, 0, 0, 0, 0])
}

/// `[0, 200+k, v0..v6]` — the bank command DOF itself sends. Kept for parity
/// tests with DOF; the tester uses the single-output command.
#[cfg(test)]
pub fn bank_packet(bank: u8, values: &[u8; 7]) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[1] = CMD_BANK_BASE + bank;
    p[2..].copy_from_slice(values);
    p
}

/// Wrap a packet in the Ultra's HID output report: report ID 6, the 9 bytes,
/// zero padding to 64 (same as DirectOutputMain `PinOneHidDevice.WriteUSB`).
pub fn ultra_report(packet: &[u8; PACKET_LEN]) -> [u8; ULTRA_REPORT_LEN] {
    let mut r = [0u8; ULTRA_REPORT_LEN];
    r[0] = ULTRA_REPORT_ID;
    r[1..1 + PACKET_LEN].copy_from_slice(packet);
    r
}

/// Text carried by an Ultra input report: `[6, len, text…]` (the firmware
/// prepends the length). Reports with another ID — the gamepad, keyboard and
/// consumer reports share the device node on Linux — yield `None`.
pub fn ultra_reply_text(report: &[u8]) -> Option<String> {
    if report.len() < 2 || report[0] != ULTRA_REPORT_ID {
        return None;
    }
    let len = (report[1] as usize).min(report.len() - 2);
    Some(String::from_utf8_lossy(&report[2..2 + len]).into_owned())
}

/// Whether accumulated serial/HID text contains the handshake answer.
pub fn is_connected_reply(text: &str) -> bool {
    text.lines().any(|l| l.trim() == CONNECTED_REPLY)
}

/// Firmware version from a `V,x.y.z` line, if one is complete.
pub fn parse_version(text: &str) -> Option<String> {
    // Only lines already terminated: a half-received "V,2.2" must wait.
    let complete = &text[..text.rfind('\n')?];
    complete
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("V,"))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Whether a HID device is the Ultra's vendor channel.
pub fn is_ultra(vid: u16, pid: u16, product: &str) -> bool {
    vid == ULTRA_VID && (pid == ULTRA_PID || product.to_ascii_lowercase().contains("pinone"))
}

// === Serial port discovery (original PinOne) ===

/// What a serial port open/probe ran into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortProblem {
    /// Another program holds the port (VPX/DOF, the PinOne config tool…).
    Busy,
    /// The OS refuses access (Linux without the dialout group / udev rule).
    PermissionDenied,
    /// The port opened but the board did not answer the handshake.
    NoAnswer,
    Other(String),
}

/// Map a `serialport` open error. The port was listed a moment earlier, so
/// `NoDevice` means "held by someone else": serialport reports Windows'
/// ERROR_ACCESS_DENIED (COM ports are exclusive) and Linux' EBUSY (TIOCEXCL)
/// both as `NoDevice`. A real permission problem is Linux' EACCES.
pub fn classify_open_error(kind: serialport::ErrorKind, description: &str) -> PortProblem {
    match kind {
        serialport::ErrorKind::NoDevice => PortProblem::Busy,
        serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied) => {
            PortProblem::PermissionDenied
        }
        _ => PortProblem::Other(description.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialCandidate {
    /// USB VID/PID says it is an original PinOne.
    Matched,
    /// No USB identity available: probe with the libdof handshake only.
    Probe,
    /// Known to be something else, or a duplicate node.
    Skip,
}

/// Decide what to do with one listed serial port. `os` is
/// `std::env::consts::OS` (a parameter so tests cover every platform).
pub fn serial_candidate(port_name: &str, port_type: &SerialPortType, os: &str) -> SerialCandidate {
    // macOS lists each device twice; the callout node (/dev/cu.*) is the one
    // to open, /dev/tty.* waits for carrier detect.
    if os == "macos" && port_name.starts_with("/dev/tty.") {
        return SerialCandidate::Skip;
    }
    match port_type {
        SerialPortType::UsbPort(info) if info.vid == LEGACY_VID && info.pid == LEGACY_PID => {
            SerialCandidate::Matched
        }
        // Any other USB device — including the Ultra's own CDC port, which
        // this tester drives over HID instead.
        SerialPortType::UsbPort(_) => SerialCandidate::Skip,
        SerialPortType::PciPort | SerialPortType::BluetoothPort => SerialCandidate::Skip,
        // On Linux the sysfs scan always identifies USB ports, so an unknown
        // port is a built-in UART, never a PinOne.
        SerialPortType::Unknown if os == "linux" => SerialCandidate::Skip,
        SerialPortType::Unknown => SerialCandidate::Probe,
    }
}

/// Outcome of looking at one serial port.
#[derive(Debug, Clone)]
pub struct SerialFinding {
    pub port: String,
    pub vid: u16,
    pub pid: u16,
    pub product: String,
    pub firmware: Option<String>,
    pub problem: Option<PortProblem>,
}

/// Find original PinOne boards on serial ports: ports matched by USB VID/PID
/// first; only when none matches, probe the ports with no USB identity using
/// libdof's exact handshake. A matched port always yields a finding (with a
/// `problem` if it could not be used) so the UI never says "no board" about
/// a board that is plugged in but busy.
pub fn detect_serial() -> Vec<SerialFinding> {
    let ports = match serialport::available_ports() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("PinOne: serial port enumeration failed: {e}");
            return Vec::new();
        }
    };
    let os = std::env::consts::OS;
    let mut matched = Vec::new();
    let mut probe = Vec::new();
    for p in &ports {
        let candidate = serial_candidate(&p.port_name, &p.port_type, os);
        match &p.port_type {
            SerialPortType::UsbPort(u) => log::info!(
                "  serial {} : USB VID {:04X} PID {:04X} product={:?} → {:?}",
                p.port_name,
                u.vid,
                u.pid,
                u.product,
                candidate
            ),
            other => log::info!("  serial {} : {:?} → {:?}", p.port_name, other, candidate),
        }
        match candidate {
            SerialCandidate::Matched => matched.push(p),
            SerialCandidate::Probe => probe.push(p),
            SerialCandidate::Skip => {}
        }
    }

    let mut out = Vec::new();
    for p in matched {
        let (vid, pid, product) = match &p.port_type {
            SerialPortType::UsbPort(u) => (u.vid, u.pid, u.product.clone().unwrap_or_default()),
            _ => (LEGACY_VID, LEGACY_PID, String::new()),
        };
        let (firmware, problem) = match probe_port(&p.port_name, true) {
            Ok(fw) => (fw, None),
            Err(problem) => (None, Some(problem)),
        };
        log::info!(
            "PinOne candidate {} (VID {vid:04X} PID {pid:04X}, matched by USB id): firmware={firmware:?} problem={problem:?}",
            p.port_name
        );
        out.push(SerialFinding {
            port: p.port_name.clone(),
            vid,
            pid,
            product,
            firmware,
            problem,
        });
    }
    if out.is_empty() {
        for p in probe {
            match probe_port(&p.port_name, false) {
                Ok(_) => {
                    log::info!("PinOne answered on {} (found by probing)", p.port_name);
                    out.push(SerialFinding {
                        port: p.port_name.clone(),
                        vid: LEGACY_VID,
                        pid: LEGACY_PID,
                        product: String::new(),
                        firmware: None,
                        problem: None,
                    });
                }
                Err(problem) => log::debug!("  probe {} : {problem:?}", p.port_name),
            }
        }
    }
    out
}

fn open_serial(port: &str) -> Result<Box<dyn SerialPort>, PortProblem> {
    let mut sp = serialport::new(port, SERIAL_BAUD)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(SERIAL_TIMEOUT_MS))
        .open()
        .map_err(|e| {
            let problem = classify_open_error(e.kind(), &e.description);
            log::warn!(
                "PinOne: open {port} failed: {e} ({:?}) → {problem:?}",
                e.kind()
            );
            problem
        })?;
    // The 32U4 only sends on its CDC port while DTR is up.
    if let Err(e) = sp.write_data_terminal_ready(true) {
        log::warn!("PinOne: set DTR on {port}: {e}");
    }
    if let Err(e) = sp.write_request_to_send(false) {
        log::warn!("PinOne: clear RTS on {port}: {e}");
    }
    Ok(sp)
}

/// Read until `done(text)` or `timeout`, returning everything read.
fn read_until(sp: &mut dyn SerialPort, timeout: Duration, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + timeout;
    let mut text = String::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline && !done(&text) {
        match sp.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => text.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    text
}

/// libdof `TestSerialPort`, then — only on a port matched by VID/PID — the
/// firmware version query and an all-off. Returns the version, if any.
fn probe_port(port: &str, matched: bool) -> Result<Option<String>, PortProblem> {
    let mut sp = open_serial(port)?;
    thread::sleep(Duration::from_millis(20));
    let _ = sp.clear(serialport::ClearBuffer::All);
    thread::sleep(Duration::from_millis(100));
    sp.write_all(&connect_packet())
        .map_err(|e| PortProblem::Other(format!("write: {e}")))?;
    let text = read_until(
        sp.as_mut(),
        Duration::from_millis(SERIAL_TIMEOUT_MS),
        is_connected_reply,
    );
    if !is_connected_reply(&text) {
        log::info!("PinOne: no handshake on {port} (read {:?})", text);
        // libdof's cleanup on a port that is not a PinOne.
        let _ = sp.clear(serialport::ClearBuffer::All);
        let _ = sp.write_request_to_send(false);
        let _ = sp.write_data_terminal_ready(false);
        return Err(PortProblem::NoAnswer);
    }
    if !matched {
        return Ok(None);
    }
    let version = sp.write_all(&version_packet()).ok().and_then(|_| {
        let text = read_until(
            sp.as_mut(),
            Duration::from_millis(VERSION_TIMEOUT_MS),
            |t| parse_version(t).is_some(),
        );
        parse_version(&text)
    });
    // Admin commands pause the board's idle light show; the all-off admin
    // puts it back in its normal "output received" state.
    let _ = sp.write_all(&all_off_packet());
    let _ = sp.flush();
    Ok(version)
}

// === Ultra (HID) helpers ===

/// Ask an Ultra for its firmware version over the vendor channel, then send
/// all-off. `Err` carries the open error text.
pub fn ultra_query_version(api: &HidApi, path: &CStr) -> Result<Option<String>, String> {
    let dev = api.open_path(path).map_err(|e| e.to_string())?;
    dev.write(&ultra_report(&version_packet()))
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_millis(VERSION_TIMEOUT_MS);
    let mut text = String::new();
    let mut buf = [0u8; ULTRA_REPORT_LEN + 1];
    while Instant::now() < deadline && parse_version(&text).is_none() {
        match dev.read_timeout(&mut buf, 50) {
            Ok(n) if n > 0 => {
                if let Some(t) = ultra_reply_text(&buf[..n]) {
                    text.push_str(&t);
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = dev.write(&ultra_report(&all_off_packet()));
    Ok(parse_version(&text))
}

// === Output drivers ===

/// Original PinOne over its CDC serial port.
pub struct PinOneSerial {
    port: Box<dyn SerialPort>,
    name: String,
    num_outputs: u16,
}

impl PinOneSerial {
    pub fn open(port: &str, num_outputs: u16) -> Result<Self, String> {
        log::info!("Opening PinOne on {port} ({num_outputs} outputs)");
        let sp = open_serial(port).map_err(|p| match p {
            PortProblem::Busy => format!("{port} is in use by another program"),
            PortProblem::PermissionDenied => format!("{port}: permission denied"),
            PortProblem::NoAnswer => format!("{port}: no answer"),
            PortProblem::Other(e) => format!("{port}: {e}"),
        })?;
        Ok(Self {
            port: sp,
            name: port.to_string(),
            num_outputs: num_outputs.min(PINONE_OUTPUTS),
        })
    }

    pub fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    pub fn send(&mut self, packet: &[u8; PACKET_LEN]) -> Result<(), String> {
        self.port
            .write_all(packet)
            .and_then(|_| self.port.flush())
            .map_err(|e| format!("PinOne serial write on {}: {e}", self.name))
    }
}

/// PinOne Ultra over its HID vendor channel.
pub struct PinOneUltraHid {
    dev: HidDevice,
    num_outputs: u16,
}

impl PinOneUltraHid {
    pub fn open(path: &CStr, num_outputs: u16) -> Result<Self, String> {
        log::info!(
            "Opening PinOne Ultra: {} ({num_outputs} outputs)",
            path.to_string_lossy()
        );
        let api = HidApi::new().map_err(|e| format!("HidApi init failed: {e}"))?;
        let dev = api
            .open_path(path)
            .map_err(|e| format!("Open HID device: {e}"))?;
        Ok(Self {
            dev,
            num_outputs: num_outputs.min(PINONE_OUTPUTS),
        })
    }

    pub fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    pub fn send(&mut self, packet: &[u8; PACKET_LEN]) -> Result<(), String> {
        self.dev
            .write(&ultra_report(packet))
            .map(|_| ())
            .map_err(|e| format!("PinOne Ultra HID write: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::UsbPortInfo;

    fn usb(vid: u16, pid: u16) -> SerialPortType {
        SerialPortType::UsbPort(UsbPortInfo {
            vid,
            pid,
            serial_number: None,
            manufacturer: None,
            product: None,
        })
    }

    #[test]
    fn packets_match_firmware() {
        assert_eq!(connect_packet(), [0, 251, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(version_packet(), [0, 250, 11, 0, 0, 0, 0, 0, 0]);
        assert_eq!(all_off_packet(), [0, 250, 255, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn single_output_is_zero_based_and_bounded() {
        assert_eq!(
            set_output_packet(1, 255),
            Some([0, 252, 0, 255, 0, 0, 0, 0, 0])
        );
        assert_eq!(
            set_output_packet(63, 7),
            Some([0, 252, 62, 7, 0, 0, 0, 0, 0])
        );
        assert_eq!(set_output_packet(0, 255), None);
        assert_eq!(set_output_packet(64, 255), None);
    }

    #[test]
    fn bank_packet_matches_dof() {
        // DOF UpdateOutputs: [0, 200+k, values of outputs k*7..k*7+6].
        assert_eq!(
            bank_packet(2, &[1, 2, 3, 4, 5, 6, 7]),
            [0, 202, 1, 2, 3, 4, 5, 6, 7]
        );
        // 63 outputs = banks 200..=208, the range the firmware accepts.
        assert_eq!(PINONE_OUTPUTS.div_ceil(7), 9);
    }

    #[test]
    fn ultra_report_layout() {
        let r = ultra_report(&set_output_packet(5, 200).unwrap());
        assert_eq!(r.len(), 64);
        assert_eq!(&r[..10], &[6, 0, 252, 4, 200, 0, 0, 0, 0, 0]);
        assert!(r[10..].iter().all(|&b| b == 0));
    }

    #[test]
    fn ultra_reply_parsing() {
        let mut rep = vec![6u8, 9];
        rep.extend_from_slice(b"V,3.0.0\r\n");
        rep.resize(64, 0);
        let text = ultra_reply_text(&rep).unwrap();
        assert_eq!(parse_version(&text).as_deref(), Some("3.0.0"));
        // Gamepad report (ID 1) on the same Linux hidraw node: ignored.
        assert_eq!(ultra_reply_text(&[1, 9, b'V', b',']), None);
        // Length byte larger than the report: clamped, no panic.
        assert_eq!(ultra_reply_text(&[6, 200, b'V']).as_deref(), Some("V"));
    }

    #[test]
    fn handshake_and_version_parsing() {
        assert!(is_connected_reply("DEBUG,CSD Board Connected\r\n"));
        assert!(is_connected_reply("junk\r\nDEBUG,CSD Board Connected\r\n"));
        assert!(!is_connected_reply("DEBUG,CSD Board Conn"));
        assert_eq!(parse_version("V,2.2.1\r\n").as_deref(), Some("2.2.1"));
        // Incomplete line: wait for the rest.
        assert_eq!(parse_version("V,2.2"), None);
        assert_eq!(parse_version("DEBUG,x\r\n"), None);
    }

    #[test]
    fn ultra_identification() {
        assert!(is_ultra(0x39F5, 0x9208, ""));
        assert!(is_ultra(0x39F5, 0x9209, "PinOne Ultra"));
        assert!(!is_ultra(0x39F5, 0x1234, "Something"));
        assert!(!is_ultra(0x0E8F, 0x9207, "PinOne"));
    }

    #[test]
    fn serial_candidates() {
        use SerialCandidate::*;
        assert_eq!(
            serial_candidate("COM5", &usb(0x0E8F, 0x9207), "windows"),
            Matched
        );
        assert_eq!(
            serial_candidate("/dev/ttyACM0", &usb(0x0E8F, 0x9207), "linux"),
            Matched
        );
        // Other USB serial devices and the Ultra's CDC port are left alone.
        assert_eq!(
            serial_candidate("COM3", &usb(0x2341, 0x8036), "windows"),
            Skip
        );
        assert_eq!(
            serial_candidate("COM7", &usb(0x39F5, 0x9208), "windows"),
            Skip
        );
        // GreenAsia (0E8F) makes generic gamepads too: the PID must match.
        assert_eq!(
            serial_candidate("COM8", &usb(0x0E8F, 0x0003), "windows"),
            Skip
        );
        // No USB identity: probe on Windows/macOS, never on Linux (UART).
        assert_eq!(
            serial_candidate("COM1", &SerialPortType::Unknown, "windows"),
            Probe
        );
        assert_eq!(
            serial_candidate("/dev/ttyS0", &SerialPortType::Unknown, "linux"),
            Skip
        );
        assert_eq!(
            serial_candidate("COM4", &SerialPortType::BluetoothPort, "windows"),
            Skip
        );
        // macOS: only the callout node.
        assert_eq!(
            serial_candidate("/dev/tty.usbmodem1", &usb(0x0E8F, 0x9207), "macos"),
            Skip
        );
        assert_eq!(
            serial_candidate("/dev/cu.usbmodem1", &usb(0x0E8F, 0x9207), "macos"),
            Matched
        );
    }

    #[test]
    fn open_errors_map_to_user_problems() {
        use serialport::ErrorKind;
        // Windows ERROR_ACCESS_DENIED and Linux EBUSY both arrive as NoDevice.
        assert_eq!(
            classify_open_error(ErrorKind::NoDevice, "Access is denied."),
            PortProblem::Busy
        );
        assert_eq!(
            classify_open_error(
                ErrorKind::Io(std::io::ErrorKind::PermissionDenied),
                "Permission denied"
            ),
            PortProblem::PermissionDenied
        );
        assert_eq!(
            classify_open_error(ErrorKind::InvalidInput, "bad"),
            PortProblem::Other("bad".into())
        );
    }

    #[test]
    fn baud_is_never_the_bootloader_touch() {
        assert_ne!(SERIAL_BAUD, 1200);
    }
}
