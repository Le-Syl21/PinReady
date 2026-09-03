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

/// Gated on `test` as well, so the one piece of the Windows path that is
/// pure string work can be checked on any machine.
#[cfg(any(target_os = "windows", test))]
/// `\\?\DISPLAY#SAM0F99#5&abc&0&UID0#{guid}` → the `Device Parameters`
/// key those two middle components name.
pub(crate) fn registry_path(device_id: &str) -> Option<String> {
    let mut parts = device_id.split('#');
    // Leading `\?\DISPLAY`, then hardware id, then instance id.
    let _prefix = parts.next()?;
    let hardware = parts.next()?;
    let instance = parts.next()?;
    Some(format!(
        "SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\{hardware}\\{instance}\\Device Parameters"
    ))
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

#[cfg(target_os = "windows")]
mod platform {
    use super::{DisplayEdid, MatchKey};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW,
        EDD_GET_DEVICE_INTERFACE_NAME, HDC, HMONITOR, MONITORINFOEXW,
    };
    use windows::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        RRF_RT_REG_BINARY,
    };

    /// Windows keeps no API for a panel's physical size — `GetDeviceCaps`
    /// returns whatever the display driver invents. It does keep the EDID
    /// itself, under the device's registry key, and that is what we read.
    ///
    /// The walk is: every `HMONITOR` on the desktop, its `\\.\DISPLAY<n>`
    /// adapter name, that adapter's attached monitor interface name
    /// (`\\?\DISPLAY#SAM0F99#5&abc&0&UID0#{guid}`), and from its two middle
    /// components the key
    /// `SYSTEM\CurrentControlSet\Enum\DISPLAY\SAM0F99\5&abc&0&UID0\Device
    /// Parameters`, value `EDID`.
    ///
    /// Keyed by `HMONITOR` because SDL hands us exactly that in
    /// `SDL_PROP_DISPLAY_WINDOWS_HMONITOR_POINTER` — an exact match, where
    /// every other platform-agnostic scheme was a guess.
    pub fn read_all() -> Vec<DisplayEdid> {
        let mut out: Vec<DisplayEdid> = Vec::new();
        // SAFETY: standard Win32 enumeration; `out` outlives the callback,
        // which only pushes through the pointer we hand it.
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(std::ptr::from_mut(&mut out) as isize),
            );
        }
        out
    }

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        // SAFETY: `data` is the &mut Vec `read_all` passed in, alive for the
        // whole enumeration.
        let out = unsafe { &mut *(data.0 as *mut Vec<DisplayEdid>) };
        if let Some(bytes) = unsafe { edid_for_monitor(monitor) } {
            out.push(DisplayEdid {
                key: MatchKey::Monitor(monitor.0 as usize),
                bytes,
            });
        }
        true.into()
    }

    unsafe fn edid_for_monitor(monitor: HMONITOR) -> Option<Vec<u8>> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>()).ok()?;
        // SAFETY: `info` is correctly sized per the contract above.
        if !unsafe { GetMonitorInfoW(monitor, std::ptr::from_mut(&mut info).cast()) }.as_bool() {
            return None;
        }

        // The monitor attached to this adapter, as a device interface name.
        let mut device = DISPLAY_DEVICEW {
            cb: u32::try_from(size_of::<DISPLAY_DEVICEW>()).ok()?,
            ..Default::default()
        };
        // SAFETY: adapter name is NUL-terminated inside the fixed array.
        let ok = unsafe {
            EnumDisplayDevicesW(
                PCWSTR(info.szDevice.as_ptr()),
                0,
                &mut device,
                EDD_GET_DEVICE_INTERFACE_NAME,
            )
        };
        if !ok.as_bool() {
            return None;
        }
        read_edid_value(&registry_path(&wide_to_string(&device.DeviceID))?)
    }

    fn read_edid_value(path: &str) -> Option<Vec<u8>> {
        let path_w = wide(path);
        let mut key = HKEY::default();
        // SAFETY: both wide strings are NUL-terminated and outlive the call.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(path_w.as_ptr()),
                Some(0),
                KEY_READ,
                &mut key,
            )
        };
        if opened != ERROR_SUCCESS {
            return None;
        }
        let name = wide("EDID");
        let mut len: u32 = 0;
        // First call sizes the buffer, second fills it.
        // SAFETY: null data pointer with a length out-param is the documented
        // sizing call.
        let sized = unsafe {
            RegGetValueW(
                key,
                None,
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_BINARY,
                None,
                None,
                Some(&mut len),
            )
        };
        let mut buf = vec![0u8; len as usize];
        let read = if sized == ERROR_SUCCESS && len > 0 {
            // SAFETY: `buf` is exactly `len` bytes, as the sizing call asked.
            unsafe {
                RegGetValueW(
                    key,
                    None,
                    PCWSTR(name.as_ptr()),
                    RRF_RT_REG_BINARY,
                    None,
                    Some(buf.as_mut_ptr().cast()),
                    Some(&mut len),
                )
            }
        } else {
            sized
        };
        // SAFETY: `key` came from RegOpenKeyExW and is not used again.
        unsafe {
            let _ = RegCloseKey(key);
        }
        (read == ERROR_SUCCESS && len > 0).then(|| {
            buf.truncate(len as usize);
            buf
        })
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_to_string(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    // Silence the unused import when the compiler cannot see the callback
    // signature requires it.
    const _: Option<HWND> = None;
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{DisplayEdid, MatchKey};
    use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_core_graphics::{
        CGDisplayModelNumber, CGDisplayVendorNumber, CGGetActiveDisplayList,
    };
    use objc2_io_kit::{
        kIOMainPortDefault, kIORegistryIterateRecursively, IOIteratorNext, IOObjectRelease,
        IORegistryEntryCreateCFProperty, IORegistryEntryCreateIterator,
    };

    /// Walk the IORegistry for entries carrying an `IODisplayEDID` blob — the
    /// same thing `ioreg -l -w0 | grep IODisplayEDID` prints — and tie each one
    /// to a CoreGraphics display through the vendor and product numbers both
    /// sides report.
    ///
    /// Vendor+product rather than geometry, because two identical panels side
    /// by side would otherwise be indistinguishable; identical *models* still
    /// are, and that is a limit of the EDID itself rather than of the match.
    pub fn read_all() -> Vec<DisplayEdid> {
        let panels = registry_panels();
        if panels.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for id in active_displays() {
            // SAFETY: `id` came from CGGetActiveDisplayList, so it is live.
            let (vendor, product) =
                unsafe { (CGDisplayVendorNumber(id), CGDisplayModelNumber(id)) };
            if let Some(bytes) = panels
                .iter()
                .find(|p| p.vendor == vendor && p.product == product)
                .map(|p| p.edid.clone())
            {
                out.push(DisplayEdid {
                    key: MatchKey::CoreGraphics(id),
                    bytes,
                });
            }
        }
        out
    }

    fn active_displays() -> Vec<u32> {
        let mut count: u32 = 0;
        // SAFETY: the null buffer form is the documented "how many?" call.
        if unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) } != 0 {
            return Vec::new();
        }
        let mut ids = vec![0u32; count as usize];
        // SAFETY: `ids` holds exactly the count the call above reported.
        if unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) } != 0 {
            return Vec::new();
        }
        ids.truncate(count as usize);
        ids
    }

    struct Panel {
        vendor: u32,
        product: u32,
        edid: Vec<u8>,
    }

    /// Every IORegistry entry that carries an EDID, with the numbers needed to
    /// match it back to a display.
    fn registry_panels() -> Vec<Panel> {
        let mut out = Vec::new();
        let mut iter = 0;
        let root = CFString::from_static_str("IOService");
        // SAFETY: recursive iteration from the service plane root; the
        // iterator is released below on every path.
        let ok = unsafe {
            IORegistryEntryCreateIterator(
                objc2_io_kit::IORegistryGetRootEntry(kIOMainPortDefault),
                root.to_string().as_ptr().cast(),
                kIORegistryIterateRecursively,
                &mut iter,
            )
        };
        if ok != 0 {
            return out;
        }
        loop {
            // SAFETY: `iter` is a live iterator until it returns 0.
            let entry = unsafe { IOIteratorNext(iter) };
            if entry == 0 {
                break;
            }
            if let (Some(edid), Some(vendor), Some(product)) = (
                data_property(entry, "IODisplayEDID"),
                number_property(entry, "DisplayVendorID"),
                number_property(entry, "DisplayProductID"),
            ) {
                out.push(Panel {
                    vendor,
                    product,
                    edid,
                });
            }
            // SAFETY: each entry from IOIteratorNext is owned by us.
            unsafe { IOObjectRelease(entry) };
        }
        // SAFETY: the iterator itself is owned by us.
        unsafe { IOObjectRelease(iter) };
        out
    }

    fn property(entry: u32, key: &str) -> Option<CFRetained<CFType>> {
        let name = CFString::from_str(key);
        // SAFETY: `entry` is live for the duration; the returned value is
        // +1 retained, which CFRetained takes ownership of.
        unsafe { IORegistryEntryCreateCFProperty(entry, Some(&name), None, 0) }
    }

    fn data_property(entry: u32, key: &str) -> Option<Vec<u8>> {
        let value = property(entry, key)?;
        let data = value.downcast_ref::<CFData>()?;
        Some(data.to_vec())
    }

    fn number_property(entry: u32, key: &str) -> Option<u32> {
        let value = property(entry, key)?;
        let number = value.downcast_ref::<CFNumber>()?;
        number.as_i32().and_then(|n| u32::try_from(n).ok())
    }

    // `CFDictionary` is referenced only through the property calls above.
    const _: Option<&CFDictionary> = None;
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod platform {
    use super::DisplayEdid;

    pub fn read_all() -> Vec<DisplayEdid> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    /// The registry path is pure string surgery on a device interface
    /// name, so it is the one part of this that can be tested anywhere.
    #[test]
    fn a_device_interface_name_becomes_its_device_parameters_key() {
        let got = super::registry_path(
            r"\\?\DISPLAY#SAM0F99#5&1234abcd&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}",
        );
        assert_eq!(
            got.as_deref(),
            Some(
                "SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\SAM0F99\\5&1234abcd&0&UID4353\\Device Parameters"
            )
        );
        assert!(super::registry_path("nonsense").is_none());
    }
}
