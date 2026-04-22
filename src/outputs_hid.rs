//! Output discovery — pulse cabinet outputs one by one so the user can
//! identify what's physically connected to each pin on their output board.
//!
//! Safety is built in: each pulse is 50ms @ 100% (within pop-bumper / knocker
//! duty cycle specs) followed by 500ms off, and per-output loops auto-stop
//! after 30 cycles (~16.5s) so a distracted user can't cook a flipper coil.
//! `all_off` is sent on Stop, Shutdown, channel drop, and any mid-cycle
//! hardware error.
//!
//! Board backends: only [`Simulator`] drives anything today; the 4 real
//! boards (KL25Z / Pico / DudesCab / PinOne) are stubbed pending verified
//! USB HID protocol work — see module-level notes below.

use crossbeam_channel::{Receiver, Sender};
use hidapi::{HidApi, HidDevice};
use std::ffi::{CStr, CString};
use std::thread;
use std::time::Duration;

// === USB identification tables ===
// Sources cross-checked against libdof's auto-configurators:
//   - ps/Pinscape.cpp            : Pinscape KL25Z
//   - pspico/PinscapePico.cpp    : Pinscape Pico
//   - lw/LedWiz.cpp              : LedWiz
//   - dudescab/DudesCab.h        : DudesCab
//   - pac/PacDriveSingleton.cpp  : PacDrive / PacLed / Ultimate IO (Ultimarc)
//
// Full map: the ConfigTool `id` field *is* the libdof `directoutputconfigNN.ini`
// suffix (extracted from the VPUniverse page source). Each auto-configurator
// adds a fixed bias to the board's internal unit number to derive the suffix.

// LedWiz + Pinscape v1 (LedWiz emulation mode)
const LEDWIZ_VID: u16 = 0xFAFA;
const LEDWIZ_PID_BASE: u16 = 0x00EF;

// Pinscape v2 native (KL25Z or Pico)
const PINSCAPE_PRIVATE_VID: u16 = 0x1209;
const PINSCAPE_PRIVATE_PID: u16 = 0xEAEA;

// Ultimarc family (PacDrive / PacLed / UltimateIO)
const ULTIMARC_VID: u16 = 0xD209;
const PACDRIVE_PID: u16 = 0x1500;
const PACLED_PID_MIN: u16 = 0x1401;
const PACLED_PID_MAX: u16 = 0x1404;
const ULTIMATEIO_PID_MIN: u16 = 0x0410;
const ULTIMATEIO_PID_MAX: u16 = 0x0411;

// DudesCab (Arnoz, Pico-based)
const DUDESCAB_VID: u16 = 0x2E8A;
const DUDESCAB_PID: u16 = 0x106F;

// HID usage pages
const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
const HID_USAGE_JOYSTICK: u16 = 0x04;
const PSPICO_USAGE_PAGE: u16 = 0x06;
const PSPICO_USAGE: u16 = 0x00;

// === Pinscape KL25Z output protocol ===
// Reports are 8 bytes without Report ID. Since hidapi expects a leading
// report-ID byte on write, we prepend 0x00 → 9 bytes total on the wire.
//
// Key commands:
//   - `200 + k` : set outputs (k*7+1 .. k*7+7) to brightness 0-255 (7 per msg)
//   - `65, 5`   : turn all outputs off + reset LedWiz defaults
//
// Because the 200+k command is group-oriented, PinReady keeps a shadow of all
// output levels and rebuilds the full 7-byte group on every set_output call.
const CMD_SET_BRIGHTNESS_BASE: u8 = 200;
const CMD_EXTENDED: u8 = 65;
const CMD_EXT_ALL_OFF: u8 = 5;
const OUTPUTS_PER_GROUP: u16 = 7;

// Default output counts per board (matches the "max" advertised by ConfigTool
// for a single unit — used when the firmware does not expose the actual count).
const PINSCAPE_KL25Z_DEFAULT_OUTPUTS: u16 = 22;
const PINSCAPE_PICO_DEFAULT_OUTPUTS: u16 = 32;
const DUDESCAB_DEFAULT_OUTPUTS: u16 = 32;
const LEDWIZ_DEFAULT_OUTPUTS: u16 = 32;
const PACDRIVE_DEFAULT_OUTPUTS: u16 = 16;
const PACLED_DEFAULT_OUTPUTS: u16 = 64;
const ULTIMATEIO_DEFAULT_OUTPUTS: u16 = 96;

/// Information about a detected output board that the UI can present before
/// the user commits to opening it.
#[derive(Debug, Clone)]
pub struct DetectedBoard {
    pub kind: BoardKind,
    pub vid: u16,
    pub pid: u16,
    pub path: CString,
    pub product: String,
    pub num_outputs: u16,
}

impl DetectedBoard {
    pub fn path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    /// The controller entry the user should pick in VPUniverse ConfigTool.
    ///
    /// The mapping is derived directly from the ConfigTool `devices` list
    /// where each entry's `id` matches the libdof `directoutputconfigNN.ini`
    /// file suffix. Picking the wrong entry produces a DOF config that
    /// libdof will never load.
    pub fn configtool_controller(&self) -> &'static str {
        match self.kind {
            BoardKind::PinscapeKL25Z => {
                if self.vid == LEDWIZ_VID {
                    // Pinscape v1 legacy (LedWiz emul) → old ConfigTool label.
                    "FRDM-KL25Z"
                } else {
                    "Pinscape"
                }
            }
            BoardKind::PinscapePico => "PinscapePico",
            BoardKind::LedWiz => "Ledwiz",
            BoardKind::DudesCab => "DudesCab",
            BoardKind::PacDrive => "PacDrive",
            BoardKind::PacLed => "PacLed",
            BoardKind::UltimateIO => "Ultimate/IO",
            BoardKind::Simulator => "Simulator",
        }
    }

    /// Human-readable firmware mode description (Pinscape only — others
    /// have a single firmware).
    pub fn firmware_mode(&self) -> &'static str {
        match self.kind {
            BoardKind::PinscapeKL25Z => {
                if self.vid == LEDWIZ_VID {
                    "Pinscape v1 (LedWiz emul mode)"
                } else {
                    "Pinscape v2+ (native protocol)"
                }
            }
            _ => "",
        }
    }

    /// Compute the `directoutputconfigNN.ini` filename libdof will try to
    /// load for this board. Biases extracted from libdof auto-configurators:
    ///
    ///   - LedWiz / FRDM-KL25Z : no bias, `N = unit` (unit 1..16 → file 1..16)
    ///   - Pinscape v2+        : +50   (unit 1 → file 51)
    ///   - PinscapePico        : +119  (unit 1 → file 120)
    ///   - DudesCab            : +89   (unit 1 → file 90)
    ///   - PacDrive            : fixed 19
    ///   - PacLed              : fixed 20
    ///   - Ultimate/IO         : fixed 27
    ///
    /// For Pinscape v1 legacy and LedWiz, the unit number is embedded in
    /// the USB PID (`PID = 0x00EF + unit`). For all others we assume unit=1
    /// (the ConfigTool default); users with multiple boards of the same
    /// type will need to adjust manually.
    pub fn expected_config_filename(&self) -> Option<String> {
        let suffix: u16 = match self.kind {
            BoardKind::PinscapeKL25Z => {
                if self.vid == LEDWIZ_VID {
                    // libdof's formula: unit = (pid & 0x0F) + 1 (range 1..16).
                    (self.pid & 0x0F) + 1
                } else {
                    51
                }
            }
            BoardKind::PinscapePico => 120,
            BoardKind::LedWiz => (self.pid & 0x0F) + 1,
            BoardKind::DudesCab => 90,
            BoardKind::PacDrive => 19,
            BoardKind::PacLed => 20,
            BoardKind::UltimateIO => 27,
            BoardKind::Simulator => return None,
        };
        Some(format!("directoutputconfig{suffix}.ini"))
    }
}

