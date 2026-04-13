// Cross-platform autostart: Linux (.desktop), macOS (LaunchAgent), Windows (Startup shortcut)

/// Path to the autostart entry for the current platform.
pub(super) fn autostart_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        dirs_path("HOME", ".config/autostart/pinready.desktop")
    }
    #[cfg(target_os = "macos")]
    {
        dirs_path("HOME", "Library/LaunchAgents/com.pinready.plist")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(|appdata| {
            std::path::PathBuf::from(appdata)
                .join(r"Microsoft\Windows\Start Menu\Programs\Startup\PinReady.lnk")
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn dirs_path(env_var: &str, suffix: &str) -> Option<std::path::PathBuf> {
    std::env::var(env_var)
        .ok()
        .map(|home| std::path::PathBuf::from(home).join(suffix))
}

/// Check if autostart is currently enabled.
pub(super) fn is_autostart_enabled() -> bool {
    autostart_path().is_some_and(|p| p.exists())
}

/// Enable or disable autostart.
pub(super) fn set_autostart(enabled: bool) -> anyhow::Result<()> {
    let path = autostart_path()
        .ok_or_else(|| anyhow::anyhow!("Autostart not supported on this platform"))?;

    if !enabled {
        if path.exists() {
            std::fs::remove_file(&path)?;
            log::info!("Autostart disabled: removed {}", path.display());
        }
        return Ok(());
    }

    // Get the path to our own executable
    let exe = std::env::current_exe()?;
    let exe_str = exe.display().to_string();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(target_os = "linux")]
    {
        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=PinReady\n\
             Comment=VPinballX configurator and launcher\n\
             Exec={exe_str}\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
        std::fs::write(&path, content)?;
    }

    #[cfg(target_os = "macos")]
    {
        let content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
                 <key>Label</key>\n\
                 <string>com.pinready</string>\n\
                 <key>ProgramArguments</key>\n\
                 <array>\n\
                     <string>{exe_str}</string>\n\
                 </array>\n\
                 <key>RunAtLoad</key>\n\
                 <true/>\n\
             </dict>\n\
             </plist>\n"
        );
        std::fs::write(&path, content)?;
    }

    #[cfg(target_os = "windows")]
    {
        // Create a .lnk shortcut via PowerShell
        let ps_cmd = format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $s = $ws.CreateShortcut('{}'); \
             $s.TargetPath = '{}'; \
             $s.WorkingDirectory = '{}'; \
             $s.Save()",
            path.display(),
            exe_str,
            exe.parent().unwrap_or(&exe).display(),
        );
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_cmd])
            .output()?;
    }

    log::info!("Autostart enabled: {}", path.display());
    Ok(())
}
