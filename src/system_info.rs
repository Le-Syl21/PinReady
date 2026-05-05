//! Cross-platform "what am I running on" probe.
//!
//! Used in two places:
//! - The wizard's first page header, so a user filing a bug can take a
//!   screenshot that already carries the system context.
//! - The VPX-error popup's call header, so when a table crashes we have
//!   distro / display server / desktop pinned to the report.
//!
//! All probing is best-effort. We never fail or panic; missing data
//! degrades to "?" or just gets dropped from the summary.

#[derive(Debug, Default, Clone)]
pub struct SystemInfo {
    /// Family name: "Linux", "macOS", "Windows", "BSD", ...
    pub os: String,
    /// Distro / version string, ideally human-readable. Examples:
    /// `Ubuntu 24.04.1 LTS`, `Fedora Linux 40 (Workstation Edition)`,
    /// `macOS 14.5 (23F79)`, `Windows 11 Pro 23H2 (build 22631.4317)`.
    pub version: String,
    /// `Wayland` / `X11` / `Quartz` / `DWM` / "" if unknown.
    pub display_server: String,
    /// Desktop / WM name. "GNOME (Mutter)", "KDE Plasma (KWin)",
    /// "Hyprland", "Sway", "XFCE (Xfwm4)", "Aqua", "Explorer".
    pub desktop: String,
}