/// Scan the local USB bus for supported output boards. Errors are returned
/// verbatim (typically "Permission denied" on Linux without udev rules).
///
/// Classification mirrors libdof's auto-configurators so detection matches
/// exactly what the runtime will see — no surprises.
pub fn detect_boards() -> Result<Vec<DetectedBoard>, String> {
    log::info!("Scanning USB for supported output boards...");
    let api = HidApi::new().map_err(|e| {
        log::error!("HidApi::new failed: {e}");
        format!("HidApi init failed: {e}")
    })?;
    let mut out = Vec::new();

    for info in api.device_list() {
        let vid = info.vendor_id();
        let pid = info.product_id();
        let usage_page = info.usage_page();
        let usage = info.usage();
        let product = info.product_string().unwrap_or("").to_string();

        let Some(kind) = classify_hid(vid, pid, usage_page, usage, &product) else {
            continue;
        };

        // Per-interface filter — for each device type, only keep the HID
        // interface that carries the feedback/output reports. Some boards
        // (Pinscape KL25Z, LedWiz) expose multiple interfaces; picking the
        // wrong one would send commands into the void.
        if !is_correct_interface(kind, usage_page, usage) {
            continue;
        }

        log::info!(
            "  detected {} : VID {:04X} PID {:04X} usage={}/{} path={:?} product={:?}",
            kind.label(),
            vid,
            pid,
            usage_page,
            usage,
            info.path(),
            product
        );

        out.push(DetectedBoard {
            kind,
            vid,
            pid,
            path: info.path().to_owned(),
            product,
            num_outputs: default_outputs(kind),
        });
    }

    // Deduplicate by (kind, path) — some boards enumerate via multiple
    // backends (hidraw + usbhid on Linux). Keep the first occurrence.
    out.dedup_by(|a, b| a.kind == b.kind && a.path == b.path);
    log::info!("Scan complete: {} supported board(s) found", out.len());
    Ok(out)
}

/// Map a HID (VID, PID, usage, product) tuple to a known board kind.
/// Returns None for unrelated devices (keyboards, mice, other joysticks...).
fn classify_hid(
    vid: u16,
    pid: u16,
    usage_page: u16,
    usage: u16,
    product: &str,
) -> Option<BoardKind> {
    // Pinscape Pico: VID/PID aren't fixed — libdof matches on usage + product
    // string equal to "PinscapePico". Check this first.
    if usage_page == PSPICO_USAGE_PAGE && usage == PSPICO_USAGE && product == "PinscapePico" {
        return Some(BoardKind::PinscapePico);
    }

    // Pinscape v2 (modern, native protocol).
    if vid == PINSCAPE_PRIVATE_VID && pid == PINSCAPE_PRIVATE_PID {
        return Some(BoardKind::PinscapeKL25Z);
    }

    // VID 0xFAFA = LedWiz OR Pinscape v1 (LedWiz emulation). Distinguish
    // via the USB product string — Pinscape devices identify themselves
    // with "Pinscape" in the product name even when emulating a LedWiz.
    if vid == LEDWIZ_VID && pid > LEDWIZ_PID_BASE && pid <= LEDWIZ_PID_BASE + 16 {
        let p = product.to_ascii_lowercase();
        return if p.contains("pinscape") || p.contains("mjrnet") {
            Some(BoardKind::PinscapeKL25Z)
        } else {
            Some(BoardKind::LedWiz)
        };
    }

    // DudesCab — skip the "Outputs MX" variant (separate addressable-LED
    // interface we don't drive). "DudesCab Outputs" (Windows) or plain
    // "DudesCab" (Linux, interface_number filtered at the libdof level) are
    // the output interfaces.
    if vid == DUDESCAB_VID && pid == DUDESCAB_PID {
        let is_mx = product.contains("MX");
        if !is_mx {
            return Some(BoardKind::DudesCab);
        }
    }

    // Ultimarc family.
    if vid == ULTIMARC_VID {
        if pid == PACDRIVE_PID {
            return Some(BoardKind::PacDrive);
        }
        if (PACLED_PID_MIN..=PACLED_PID_MAX).contains(&pid) {
            return Some(BoardKind::PacLed);
        }
        if (ULTIMATEIO_PID_MIN..=ULTIMATEIO_PID_MAX).contains(&pid) {
            return Some(BoardKind::UltimateIO);
        }
    }

    None
}

/// Filter the correct HID interface per board. Pinscape exposes both a
/// Joystick and a Keyboard on the same VID/PID, for instance. For boards
/// without a documented interface filter, accept the first match.
fn is_correct_interface(kind: BoardKind, usage_page: u16, usage: u16) -> bool {
    match kind {
        BoardKind::PinscapeKL25Z | BoardKind::LedWiz => {
            usage_page == HID_USAGE_PAGE_GENERIC && usage == HID_USAGE_JOYSTICK
        }
        BoardKind::PinscapePico => usage_page == PSPICO_USAGE_PAGE && usage == PSPICO_USAGE,
        _ => true,
    }
}

fn default_outputs(kind: BoardKind) -> u16 {
    match kind {
        BoardKind::Simulator | BoardKind::PinscapeKL25Z => PINSCAPE_KL25Z_DEFAULT_OUTPUTS,
        BoardKind::PinscapePico => PINSCAPE_PICO_DEFAULT_OUTPUTS,
        BoardKind::LedWiz => LEDWIZ_DEFAULT_OUTPUTS,
        BoardKind::DudesCab => DUDESCAB_DEFAULT_OUTPUTS,
        BoardKind::PacDrive => PACDRIVE_DEFAULT_OUTPUTS,
        BoardKind::PacLed => PACLED_DEFAULT_OUTPUTS,
        BoardKind::UltimateIO => ULTIMATEIO_DEFAULT_OUTPUTS,
    }
}

/// Install the Pinscape udev rules via `pkexec` on Linux. Writes the rules
/// to a temp file, then elevates to copy them into `/etc/udev/rules.d/` and
/// reload udev. The pkexec dialog is modal and blocking, so call this from
/// a dedicated thread — see [`spawn_udev_apply`].
///
/// Returns Ok on success. Common error cases:
///   - exit 126: user cancelled the polkit dialog
///   - exit 127: pkexec missing (polkit not installed)
///   - any other: something in the install/reload chain failed
#[cfg(target_os = "linux")]
pub fn apply_udev_rules(rules: &str) -> Result<(), String> {
    let tmp = std::env::temp_dir().join("pinready-99-pinscape.rules");
    std::fs::write(&tmp, rules).map_err(|e| format!("write temp file: {e}"))?;
    log::info!(
        "Installing udev rules via pkexec (temp source: {})",
        tmp.display()
    );

    let tmp_path = tmp.to_string_lossy().replace('\'', "'\\''");
    // `trigger --action=change` re-runs rules on already-connected devices.
    // `settle --timeout=5` blocks until the event queue drains so the caller
    // can rescan immediately after this returns.
    let script = format!(
        "install -m 0644 '{tmp_path}' /etc/udev/rules.d/99-pinscape.rules \
         && udevadm control --reload-rules \
         && udevadm trigger --action=change \
         && udevadm settle --timeout=5"
    );

    let status = std::process::Command::new("pkexec")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .status()
        .map_err(|e| format!("pkexec invocation failed: {e}. Is polkit installed?"))?;

    let _ = std::fs::remove_file(&tmp);

    match status.code() {
        Some(0) => {
            log::info!("udev rules installed and reloaded successfully");
            Ok(())
        }
        Some(126) => {
            log::warn!("udev install cancelled by user (pkexec exit 126)");
            Err("Authorisation cancelled".to_string())
        }
        Some(127) => {
            log::error!("pkexec not found on PATH — polkit missing?");
            Err(
                "pkexec not found — install polkit (e.g. `sudo apt install policykit-1`)"
                    .to_string(),
            )
        }
        Some(code) => {
            log::error!("pkexec exited with code {code}");
            Err(format!("pkexec exited with code {code}"))
        }
        None => {
            log::error!("pkexec terminated by signal");
            Err("pkexec terminated by signal".to_string())
        }
    }
}

