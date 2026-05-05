// Cross-platform desktop integration: app-menu shortcuts (PinReady + VPinballX)
// and `.vpx` file association.
//
// Linux: freedesktop standard — works identically on GNOME, KDE Plasma, XFCE,
// Cinnamon, MATE, LXQt. Writes user-level files under ~/.local/share so no
// sudo is required and an uninstall reverses cleanly.
//
// macOS: writes a thin ~/Applications/PinReady.app bundle that launches the
// running binary, so PinReady appears in Launchpad/Spotlight. The shipped
// VPinballX is already a .app and registers `.vpx` itself via its
// CFBundleDocumentTypes — we don't duplicate that.
//
// Windows: writes Start Menu .lnk shortcuts (per-user) and HKCU registry
// entries that bind `.vpx` to VPinballX with our icon. All HKCU — no UAC.

use std::path::{Path, PathBuf};

const PINREADY_LOGO_PNG: &[u8] = include_bytes!("../../assets/vpinball_logo.png");
#[cfg(target_os = "windows")]
const PINREADY_ICON_ICO: &[u8] = include_bytes!("../../assets/icon.ico");

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns true if PinReady's desktop entry is currently installed.
/// Checked via the platform-specific marker file written by `install`.
pub(super) fn is_desktop_integration_installed() -> bool {
    pinready_marker_path().is_some_and(|p| p.exists())
}

/// Install or remove desktop shortcuts and `.vpx` association.
/// `vpx_exe_path` is the resolved VPinballX binary path (may be empty if the
/// user hasn't installed VPX yet — in that case only PinReady's own shortcut
/// is installed and the `.vpx` association is skipped).
pub(super) fn set_desktop_integration(enabled: bool, vpx_exe_path: &str) -> anyhow::Result<()> {
    if enabled {
        install(vpx_exe_path)
    } else {
        uninstall()
    }
}

// ---------------------------------------------------------------------------
// Platform-specific paths
// ---------------------------------------------------------------------------