impl SystemInfo {
    /// One-line summary suitable for a UI label or a log line.
    /// Example: `Ubuntu 24.04 LTS · Wayland · GNOME (Mutter)`.
    pub fn one_liner(&self) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(4);
        let header = if self.version.is_empty() {
            self.os.as_str()
        } else {
            self.version.as_str()
        };
        if !header.is_empty() {
            parts.push(header);
        }
        if !self.display_server.is_empty() {
            parts.push(self.display_server.as_str());
        }
        if !self.desktop.is_empty() {
            parts.push(self.desktop.as_str());
        }
        if parts.is_empty() {
            "unknown".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// Detect the running system. Cheap (a few file reads / env lookups);
/// safe to call from anywhere. Cached at first call.
pub fn detect() -> SystemInfo {
    use std::sync::OnceLock;
    static CACHE: OnceLock<SystemInfo> = OnceLock::new();
    CACHE.get_or_init(detect_uncached).clone()
}

fn detect_uncached() -> SystemInfo {
    #[cfg(target_os = "linux")]
    {
        detect_linux()
    }
    #[cfg(target_os = "macos")]
    {
        detect_macos()
    }
    #[cfg(target_os = "windows")]
    {
        detect_windows()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        SystemInfo {
            os: std::env::consts::OS.to_string(),
            ..SystemInfo::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn detect_linux() -> SystemInfo {
    SystemInfo {
        os: "Linux".into(),
        version: read_os_release_pretty().unwrap_or_else(|| "Linux".into()),
        display_server: linux_display_server(),
        desktop: linux_desktop(),
    }
}

#[cfg(target_os = "linux")]
fn read_os_release_pretty() -> Option<String> {
    // /etc/os-release is the systemd / freedesktop standard. NixOS,
    // Alpine, Debian, Ubuntu, Fedora, Arch, openSUSE, all ship it.
    let content = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn linux_display_server() -> String {
    // XDG_SESSION_TYPE is the most reliable signal (set by logind). Env
    // vars `WAYLAND_DISPLAY` / `DISPLAY` are fallbacks — the latter is
    // also set under XWayland, so prefer XDG_SESSION_TYPE.
    if let Ok(t) = std::env::var("XDG_SESSION_TYPE") {
        match t.as_str() {
            "wayland" => return "Wayland".into(),
            "x11" => return "X11".into(),
            "tty" => return "TTY".into(),
            _ => {}
        }
    }
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        "Wayland".into()
    } else if std::env::var("DISPLAY").is_ok() {
        "X11".into()
    } else {
        String::new()
    }
}

#[cfg(target_os = "linux")]
fn linux_desktop() -> String {
    // XDG_CURRENT_DESKTOP is colon-separated and ordered most-specific
    // first ("GNOME-Classic:GNOME"). Take the first token.
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("XDG_SESSION_DESKTOP"))
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .ok()
        .and_then(|s| s.split(':').next().map(str::to_string))
        .unwrap_or_default();
    if desktop.is_empty() {
        return String::new();
    }
    // Normalize the spelling and append the underlying WM where it's a
    // 1:1 mapping the user expects to see.
    let upper = desktop.to_ascii_uppercase();
    match upper.as_str() {
        "GNOME" | "GNOME-CLASSIC" | "UBUNTU" | "UBUNTU:GNOME" => "GNOME (Mutter)".into(),
        "KDE" | "PLASMA" => "KDE Plasma (KWin)".into(),
        "XFCE" => "XFCE (Xfwm4)".into(),
        "X-CINNAMON" | "CINNAMON" => "Cinnamon (Muffin)".into(),
        "MATE" => "MATE (Marco)".into(),
        "LXQT" => "LXQt (Openbox)".into(),
        "LXDE" => "LXDE (Openbox)".into(),
        "PANTHEON" => "Pantheon (Gala)".into(),
        "BUDGIE" | "BUDGIE:GNOME" => "Budgie".into(),
        "DEEPIN" => "Deepin".into(),
        "SWAY" => "Sway".into(),
        "HYPRLAND" => "Hyprland".into(),
        "WAYFIRE" => "Wayfire".into(),
        "I3" => "i3".into(),
        "BSPWM" => "bspwm".into(),
        "AWESOME" => "awesome".into(),
        _ => desktop, // unknown — surface the raw value
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn detect_macos() -> SystemInfo {
    let version = read_macos_version().unwrap_or_else(|| "macOS".into());
    SystemInfo {
        os: "macOS".into(),
        version,
        display_server: "Quartz".into(),
        desktop: "Aqua".into(),
    }
}

/// Read `/System/Library/CoreServices/SystemVersion.plist` directly to
/// avoid shelling out to `sw_vers`. The plist is XML; we grep the two
/// keys we care about with hand-rolled string ops to avoid pulling a
/// plist crate just for this.
#[cfg(target_os = "macos")]
fn read_macos_version() -> Option<String> {
    let path = "/System/Library/CoreServices/SystemVersion.plist";
    let xml = std::fs::read_to_string(path).ok()?;
    let value_after = |key: &str| -> Option<String> {
        let needle = format!("<key>{key}</key>");
        let idx = xml.find(&needle)? + needle.len();
        let rest = &xml[idx..];
        let open = rest.find("<string>")? + "<string>".len();
        let close = rest[open..].find("</string>")?;
        Some(rest[open..open + close].trim().to_string())
    };
    let product = value_after("ProductName").unwrap_or_else(|| "macOS".into());
    let version = value_after("ProductVersion").unwrap_or_default();
    let build = value_after("ProductBuildVersion").unwrap_or_default();
    let mut s = product;
    if !version.is_empty() {
        s.push(' ');
        s.push_str(&version);
    }
    if !build.is_empty() {
        s.push_str(" (");
        s.push_str(&build);
        s.push(')');
    }
    Some(s)
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn detect_windows() -> SystemInfo {
    SystemInfo {
        os: "Windows".into(),
        version: read_windows_version().unwrap_or_else(|| "Windows".into()),
        display_server: "DWM".into(),
        desktop: "Explorer".into(),
    }
}

/// Pull a friendly version string from the registry via a one-shot
/// PowerShell invocation. Avoids adding `winreg` as a dependency.
/// Format: `Windows 11 Pro 23H2 (build 22631.4317)`.
#[cfg(target_os = "windows")]
fn read_windows_version() -> Option<String> {
    let script = r#"$ErrorActionPreference='Stop';
        $k = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion';
        $name = $k.ProductName;
        $disp = $k.DisplayVersion;
        if (-not $disp) { $disp = $k.ReleaseId };
        $build = "$($k.CurrentBuild).$($k.UBR)";
        Write-Output "$name $disp (build $build)";
    "#;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