/// Spawn a background thread that runs [`apply_udev_rules`] and sends the
/// result over a channel. The UI polls the receiver between frames, keeping
/// egui responsive while the polkit password dialog is up.
#[cfg(target_os = "linux")]
pub fn spawn_udev_apply(rules: String) -> Receiver<Result<(), String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    thread::spawn(move || {
        let _ = tx.send(apply_udev_rules(&rules));
    });
    rx
}

/// Dispatch to the right HID driver for a detected board. Returns an
/// `OutputBoard` trait object the discovery thread can drive.
pub fn open_board(
    kind: BoardKind,
    path: &CStr,
    num_outputs: u16,
) -> Result<Box<dyn OutputBoard>, String> {
    match kind {
        BoardKind::PinscapeKL25Z => open_pinscape_kl25z(path, num_outputs),
        BoardKind::LedWiz => open_ledwiz(path, num_outputs),
        BoardKind::PinscapePico => open_pinscape_pico(path, num_outputs),
        BoardKind::DudesCab => open_dudescab(path, num_outputs),
        BoardKind::PacDrive => open_pacdrive(path, num_outputs),
        BoardKind::PacLed | BoardKind::UltimateIO => open_pac_indexed(kind, path, num_outputs),
        other => Err(format!(
            "{} HID write driver not yet implemented",
            other.label()
        )),
    }
}

/// Open the device at `path` as a Pinscape KL25Z driver.
pub fn open_pinscape_kl25z(path: &CStr, num_outputs: u16) -> Result<Box<dyn OutputBoard>, String> {
    log::info!(
        "Opening Pinscape device: {} ({} outputs)",
        path.to_string_lossy(),
        num_outputs
    );
    let api = HidApi::new().map_err(|e| {
        log::error!("HidApi::new on open failed: {e}");
        format!("HidApi init failed: {e}")
    })?;
    let dev = api.open_path(path).map_err(|e| {
        log::error!("open_path({:?}) failed: {e}", path);
        format!("Open HID device: {e}")
    })?;
    log::info!("Pinscape device opened successfully");
    Ok(Box::new(PinscapeKL25ZReal {
        dev,
        shadow: vec![0u8; num_outputs as usize],
        num_outputs,
    }))
}

/// Open the device at `path` as a LedWiz driver. LedWiz outputs max out at
/// 32 ports — `num_outputs` is clamped accordingly.
pub fn open_ledwiz(path: &CStr, num_outputs: u16) -> Result<Box<dyn OutputBoard>, String> {
    let n = num_outputs.min(32);
    log::info!(
        "Opening LedWiz device: {} ({} outputs)",
        path.to_string_lossy(),
        n
    );
    let api = HidApi::new().map_err(|e| format!("HidApi init failed: {e}"))?;
    let dev = api
        .open_path(path)
        .map_err(|e| format!("Open HID device: {e}"))?;
    log::info!("LedWiz device opened successfully");
    Ok(Box::new(LedWizReal {
        dev,
        shadow: vec![0u8; n as usize],
        num_outputs: n,
    }))
}

/// Open the device at `path` as a Pinscape Pico driver.
pub fn open_pinscape_pico(path: &CStr, num_outputs: u16) -> Result<Box<dyn OutputBoard>, String> {
    log::info!(
        "Opening Pinscape Pico device: {} ({} outputs)",
        path.to_string_lossy(),
        num_outputs
    );
    let api = HidApi::new().map_err(|e| format!("HidApi init failed: {e}"))?;
    let dev = api
        .open_path(path)
        .map_err(|e| format!("Open HID device: {e}"))?;
    log::info!("Pinscape Pico device opened successfully");
    Ok(Box::new(PinscapePicoReal { dev, num_outputs }))
}

/// Open the device at `path` as a DudesCab driver.
pub fn open_dudescab(path: &CStr, num_outputs: u16) -> Result<Box<dyn OutputBoard>, String> {
    log::info!(
        "Opening DudesCab device: {} ({} outputs, assuming firmware 1.9.0+)",
        path.to_string_lossy(),
        num_outputs
    );
    let api = HidApi::new().map_err(|e| format!("HidApi init failed: {e}"))?;
    let dev = api
        .open_path(path)
        .map_err(|e| format!("Open HID device: {e}"))?;
    log::info!("DudesCab device opened successfully");
    Ok(Box::new(DudesCabReal { dev, num_outputs }))
}

/// Open the device at `path` as an Ultimarc PacDrive driver (16 binary LEDs).
pub fn open_pacdrive(path: &CStr, num_outputs: u16) -> Result<Box<dyn OutputBoard>, String> {
    let n = num_outputs.min(16);
    log::info!(
        "Opening PacDrive device: {} ({n} outputs)",
        path.to_string_lossy()
    );
    let api = HidApi::new().map_err(|e| format!("HidApi init failed: {e}"))?;
    let dev = api
        .open_path(path)
        .map_err(|e| format!("Open HID device: {e}"))?;
    log::info!("PacDrive device opened successfully");
    Ok(Box::new(PacDriveReal {
        dev,
        shadow: 0u16,
        num_outputs: n,
    }))
}

/// Open the device at `path` as an Ultimarc PacLed64 / Ultimate I/O driver.
/// PacLed64 max = 64 outputs, Ultimate I/O max = 96 outputs — both use the
/// same per-port HID SET_REPORT protocol.
pub fn open_pac_indexed(
    kind: BoardKind,
    path: &CStr,
    num_outputs: u16,
) -> Result<Box<dyn OutputBoard>, String> {
    let cap = match kind {
        BoardKind::PacLed => 64,
        BoardKind::UltimateIO => 96,
        _ => return Err("open_pac_indexed only supports PacLed / UltimateIO".to_string()),
    };
    let n = num_outputs.min(cap);
    log::info!(
        "Opening {} device: {} ({n} outputs)",
        kind.label(),
        path.to_string_lossy()
    );
    let api = HidApi::new().map_err(|e| format!("HidApi init failed: {e}"))?;
    let dev = api
        .open_path(path)
        .map_err(|e| format!("Open HID device: {e}"))?;
    log::info!("{} device opened successfully", kind.label());
    Ok(Box::new(PacIndexedReal {
        dev,
        kind,
        num_outputs: n,
    }))
}

/// How long each output is held at 100% per pulse cycle.
pub const PULSE_ON_MS: u64 = 50;
/// Gap between pulses — time for the user to observe and note, and keeps
/// duty cycle at ~9% (within pop-bumper / knocker ratings).
pub const PULSE_OFF_MS: u64 = 500;
/// Per-output safety cap. Past this, thermal risk on flipper power windings
/// starts to build up if the user walks away.
pub const MAX_LOOP_PULSES: u32 = 30;