fn home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// Marker file whose presence means "we installed desktop entries".
fn pinready_marker_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        home().map(|h| h.join(".local/share/applications/pinready.desktop"))
    }
    #[cfg(target_os = "macos")]
    {
        home().map(|h| h.join("Applications/PinReady.app/Contents/Info.plist"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(|appdata| {
            PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\PinReady.lnk")
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

// ---------------------------------------------------------------------------
// Linux (freedesktop — GNOME / KDE / XFCE / Cinnamon / MATE / LXQt)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn install(vpx_exe_path: &str) -> anyhow::Result<()> {
    let h = home().ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let exe = std::env::current_exe()?;

    let apps = h.join(".local/share/applications");
    let icons = h.join(".local/share/icons/hicolor/128x128");
    let mime_pkgs = h.join(".local/share/mime/packages");
    std::fs::create_dir_all(&apps)?;
    std::fs::create_dir_all(icons.join("apps"))?;
    std::fs::create_dir_all(icons.join("mimetypes"))?;
    std::fs::create_dir_all(&mime_pkgs)?;

    // Icons (reuse the bundled vpinball logo for both menu entries and the
    // .vpx mimetype — same artwork is fine and keeps the binary smaller).
    std::fs::write(icons.join("apps/pinready.png"), PINREADY_LOGO_PNG)?;
    std::fs::write(icons.join("apps/vpinballx.png"), PINREADY_LOGO_PNG)?;
    std::fs::write(
        icons.join("mimetypes/application-x-vpinball.png"),
        PINREADY_LOGO_PNG,
    )?;

    // PinReady .desktop — points to the currently-running binary.
    let exe_quoted = shell_quote(&exe.display().to_string());
    let pinready_desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=PinReady\n\
         GenericName=Visual Pinball Launcher\n\
         Comment=Visual Pinball configurator and table launcher\n\
         Exec={exe_quoted}\n\
         Icon=pinready\n\
         Terminal=false\n\
         Categories=Game;ArcadeGame;\n\
         StartupNotify=true\n\
         StartupWMClass=PinReady\n"
    );
    std::fs::write(apps.join("pinready.desktop"), pinready_desktop)?;

    // MIME type definition for .vpx — installed unconditionally so the file
    // manager learns the type (and thus the icon) even if VPX isn't yet
    // installed.
    let mime_xml = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n\
  <mime-type type=\"application/x-vpinball\">\n\
    <comment>Visual Pinball Table</comment>\n\
    <icon name=\"application-x-vpinball\"/>\n\
    <glob pattern=\"*.vpx\"/>\n\
  </mime-type>\n\
</mime-info>\n";
    std::fs::write(mime_pkgs.join("pinready-vpx.xml"), mime_xml)?;

    // VPinballX shortcut + .vpx association — only when we know where VPX is.
    let vpx_resolved = if !vpx_exe_path.is_empty() {
        let p = crate::updater::resolve_vpx_exe(Path::new(vpx_exe_path));
        p.is_file().then_some(p)
    } else {
        None
    };
    if let Some(vpx) = vpx_resolved {
        let vpx_quoted = shell_quote(&vpx.display().to_string());
        let vpx_desktop = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Visual Pinball X\n\
             GenericName=Pinball Simulator\n\
             Comment=Play Visual Pinball X tables\n\
             Exec={vpx_quoted} -play %f\n\
             Icon=vpinballx\n\
             Terminal=false\n\
             MimeType=application/x-vpinball;\n\
             Categories=Game;ArcadeGame;\n"
        );
        std::fs::write(apps.join("vpinballx.desktop"), vpx_desktop)?;
    } else {
        // Stale entry from a previous install where VPX was set — clean up.
        let _ = std::fs::remove_file(apps.join("vpinballx.desktop"));
    }

    // Refresh system caches. These are best-effort: on a vanilla desktop
    // install they all exist, but pincab images may strip some — log and
    // continue rather than fail the whole operation.
    run_quiet("update-mime-database", &[h.join(".local/share/mime")]);
    run_quiet("update-desktop-database", std::slice::from_ref(&apps));
    // gtk-update-icon-cache only works on a theme dir with index.theme;
    // hicolor user dir typically has none, and modern desktops watch the
    // dir directly. Skip.

    // Make VPinballX the default handler for .vpx (only if we wrote it).
    if apps.join("vpinballx.desktop").is_file() {
        run_quiet(
            "xdg-mime",
            &["default", "vpinballx.desktop", "application/x-vpinball"],
        );
    }

    log::info!("Desktop integration installed under {}", h.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall() -> anyhow::Result<()> {
    let h = home().ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let apps = h.join(".local/share/applications");
    let icons = h.join(".local/share/icons/hicolor/128x128");
    let mime_pkgs = h.join(".local/share/mime/packages");

    let _ = std::fs::remove_file(apps.join("pinready.desktop"));
    let _ = std::fs::remove_file(apps.join("vpinballx.desktop"));
    let _ = std::fs::remove_file(mime_pkgs.join("pinready-vpx.xml"));
    let _ = std::fs::remove_file(icons.join("apps/pinready.png"));
    let _ = std::fs::remove_file(icons.join("apps/vpinballx.png"));
    let _ = std::fs::remove_file(icons.join("mimetypes/application-x-vpinball.png"));

    run_quiet("update-mime-database", &[h.join(".local/share/mime")]);
    run_quiet("update-desktop-database", std::slice::from_ref(&apps));

    log::info!("Desktop integration removed");
    Ok(())
}

/// POSIX-shell quote: wrap in single quotes, escape embedded `'` as `'\''`.
/// Matches the convention used in autostart.rs's Exec= line.
#[cfg(target_os = "linux")]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_quiet<S: AsRef<std::ffi::OsStr>>(cmd: &str, args: &[S]) {
    match std::process::Command::new(cmd).args(args).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => log::warn!(
            "{cmd} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => log::warn!("{cmd} not available: {e}"),
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn install(_vpx_exe_path: &str) -> anyhow::Result<()> {
    let h = home().ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let exe = std::env::current_exe()?;

    // Build a thin bundle at ~/Applications/PinReady.app that re-launches the
    // running binary. Launchpad/Spotlight pick up new bundles automatically.
    let app = h.join("Applications/PinReady.app");
    let macos_dir = app.join("Contents/MacOS");
    let resources = app.join("Contents/Resources");
    std::fs::create_dir_all(&macos_dir)?;
    std::fs::create_dir_all(&resources)?;

    // Launcher script — execs the real binary so dock/relaunch keeps the
    // same PID lifecycle. Symlinking the binary works too but breaks if the
    // user moves the cargo build output; a tiny shim is more robust.
    let exe_str = exe.display().to_string();
    let launcher = format!("#!/bin/sh\nexec {:?} \"$@\"\n", exe_str);
    let launcher_path = macos_dir.join("PinReady");
    std::fs::write(&launcher_path, launcher)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&launcher_path, std::fs::Permissions::from_mode(0o755))?;

    // Icon: macOS wants .icns; we ship .png. Drop the PNG as a fallback —
    // Finder will use the generic icon if .icns is missing, but Spotlight
    // and Launchpad still display the bundle name.
    std::fs::write(resources.join("pinready.png"), PINREADY_LOGO_PNG)?;

    let plist = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>CFBundleDisplayName</key><string>PinReady</string>\n\
    <key>CFBundleExecutable</key><string>PinReady</string>\n\
    <key>CFBundleIdentifier</key><string>com.pinready.launcher</string>\n\
    <key>CFBundleName</key><string>PinReady</string>\n\
    <key>CFBundlePackageType</key><string>APPL</string>\n\
    <key>CFBundleShortVersionString</key><string>1.0</string>\n\
    <key>CFBundleIconFile</key><string>pinready.png</string>\n\
    <key>NSHighResolutionCapable</key><true/>\n\
</dict>\n\
</plist>\n";
    std::fs::write(app.join("Contents/Info.plist"), plist)?;

    // Re-register with Launch Services so Launchpad/Spotlight pick it up.
    run_quiet(
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
        &["-f", app.to_string_lossy().as_ref()],
    );

    // .vpx association on macOS is owned by the shipped VPinballX.app
    // (its Info.plist declares CFBundleDocumentTypes for `.vpx`). We don't
    // duplicate that here — duti would be needed to override the user's
    // default handler and isn't preinstalled.

    log::info!("Desktop integration installed at {}", app.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall() -> anyhow::Result<()> {
    let h = home().ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let app = h.join("Applications/PinReady.app");
    if app.exists() {
        std::fs::remove_dir_all(&app)?;
        log::info!("Removed {}", app.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn install(vpx_exe_path: &str) -> anyhow::Result<()> {
    let appdata = std::env::var("APPDATA").map_err(|_| anyhow::anyhow!("APPDATA not set"))?;
    let exe = std::env::current_exe()?;

    // Stash a real .ico under %APPDATA%\PinReady so .lnk + registry entries
    // can reference it by stable path. The exe itself doesn't embed an icon
    // unless built with windres.
    let pinready_dir = PathBuf::from(&appdata).join("PinReady");
    std::fs::create_dir_all(&pinready_dir)?;
    let icon_path = pinready_dir.join("pinready.ico");
    std::fs::write(&icon_path, PINREADY_ICON_ICO)?;

    let start_menu = PathBuf::from(&appdata).join(r"Microsoft\Windows\Start Menu\Programs");
    std::fs::create_dir_all(&start_menu)?;

    // PinReady shortcut.
    create_lnk(
        &start_menu.join("PinReady.lnk"),
        &exe,
        exe.parent().unwrap_or(&exe),
        &icon_path,
        "Visual Pinball configurator and table launcher",
    )?;

    // VPinballX shortcut + .vpx association — only when VPX path is known.
    let vpx_resolved = (!vpx_exe_path.is_empty())
        .then(|| crate::updater::resolve_vpx_exe(Path::new(vpx_exe_path)))
        .filter(|p| p.is_file());

    if let Some(vpx) = vpx_resolved {
        create_lnk(
            &start_menu.join("Visual Pinball X.lnk"),
            &vpx,
            vpx.parent().unwrap_or(&vpx),
            &icon_path,
            "Visual Pinball X — Pinball simulator",
        )?;

        // Per-user file association via HKCU\Software\Classes. No UAC needed.
        let vpx_str = vpx.display().to_string();
        let icon_str = icon_path.display().to_string();
        let ps = format!(
            r#"$ErrorActionPreference='Stop';
            New-Item -Path 'HKCU:\Software\Classes\.vpx' -Force | Out-Null;
            Set-ItemProperty -Path 'HKCU:\Software\Classes\.vpx' -Name '(Default)' -Value 'PinReady.VPX';
            New-Item -Path 'HKCU:\Software\Classes\PinReady.VPX' -Force | Out-Null;
            Set-ItemProperty -Path 'HKCU:\Software\Classes\PinReady.VPX' -Name '(Default)' -Value 'Visual Pinball Table';
            New-Item -Path 'HKCU:\Software\Classes\PinReady.VPX\DefaultIcon' -Force | Out-Null;
            Set-ItemProperty -Path 'HKCU:\Software\Classes\PinReady.VPX\DefaultIcon' -Name '(Default)' -Value '"{icon}"';
            New-Item -Path 'HKCU:\Software\Classes\PinReady.VPX\shell\open\command' -Force | Out-Null;
            Set-ItemProperty -Path 'HKCU:\Software\Classes\PinReady.VPX\shell\open\command' -Name '(Default)' -Value '"{vpx}" -play "%1"';
            "#,
            icon = icon_str.replace('\'', "''"),
            vpx = vpx_str.replace('\'', "''"),
        );
        run_powershell(&ps);
    } else {
        let _ = std::fs::remove_file(start_menu.join("Visual Pinball X.lnk"));
    }

    log::info!(
        "Desktop integration installed under {}",
        start_menu.display()
    );
    Ok(())
}

#[cfg(target_os = "windows")]
fn uninstall() -> anyhow::Result<()> {
    let appdata = std::env::var("APPDATA").map_err(|_| anyhow::anyhow!("APPDATA not set"))?;
    let start_menu = PathBuf::from(&appdata).join(r"Microsoft\Windows\Start Menu\Programs");
    let _ = std::fs::remove_file(start_menu.join("PinReady.lnk"));
    let _ = std::fs::remove_file(start_menu.join("Visual Pinball X.lnk"));

    let _ = std::fs::remove_file(PathBuf::from(&appdata).join(r"PinReady\pinready.ico"));

    let ps = r#"$ErrorActionPreference='SilentlyContinue';
        Remove-Item -Path 'HKCU:\Software\Classes\.vpx' -Force -Recurse;
        Remove-Item -Path 'HKCU:\Software\Classes\PinReady.VPX' -Force -Recurse;
        "#;
    run_powershell(ps);

    log::info!("Desktop integration removed");
    Ok(())
}

#[cfg(target_os = "windows")]
fn create_lnk(
    lnk_path: &Path,
    target: &Path,
    workdir: &Path,
    icon: &Path,
    description: &str,
) -> anyhow::Result<()> {
    // PowerShell single-quote escape — duplicate any embedded apostrophe.
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let ps = format!(
        r#"$ws = New-Object -ComObject WScript.Shell;
        $s = $ws.CreateShortcut('{lnk}');
        $s.TargetPath = '{target}';
        $s.WorkingDirectory = '{workdir}';
        $s.IconLocation = '{icon},0';
        $s.Description = '{desc}';
        $s.Save();
        "#,
        lnk = q(lnk_path),
        target = q(target),
        workdir = q(workdir),
        icon = q(icon),
        desc = description.replace('\'', "''"),
    );
    run_powershell(&ps);
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_powershell(script: &str) {
    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
    {
        Ok(out) if out.status.success() => {}
        Ok(out) => log::warn!(
            "powershell exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => log::warn!("powershell not available: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Other platforms (BSD, etc.) — no-op
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn install(_vpx_exe_path: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn uninstall() -> anyhow::Result<()> {
    Ok(())
}