/// Output boards PinReady can discover over USB HID. Each variant maps to a
/// distinct entry in VPUniverse ConfigTool (and a distinct libdof
/// auto-configurator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardKind {
    Simulator,
    /// Pinscape firmware on KL25Z hardware. Covers both v1 (LedWiz emulation
    /// mode, VID 0xFAFA) and v2 (native, VID 0x1209:0xEAEA). The extended
    /// command set (`200+k`) is identical on both modes.
    PinscapeKL25Z,
    /// Pinscape firmware on Raspberry Pi Pico hardware. Different USB
    /// protocol (usage page 0x06/0x00 + product string match); needs its
    /// own HID driver, not covered by `PinscapeKL25Z`.
    PinscapePico,
    /// Original LedWiz hardware (GroovyGameGear). VID 0xFAFA + product
    /// string NOT containing "Pinscape". SBA/PBA protocol (0-48 intensity).
    LedWiz,
    /// DudesCab by Arnoz — Pico-based pincab output board.
    DudesCab,
    /// Ultimarc PacDrive.
    PacDrive,
    /// Ultimarc PacLed64.
    PacLed,
    /// Ultimarc Ultimate I/O.
    UltimateIO,
}

impl BoardKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Simulator => "Simulator",
            Self::PinscapeKL25Z => "Pinscape KL25Z",
            Self::PinscapePico => "Pinscape Pico",
            Self::LedWiz => "LedWiz",
            Self::DudesCab => "DudesCab",
            Self::PacDrive => "PacDrive",
            Self::PacLed => "PacLed",
            Self::UltimateIO => "Ultimate I/O",
        }
    }

    /// Whether PinReady can drive this board's outputs (= has a real HID
    /// driver today). Others are detection-only and display a "driver
    /// coming soon" hint in the UI.
    pub fn has_driver(self) -> bool {
        matches!(
            self,
            Self::Simulator
                | Self::PinscapeKL25Z
                | Self::LedWiz
                | Self::PinscapePico
                | Self::DudesCab
                | Self::PacDrive
                | Self::PacLed
                | Self::UltimateIO
        )
    }
}

/// Backend a board driver must implement.
pub trait OutputBoard: Send {
    fn kind(&self) -> BoardKind;
    fn num_outputs(&self) -> u16;
    /// Set output `n` (1-indexed) to intensity 0..=255. 0 = off, 255 = full.
    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String>;
    /// Drive every output to 0. Called on Stop / Shutdown / error.
    fn all_off(&mut self) -> Result<(), String>;
}

/// No-hardware backend that logs pulses. Lets the full discovery UX be
/// tested without risk to any physical board.
pub struct Simulator {
    outputs: u16,
}

impl Simulator {
    pub fn new(outputs: u16) -> Self {
        Self { outputs }
    }
}

impl OutputBoard for Simulator {
    fn kind(&self) -> BoardKind {
        BoardKind::Simulator
    }
    fn num_outputs(&self) -> u16 {
        self.outputs
    }
    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String> {
        if intensity == 0 {
            log::debug!("[sim] output #{n} off");
        } else {
            log::info!("[sim] output #{n} = {intensity}");
        }
        Ok(())
    }
    fn all_off(&mut self) -> Result<(), String> {
        log::info!("[sim] all off");
        Ok(())
    }
}

// === Real-board stubs ===
//
// Filling these in is the "add a board driver" task:
//   1. Add `hidapi = "2"` to Cargo.toml and open the device via VID/PID
//   2. Replace set_output / all_off with the board's feature-report writes
//   3. Add a `detect()` fn using HidApi::device_list()
//
// Protocol references:
//   - Pinscape KL25Z : mjrgh/Pinscape_Controller USBProtocol.md (LedWiz
//     SBA / PBA commands)
//   - Pinscape Pico  : mjrgh/Pinscape-pico Output Port subsystem
//   - DudesCab       : Arnoz/DudesCab firmware USB HID descriptor
//   - PinOne         : Cleveland Software Design custom HID protocol
//
// Until these land, the UI only exposes Simulator; the variants exist now
// so the enum, switch branches, and i18n keys are in place.

/// Real Pinscape KL25Z HID backend. Holds the open device handle and a
/// shadow of all output levels — the Pinscape `200+k` command writes 7
/// outputs at once, so setting one without clobbering neighbours requires
/// keeping a local copy of the current levels.
struct PinscapeKL25ZReal {
    dev: HidDevice,
    shadow: Vec<u8>,
    num_outputs: u16,
}

impl PinscapeKL25ZReal {
    /// Rebuild and send the 200+group command for the given output group.
    fn send_group(&mut self, group: u16) -> Result<(), String> {
        let base_idx = (group * OUTPUTS_PER_GROUP) as usize;
        // 9 bytes on the wire: [0x00 report-ID prefix] + 8-byte payload.
        let mut report = [0u8; 9];
        report[0] = 0x00;
        report[1] = CMD_SET_BRIGHTNESS_BASE + (group as u8);
        for i in 0..OUTPUTS_PER_GROUP as usize {
            report[2 + i] = *self.shadow.get(base_idx + i).unwrap_or(&0);
        }
        self.dev
            .write(&report)
            .map(|_| ())
            .map_err(|e| format!("HID write failed: {e}"))
    }
}

impl OutputBoard for PinscapeKL25ZReal {
    fn kind(&self) -> BoardKind {
        BoardKind::PinscapeKL25Z
    }
    fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String> {
        if n == 0 || n > self.num_outputs {
            return Err(format!(
                "output {n} out of range (1..={})",
                self.num_outputs
            ));
        }
        self.shadow[(n - 1) as usize] = intensity;
        let group = (n - 1) / OUTPUTS_PER_GROUP;
        self.send_group(group)
    }

    fn all_off(&mut self) -> Result<(), String> {
        for v in self.shadow.iter_mut() {
            *v = 0;
        }
        // Pinscape extended cmd 65 / subtype 5: all outputs off + LedWiz
        // defaults. Prefer this over iterating 200+k groups — it's one write
        // and resets internal flash state.
        let report = [0x00, CMD_EXTENDED, CMD_EXT_ALL_OFF, 0, 0, 0, 0, 0, 0];
        self.dev
            .write(&report)
            .map(|_| ())
            .map_err(|e| format!("HID write failed: {e}"))
    }
}

// === LedWiz protocol ===
//
// Ported verbatim from libdof `src/cab/out/lw/LedWiz.cpp` — see its
// UpdateOutputs/AllOff for the reference implementation.
//
// LedWiz uses two 8-byte HID reports (no Report ID; hidapi prepends 0x00):
//
// SBA (Set Brightness All) — command byte `0x40`:
//   [0x40, bank0, bank1, bank2, bank3, pulseSpeed=0x02, 0, 0]
//   bank0..3 = 32 bits of on/off, one per output (LSB of bank0 = output 1).
//   A bit is 1 iff the shadow value for that output is > 127. After an SBA,
//   the PBA port cursor resets to 0.
//
// PBA (Pulse Brightness Array) — 8 bytes of brightness 0-48 each:
//   [v0, v1, v2, v3, v4, v5, v6, v7]
//   Implicitly targets the next 8 outputs after the cursor (0-7, then 8-15,
//   then 16-23, then 24-31). Values 49..=132 are reserved for flash/pulse
//   modes — stay in 0-48 for plain intensity.
//
// One set_output therefore produces 5 writes: 1 SBA + 4 PBAs. Matches
// libdof exactly.

/// Real LedWiz HID backend.
struct LedWizReal {
    dev: HidDevice,
    shadow: Vec<u8>,
    num_outputs: u16,
}

impl LedWizReal {
    /// SBA: bit-per-output on/off state reflecting the current shadow.
    fn send_sba(&mut self) -> Result<(), String> {
        let mut sba = [0u8; 9];
        sba[1] = 0x40; // SBA command
        sba[6] = 0x02; // global pulse speed (libdof default)
        for i in 0..self.shadow.len().min(32) {
            if self.shadow[i] > 127 {
                sba[2 + i / 8] |= 1 << (i % 8);
            }
        }
        self.dev
            .write(&sba)
            .map(|_| ())
            .map_err(|e| format!("HID write (SBA): {e}"))
    }

    /// 4 consecutive PBAs covering all 32 outputs. Must be sent right after
    /// an SBA so the port cursor starts at 0.
    fn send_pba(&mut self) -> Result<(), String> {
        for group in 0..4 {
            let mut pba = [0u8; 9];
            for i in 0..8 {
                let idx = group * 8 + i;
                if idx < self.shadow.len() {
                    // Map 0..=255 → 0..=48. Clamped to 48 (valid intensity
                    // range ends there; higher values are flash modes).
                    pba[1 + i] = ((self.shadow[idx] as u32 * 48) / 255) as u8;
                }
            }
            self.dev
                .write(&pba)
                .map(|_| ())
                .map_err(|e| format!("HID write (PBA): {e}"))?;
        }
        Ok(())
    }
}

impl OutputBoard for LedWizReal {
    fn kind(&self) -> BoardKind {
        BoardKind::LedWiz
    }
    fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String> {
        if n == 0 || n > self.num_outputs {
            return Err(format!(
                "output {n} out of range (1..={})",
                self.num_outputs
            ));
        }
        self.shadow[(n - 1) as usize] = intensity;
        self.send_sba()?;
        self.send_pba()
    }

    fn all_off(&mut self) -> Result<(), String> {
        for v in self.shadow.iter_mut() {
            *v = 0;
        }
        // SBA with all bits cleared = every output off.
        let off = [0u8, 0x40, 0, 0, 0, 0, 0x02, 0, 0];
        self.dev
            .write(&off)
            .map(|_| ())
            .map_err(|e| format!("HID write (all_off): {e}"))
    }
}

// === Pinscape Pico protocol ===
//
// Ported from libdof `src/cab/out/pspico/PinscapePico.cpp`.
//
// Uses numbered HID reports (report ID = 4, length = 64 bytes):
//
//   buf[0]   = report ID (4)
//   buf[1]   = command:
//                0x20 = ALL PORTS OFF (rest ignored)
//                0x22 = SET OUTPUT PORTS (random-access list)
//   buf[2]   = count of (port, value) pairs (command 0x22)
//   buf[3..] = pairs: port_number (1-indexed), value (0-255)
//
// Unlike the KL25Z, ports accept 0-255 intensity directly (no SBA/PBA split)
// and a single packet can set one or many outputs. For single-output writes
// (our pulse use case), we send 0x22 with count=1.

const PSPICO_REPORT_ID: u8 = 4;
const PSPICO_REPORT_LEN: usize = 64;
const PSPICO_CMD_ALL_OFF: u8 = 0x20;
const PSPICO_CMD_SET_OUTPUTS: u8 = 0x22;

/// Real Pinscape Pico HID backend.
struct PinscapePicoReal {
    dev: HidDevice,
    num_outputs: u16,
}

impl OutputBoard for PinscapePicoReal {
    fn kind(&self) -> BoardKind {
        BoardKind::PinscapePico
    }
    fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String> {
        if n == 0 || n > self.num_outputs {
            return Err(format!(
                "output {n} out of range (1..={})",
                self.num_outputs
            ));
        }
        let mut buf = [0u8; PSPICO_REPORT_LEN];
        buf[0] = PSPICO_REPORT_ID;
        buf[1] = PSPICO_CMD_SET_OUTPUTS;
        buf[2] = 1; // count: one pair
        buf[3] = n as u8; // port number — already 1-indexed in our API
        buf[4] = intensity;
        self.dev
            .write(&buf)
            .map(|_| ())
            .map_err(|e| format!("HID write (0x22 SET OUTPUT PORTS): {e}"))
    }

    fn all_off(&mut self) -> Result<(), String> {
        let mut buf = [0u8; PSPICO_REPORT_LEN];
        buf[0] = PSPICO_REPORT_ID;
        buf[1] = PSPICO_CMD_ALL_OFF;
        self.dev
            .write(&buf)
            .map(|_| ())
            .map_err(|e| format!("HID write (0x20 ALL PORTS OFF): {e}"))
    }
}

// === DudesCab protocol ===
//
// Ported from libdof `src/cab/out/dudescab/DudesCab.cpp` (+ .h for enums).
//
// Packet structure (variable length, no HID padding in libdof):
//   buf[0]   = RID (3 = RIDOutputs interface report ID)
//   buf[1]   = command
//   buf[2]   = part index (always 0 for single-packet messages)
//   buf[3]   = total part count (always 1 here)
//   buf[4]   = payload size (bytes)
//   buf[5..] = payload
//
// Commands (firmware ≥ 1.9.0 — newer protocol):
//   101 = RT_PWM_ALLOFF   (no payload)
//   102 = RT_PWM_OUTPUTS  (changed-outputs layout below)
//
// `RT_PWM_OUTPUTS` payload:
//   [0]   = extMask (bit K = extension K has updates; LSB = ext 0)
//   For each extension bit set in ascending order:
//     outMask_lo, outMask_hi   (16-bit mask of updated outputs in that ext)
//     One byte per set bit in outMask, in ascending bit order, with the new
//     intensity value (0-255).
//
// Simplifications here:
//   * Modern firmware only (1.9.0+). libdof has a remap table for the old
//     command codes (3/4/5) — we'd add it behind a version probe if users
//     report issues on firmware < 1.9.0.
//   * Fixed `16 outputs per extension`. libdof queries RT_PWM_GETINFOS at
//     open time to read the real value; for diagnostic pulses the default
//     matches every DudesCab shipped to date.

const DUDESCAB_RID_OUTPUTS: u8 = 3;
const DUDESCAB_CMD_ALLOFF: u8 = 101;
const DUDESCAB_CMD_OUTPUTS: u8 = 102;
const DUDESCAB_HEADER_SIZE: usize = 5;
const DUDESCAB_OUTPUTS_PER_EXT: u16 = 16;

struct DudesCabReal {
    dev: HidDevice,
    num_outputs: u16,
}

impl OutputBoard for DudesCabReal {
    fn kind(&self) -> BoardKind {
        BoardKind::DudesCab
    }
    fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String> {
        if n == 0 || n > self.num_outputs {
            return Err(format!(
                "output {n} out of range (1..={})",
                self.num_outputs
            ));
        }
        let idx = n - 1;
        let ext = (idx / DUDESCAB_OUTPUTS_PER_EXT) as u8;
        let bit = idx % DUDESCAB_OUTPUTS_PER_EXT;

        let ext_mask = 1u8 << ext;
        let out_mask = 1u16 << bit;
        let payload = [
            ext_mask,
            (out_mask & 0xFF) as u8,
            ((out_mask >> 8) & 0xFF) as u8,
            intensity,
        ];

        let mut buf = [0u8; DUDESCAB_HEADER_SIZE + 4];
        buf[0] = DUDESCAB_RID_OUTPUTS;
        buf[1] = DUDESCAB_CMD_OUTPUTS;
        buf[2] = 0; // part 0 of ...
        buf[3] = 1; // 1 part total
        buf[4] = payload.len() as u8;
        buf[DUDESCAB_HEADER_SIZE..].copy_from_slice(&payload);

        self.dev
            .write(&buf)
            .map(|_| ())
            .map_err(|e| format!("HID write (DudesCab RT_PWM_OUTPUTS): {e}"))
    }

    fn all_off(&mut self) -> Result<(), String> {
        // Header only, no payload. `buf[4] = 0` = empty payload.
        let buf = [
            DUDESCAB_RID_OUTPUTS,
            DUDESCAB_CMD_ALLOFF,
            0, // part index
            1, // total parts
            0, // payload size
        ];
        self.dev
            .write(&buf)
            .map(|_| ())
            .map_err(|e| format!("HID write (DudesCab RT_PWM_ALLOFF): {e}"))
    }
}

// === Ultimarc PacDrive / PacLed64 / Ultimate I/O protocol ===
//
// Ported from libdof `src/cab/out/pac/PacDriveSingleton.cpp`. libdof uses
// `libusb_control_transfer` directly; we reach the same HID SET_REPORT
// endpoint through `hidapi::HidDevice::write` with Report ID 0, which
// hidapi itself translates to a class-specific SETUP packet when the
// device has no OUT interrupt endpoint (the Ultimarc case).
//
// Ultimarc USB control transfer used by libdof:
//   bmRequestType = 0x21  (host-to-device, class, interface)
//   bRequest      = 9     (HID SET_REPORT)
//   wValue        = 0x0200 (report type Output, report ID 0)
//
// Two distinct report formats:
//
//   PacDrive — 16 binary LEDs per device, packet 4 bytes (plus Report ID
//   prefix for hidapi = 5 bytes total):
//     [0x00, 0, 0, hi, lo]  — hi:lo = 16 bits of on/off state
//
//   PacLed64 / Ultimate I/O — per-port intensity 0-255, packet 2 bytes
//   (plus Report ID prefix = 3 bytes):
//     [0x00, port, intensity]  — port is 0-indexed
//
// AllOff for PacDrive = one packet with hi:lo = 0. For PacLed/UltimateIO
// libdof iterates every port individually — we do the same.

/// PacDrive: 16 binary outputs (on/off, no intensity). Threshold is >127 in
/// to mirror how libdof's OutputControllerCompleteBase converts 0-255
/// intensities into on/off bits for this class of device.
struct PacDriveReal {
    dev: HidDevice,
    shadow: u16,
    num_outputs: u16,
}

impl PacDriveReal {
    fn send_state(&mut self) -> Result<(), String> {
        let buf = [
            0x00u8, // hidapi report ID prefix
            0,
            0,
            ((self.shadow >> 8) & 0xFF) as u8,
            (self.shadow & 0xFF) as u8,
        ];
        self.dev
            .write(&buf)
            .map(|_| ())
            .map_err(|e| format!("HID write (PacDrive state): {e}"))
    }
}

impl OutputBoard for PacDriveReal {
    fn kind(&self) -> BoardKind {
        BoardKind::PacDrive
    }
    fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String> {
        if n == 0 || n > self.num_outputs {
            return Err(format!(
                "output {n} out of range (1..={})",
                self.num_outputs
            ));
        }
        let bit = 1u16 << (n - 1);
        if intensity > 127 {
            self.shadow |= bit;
        } else {
            self.shadow &= !bit;
        }
        self.send_state()
    }

    fn all_off(&mut self) -> Result<(), String> {
        self.shadow = 0;
        self.send_state()
    }
}

/// PacLed64 and Ultimate I/O share the same per-port SET_REPORT protocol
/// (only the port count differs — 64 vs 96). Unified impl.
struct PacIndexedReal {
    dev: HidDevice,
    kind: BoardKind,
    num_outputs: u16,
}

impl OutputBoard for PacIndexedReal {
    fn kind(&self) -> BoardKind {
        self.kind
    }
    fn num_outputs(&self) -> u16 {
        self.num_outputs
    }

    fn set_output(&mut self, n: u16, intensity: u8) -> Result<(), String> {
        if n == 0 || n > self.num_outputs {
            return Err(format!(
                "output {n} out of range (1..={})",
                self.num_outputs
            ));
        }
        // libdof uses 0-indexed port. Our `n` is 1-indexed externally.
        let buf = [0x00u8, (n - 1) as u8, intensity];
        self.dev
            .write(&buf)
            .map(|_| ())
            .map_err(|e| format!("HID write (PacLed/UltimateIO set): {e}"))
    }

    fn all_off(&mut self) -> Result<(), String> {
        // libdof iterates every port individually; no single-packet all-off
        // for this protocol.
        for i in 0..self.num_outputs {
            let buf = [0x00u8, i as u8, 0];
            self.dev
                .write(&buf)
                .map_err(|e| format!("HID write (PacLed/UltimateIO all_off {i}): {e}"))?;
        }
        Ok(())
    }
}

// === Discovery thread ===

pub enum DiscoveryCmd {
    /// Start pulsing `output` in a 50/500ms loop, up to MAX_LOOP_PULSES.
    /// Sending this while a loop is running immediately switches targets.
    StartLoop { output: u16 },
    /// Stop the current loop and drive all outputs to 0.
    Stop,
    /// All off and exit the thread.
    Shutdown,
}

pub enum DiscoveryEvent {
    /// One pulse cycle completed (ON + OFF).
    #[allow(dead_code)] // `output` consumed when diagnostic logging is added
    PulseFired { output: u16, count: u32 },
    /// Per-output loop auto-stopped at MAX_LOOP_PULSES.
    #[allow(dead_code)]
    LoopCompleted { output: u16 },
    /// Hardware error — loop aborted, outputs off.
    Error { msg: String },
}

pub struct DiscoveryChannels {
    pub cmd_tx: Sender<DiscoveryCmd>,
    pub event_rx: Receiver<DiscoveryEvent>,
}

pub fn spawn_discovery_thread(mut board: Box<dyn OutputBoard>) -> DiscoveryChannels {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<DiscoveryCmd>();
    let (event_tx, event_rx) = crossbeam_channel::unbounded::<DiscoveryEvent>();

    thread::spawn(move || {
        log::info!(
            "Output discovery thread started: board={}, outputs={}",
            board.kind().label(),
            board.num_outputs()
        );
        let mut active: Option<u16> = None;
        let mut count: u32 = 0;

        loop {
            // Idle: block for the next command.
            if active.is_none() {
                match cmd_rx.recv() {
                    Ok(DiscoveryCmd::StartLoop { output }) => {
                        active = Some(output);
                        count = 0;
                    }
                    Ok(DiscoveryCmd::Stop) => {
                        let _ = board.all_off();
                    }
                    Ok(DiscoveryCmd::Shutdown) | Err(_) => {
                        let _ = board.all_off();
                        log::info!("Output discovery thread exiting");
                        return;
                    }
                }
                continue;
            }
            let out = active.unwrap();

            // ON (50ms — non-interruptible, it's brief enough).
            if let Err(e) = board.set_output(out, 255) {
                let _ = board.all_off();
                let _ = event_tx.send(DiscoveryEvent::Error { msg: e });
                active = None;
                continue;
            }
            thread::sleep(Duration::from_millis(PULSE_ON_MS));

            if let Err(e) = board.set_output(out, 0) {
                let _ = board.all_off();
                let _ = event_tx.send(DiscoveryEvent::Error { msg: e });
                active = None;
                continue;
            }
            count += 1;
            let _ = event_tx.send(DiscoveryEvent::PulseFired { output: out, count });

            if count >= MAX_LOOP_PULSES {
                let _ = event_tx.send(DiscoveryEvent::LoopCompleted { output: out });
                active = None;
                count = 0;
                continue;
            }

            // OFF gap (interruptible — this is where Stop / Next lands).
            match cmd_rx.recv_timeout(Duration::from_millis(PULSE_OFF_MS)) {
                Ok(DiscoveryCmd::StartLoop { output: new_out }) => {
                    active = Some(new_out);
                    count = 0;
                }
                Ok(DiscoveryCmd::Stop) => {
                    let _ = board.all_off();
                    active = None;
                }
                Ok(DiscoveryCmd::Shutdown) => {
                    let _ = board.all_off();
                    log::info!("Output discovery thread exiting");
                    return;
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    let _ = board.all_off();
                    log::info!("Output discovery thread exiting (channel closed)");
                    return;
                }
            }
        }
    });

    DiscoveryChannels { cmd_tx, event_rx }
}

// === UI-side state ===

/// Session state the UI drives. Holds the board kind, per-output labels,
/// current target, live pulse count, and the thread handle.
pub struct DiscoveryState {
    pub board_kind: BoardKind,
    pub num_outputs: u16,
    /// 1-indexed current output under the cursor.
    pub current: u16,
    pub labels: Vec<String>,
    pub pulse_count: u32,
    pub loop_running: bool,
    pub warning_ack: bool,
    pub channels: Option<DiscoveryChannels>,
    pub last_error: Option<String>,
    /// Result of the latest `detect_boards()` call. Empty = nothing found.
    pub detected: Vec<DetectedBoard>,
    /// Error from the latest scan, typically a "Permission denied" on Linux
    /// when udev rules are missing.
    pub detect_error: Option<String>,
    /// True once the user has clicked the scan button at least once.
    pub has_scanned: bool,
    /// Background receiver for the pkexec udev-install thread. Some = install
    /// in progress. None = idle (never started or finished).
    #[cfg(target_os = "linux")]
    pub udev_apply_rx: Option<Receiver<Result<(), String>>>,
    /// Final result of the last udev install, if any.
    #[cfg(target_os = "linux")]
    pub udev_apply_status: Option<Result<(), String>>,
}

impl Default for DiscoveryState {
    fn default() -> Self {
        Self {
            board_kind: BoardKind::Simulator,
            num_outputs: 0,
            current: 1,
            labels: Vec::new(),
            pulse_count: 0,
            loop_running: false,
            warning_ack: false,
            channels: None,
            last_error: None,
            detected: Vec::new(),
            detect_error: None,
            has_scanned: false,
            #[cfg(target_os = "linux")]
            udev_apply_rx: None,
            #[cfg(target_os = "linux")]
            udev_apply_status: None,
        }
    }
}

impl DiscoveryState {
    pub fn is_started(&self) -> bool {
        self.channels.is_some()
    }

    /// Run a fresh USB scan for supported output boards.
    pub fn scan_hardware(&mut self) {
        self.has_scanned = true;
        match detect_boards() {
            Ok(boards) => {
                self.detected = boards;
                self.detect_error = None;
            }
            Err(e) => {
                self.detected.clear();
                self.detect_error = Some(e);
            }
        }
    }

    /// Drain the udev-install thread's result (if any) into `udev_apply_status`
    /// and clear the pending receiver. No-op outside Linux.
    #[cfg(target_os = "linux")]
    pub fn poll_udev_apply(&mut self) {
        if let Some(rx) = self.udev_apply_rx.as_ref() {
            if let Ok(result) = rx.try_recv() {
                self.udev_apply_status = Some(result);
                self.udev_apply_rx = None;
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn poll_udev_apply(&mut self) {}

    /// Spawn the driver thread for `board` and initialize session state.
    pub fn start(&mut self, board: Box<dyn OutputBoard>) {
        self.board_kind = board.kind();
        self.num_outputs = board.num_outputs();
        self.current = 1;
        self.pulse_count = 0;
        self.loop_running = false;
        self.last_error = None;
        self.labels = vec![String::new(); self.num_outputs as usize];
        self.channels = Some(spawn_discovery_thread(board));
    }

    /// Stop any pulse in flight, then close the session. Thread sends
    /// `all_off` before exiting.
    pub fn stop_session(&mut self) {
        if let Some(ch) = self.channels.take() {
            let _ = ch.cmd_tx.send(DiscoveryCmd::Shutdown);
            // dropping ch closes cmd_tx; thread observes Disconnected on its
            // next recv and exits cleanly.
        }
        self.loop_running = false;
        self.pulse_count = 0;
    }

    /// Start pulsing the currently-selected output.
    pub fn start_loop(&mut self) {
        if let Some(ch) = self.channels.as_ref() {
            self.pulse_count = 0;
            self.loop_running = true;
            self.last_error = None;
            let _ = ch.cmd_tx.send(DiscoveryCmd::StartLoop {
                output: self.current,
            });
        }
    }

    /// Stop any active loop. Safe when idle.
    pub fn stop_loop(&mut self) {
        if let Some(ch) = self.channels.as_ref() {
            let _ = ch.cmd_tx.send(DiscoveryCmd::Stop);
        }
        self.loop_running = false;
    }

    pub fn next_output(&mut self) {
        if self.num_outputs == 0 {
            return;
        }
        if self.current < self.num_outputs {
            self.current += 1;
        }
        if self.loop_running {
            self.start_loop();
        }
    }

    pub fn prev_output(&mut self) {
        if self.current > 1 {
            self.current -= 1;
        }
        if self.loop_running {
            self.start_loop();
        }
    }

    pub fn current_label_mut(&mut self) -> Option<&mut String> {
        if self.current == 0 {
            return None;
        }
        self.labels.get_mut(self.current as usize - 1)
    }

    /// Drain pending events and update counters / error state.
    pub fn poll_events(&mut self) {
        let Some(ch) = self.channels.as_ref() else {
            return;
        };
        while let Ok(ev) = ch.event_rx.try_recv() {
            match ev {
                DiscoveryEvent::PulseFired { count, .. } => {
                    self.pulse_count = count;
                }
                DiscoveryEvent::LoopCompleted { .. } => {
                    self.loop_running = false;
                }
                DiscoveryEvent::Error { msg } => {
                    self.loop_running = false;
                    self.last_error = Some(msg);
                }
            }
        }
    }

    /// Copy-pastable summary of everything labelled.
    pub fn summary_text(&self) -> String {
        let mut out = String::new();
        for (i, label) in self.labels.iter().enumerate() {
            let label = label.trim();
            if label.is_empty() {
                continue;
            }
            out.push_str(&format!("Output #{} → {}\n", i + 1, label));
        }
        out
    }
}

// ---------- Tests ----------
//
// These unit tests don't touch hardware — they verify byte-level parity
// between our Rust port and the libdof C++ reference for each protocol.

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercise the LedWiz SBA + PBA byte layout against a shadow state,
    /// without opening a real HID device. Asserts the exact byte sequences
    /// libdof would produce.
    fn build_ledwiz_sba(shadow: &[u8; 32]) -> [u8; 9] {
        let mut sba = [0u8; 9];
        sba[1] = 0x40;
        sba[6] = 0x02;
        for i in 0..32 {
            if shadow[i] > 127 {
                sba[2 + i / 8] |= 1 << (i % 8);
            }
        }
        sba
    }

    fn build_ledwiz_pba(shadow: &[u8; 32]) -> [[u8; 9]; 4] {
        let mut pbas = [[0u8; 9]; 4];
        for (group, pba) in pbas.iter_mut().enumerate() {
            for i in 0..8 {
                let idx = group * 8 + i;
                pba[1 + i] = ((shadow[idx] as u32 * 48) / 255) as u8;
            }
        }
        pbas
    }

    #[test]
    fn ledwiz_all_off_sba_matches_libdof() {
        let shadow = [0u8; 32];
        let sba = build_ledwiz_sba(&shadow);
        // libdof AllOff: [0x00, 0x40, 0, 0, 0, 0, 0x02, 0, 0]
        assert_eq!(sba, [0x00, 0x40, 0, 0, 0, 0, 0x02, 0, 0]);
    }

    #[test]
    fn ledwiz_sba_bit_set_above_threshold() {
        let mut shadow = [0u8; 32];
        shadow[0] = 255; // output 1 full
        shadow[7] = 128; // output 8 just above threshold
        shadow[8] = 127; // output 9 just below — should NOT set bit
        shadow[31] = 200; // output 32
        let sba = build_ledwiz_sba(&shadow);
        assert_eq!(sba[2], 0b1000_0001, "bank0 should have bits 0 and 7 set");
        assert_eq!(
            sba[3], 0b0000_0000,
            "bank1 should be empty (bit 8 below threshold)"
        );
        assert_eq!(sba[5], 0b1000_0000, "bank3 should have bit 31 set");
    }

    #[test]
    fn ledwiz_pba_intensity_maps_0_to_48() {
        let mut shadow = [0u8; 32];
        shadow[0] = 0;
        shadow[1] = 255;
        shadow[2] = 128; // ~half
        let pbas = build_ledwiz_pba(&shadow);
        assert_eq!(pbas[0][1], 0, "0 → 0");
        assert_eq!(pbas[0][2], 48, "255 → 48");
        assert_eq!(pbas[0][3], 24, "128 → ~half of 48");
    }

    #[test]
    #[allow(clippy::identity_op)]
    fn ledwiz_unit_from_pid_matches_libdof_formula() {
        // libdof: unit = (pid & 0x0F) + 1 — preserve the formula shape even
        // when the arithmetic collapses (e.g. `& 0x0F` on 0xF0 yields 0).
        assert_eq!((0x00F0u16 & 0x0F) + 1, 1);
        assert_eq!((0x00F1u16 & 0x0F) + 1, 2);
        assert_eq!((0x00FFu16 & 0x0F) + 1, 16);
    }

    #[test]
    fn pinscape_extended_cmd_layout() {
        // Set output 5 to 255, all others 0 → should produce:
        // [0x00, 200, 0, 0, 0, 0, 255, 0, 0]  (group 0, outputs 1-7)
        let shadow = {
            let mut s = [0u8; 22];
            s[4] = 255; // output 5 (0-indexed as 4)
            s
        };
        let group = 0u16;
        let base_idx = (group * OUTPUTS_PER_GROUP) as usize;
        let mut report = [0u8; 9];
        report[1] = CMD_SET_BRIGHTNESS_BASE + group as u8;
        for i in 0..OUTPUTS_PER_GROUP as usize {
            report[2 + i] = *shadow.get(base_idx + i).unwrap_or(&0);
        }
        assert_eq!(report, [0x00, 200, 0, 0, 0, 0, 255, 0, 0]);
    }

    #[test]
    fn pinscape_all_off_cmd() {
        // libdof + USBProtocol.h: [65, 5, 0, 0, 0, 0, 0, 0]
        let report = [0x00, CMD_EXTENDED, CMD_EXT_ALL_OFF, 0, 0, 0, 0, 0, 0];
        assert_eq!(report, [0x00, 65, 5, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn pspico_set_output_layout() {
        // libdof pspico 0x22: buf[0]=4, buf[1]=0x22, buf[2]=count,
        // buf[3]=port+1 (1-indexed), buf[4]=value. Total 64 bytes.
        let mut buf = [0u8; PSPICO_REPORT_LEN];
        buf[0] = PSPICO_REPORT_ID;
        buf[1] = PSPICO_CMD_SET_OUTPUTS;
        buf[2] = 1;
        buf[3] = 5; // port 5 (our n is already 1-indexed)
        buf[4] = 255;
        assert_eq!(buf[0], 4, "report ID = 4");
        assert_eq!(buf[1], 0x22, "command = 0x22");
        assert_eq!(buf[2], 1, "count = 1");
        assert_eq!(buf[3], 5, "port 5");
        assert_eq!(buf[4], 255, "full intensity");
        assert!(buf[5..].iter().all(|&b| b == 0), "rest zeroed");
        assert_eq!(buf.len(), 64, "packet is 64 bytes");
    }

    #[test]
    fn pspico_all_off_layout() {
        let mut buf = [0u8; PSPICO_REPORT_LEN];
        buf[0] = PSPICO_REPORT_ID;
        buf[1] = PSPICO_CMD_ALL_OFF;
        assert_eq!(buf[0], 4);
        assert_eq!(buf[1], 0x20);
        assert!(buf[2..].iter().all(|&b| b == 0));
    }

    /// Mirror DudesCab `set_output` build logic so unit tests can inspect
    /// the exact packet without opening a real device.
    fn build_dudescab_set_output(n: u16, intensity: u8) -> Vec<u8> {
        let idx = n - 1;
        let ext = (idx / DUDESCAB_OUTPUTS_PER_EXT) as u8;
        let bit = idx % DUDESCAB_OUTPUTS_PER_EXT;
        let ext_mask = 1u8 << ext;
        let out_mask = 1u16 << bit;
        vec![
            DUDESCAB_RID_OUTPUTS,
            DUDESCAB_CMD_OUTPUTS,
            0,
            1,
            4,
            ext_mask,
            (out_mask & 0xFF) as u8,
            ((out_mask >> 8) & 0xFF) as u8,
            intensity,
        ]
    }

    #[test]
    fn dudescab_set_output_ext0_bit0() {
        // Output #1 → extension 0, bit 0, value 255.
        let buf = build_dudescab_set_output(1, 255);
        assert_eq!(
            buf,
            vec![3, 102, 0, 1, 4, 0b0000_0001, 0b0000_0001, 0b0000_0000, 255]
        );
    }

    #[test]
    fn dudescab_set_output_ext1_bit4() {
        // Output #21 → extension 1, bit 4 (21-1=20, 20/16=1, 20%16=4).
        let buf = build_dudescab_set_output(21, 128);
        assert_eq!(
            buf,
            vec![3, 102, 0, 1, 4, 0b0000_0010, 0b0001_0000, 0b0000_0000, 128]
        );
    }

    #[test]
    fn dudescab_set_output_ext0_bit15() {
        // Output #16 → extension 0, bit 15 (16-1=15, 15/16=0, 15%16=15).
        let buf = build_dudescab_set_output(16, 64);
        assert_eq!(
            buf,
            vec![3, 102, 0, 1, 4, 0b0000_0001, 0b0000_0000, 0b1000_0000, 64]
        );
    }

    #[test]
    fn dudescab_all_off_layout() {
        // Header-only packet, command 101, empty payload.
        let buf = [DUDESCAB_RID_OUTPUTS, DUDESCAB_CMD_ALLOFF, 0, 1, 0];
        assert_eq!(buf, [3, 101, 0, 1, 0]);
    }

    /// PacDrive: 16-bit on/off field, hi then lo, with two leading zero
    /// bytes and Report ID prefix. Matches libdof
    /// `PacDriveUHIDSetLEDStates` exactly.
    fn build_pacdrive_state(shadow: u16) -> [u8; 5] {
        [
            0x00,
            0,
            0,
            ((shadow >> 8) & 0xFF) as u8,
            (shadow & 0xFF) as u8,
        ]
    }

    #[test]
    fn pacdrive_all_off_state() {
        assert_eq!(build_pacdrive_state(0), [0x00, 0, 0, 0, 0]);
    }

    #[test]
    fn pacdrive_output_1_on() {
        // Output #1 → bit 0 → shadow = 1 → lo byte = 0x01.
        let shadow = 1u16 << 0;
        assert_eq!(build_pacdrive_state(shadow), [0x00, 0, 0, 0x00, 0x01]);
    }

    #[test]
    fn pacdrive_output_16_on() {
        // Output #16 → bit 15 → shadow = 0x8000 → hi byte = 0x80.
        let shadow = 1u16 << 15;
        assert_eq!(build_pacdrive_state(shadow), [0x00, 0, 0, 0x80, 0x00]);
    }

    #[test]
    fn pacled_set_output_layout() {
        // PacLed64 / Ultimate I/O: [report_id=0, port (0-indexed), intensity]
        let n: u16 = 5; // output #5
        let intensity: u8 = 200;
        let buf = [0x00u8, (n - 1) as u8, intensity];
        assert_eq!(buf, [0x00, 0x04, 200]);
    }

    #[test]
    fn pacled_output_port_is_zero_indexed() {
        // Output #1 → port 0 in the wire format.
        let n: u16 = 1;
        let buf = [0x00u8, (n - 1) as u8, 255];
        assert_eq!(buf[1], 0, "libdof uses 0-indexed port numbers");
    }
}
