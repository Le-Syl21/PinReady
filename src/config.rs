use anyhow::{Context, Result};
use ini_preserve::Ini;
use std::path::{Path, PathBuf};

/// The settings folder of the VPX builds PinReady installs. VPX keeps its
/// settings per major.minor version (`<data dir>/VPinballX/10.9/`), and from
/// 10.9 on it starts by copying the previous version's folder.
pub const VPX_SETTINGS_VERSION: &str = "10.9";

/// The settings version of the VPX in use, from its tag (`v10.8.1-6823-…` →
/// `10.8`), set at start-up and after a VPX update.
static INSTALLED_VERSION: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// `v10.9.0-6988-556075741` (or `10.9.0`) → `10.9`.
pub fn settings_version_of_tag(tag: &str) -> Option<String> {
    let v = tag.trim().trim_start_matches('v');
    let mut parts = v.split(['.', '-']);
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some(format!("{major}.{minor}"))
}

fn vpx_base_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("VPinballX")
}

/// Tell PinReady which VPX it drives (its tag), so the settings go where that
/// VPX reads them. A newer settings folder is first filled from the older
/// one, the way VPX does it, so nothing is lost when VPX is updated.
pub fn set_installed_vpx(tag: &str) {
    let Some(version) = settings_version_of_tag(tag) else {
        return;
    };
    if let Ok(mut v) = INSTALLED_VERSION.write() {
        *v = Some(version.clone());
    }
    migrate_settings(&vpx_base_dir(), &version);
}

/// The VPX settings folder in use: the installed VPX's version when known,
/// else the newest of 10.9 and 10.8 that exists (a VPX installed by hand),
/// else 10.9.
pub fn vpx_pref_dir() -> PathBuf {
    let base = vpx_base_dir();
    let known = INSTALLED_VERSION.read().ok().and_then(|v| v.clone());
    let version = known.unwrap_or_else(|| {
        if !base.join(VPX_SETTINGS_VERSION).is_dir() && base.join("10.8").is_dir() {
            "10.8".to_string()
        } else {
            VPX_SETTINGS_VERSION.to_string()
        }
    });
    base.join(version)
}

/// Default VPinballX.ini location, following OS conventions:
/// - Linux:   ~/.local/share/VPinballX/<version>/VPinballX.ini
/// - macOS:   ~/Library/Application Support/VPinballX/<version>/VPinballX.ini
/// - Windows: %APPDATA%\VPinballX\<version>\VPinballX.ini
pub fn default_ini_path() -> PathBuf {
    vpx_pref_dir().join("VPinballX.ini")
}

/// When `base/<version>` does not exist yet, create it from the newest older
/// version folder (10.8, 10.7, …), copying everything (ini, DOF config, …),
/// like VPX's own `FileLocator` does on its first start. Returns the folder
/// copied from.
pub fn migrate_settings(base: &Path, version: &str) -> Option<PathBuf> {
    let target = base.join(version);
    if target.exists() {
        return None;
    }
    let (major, minor) = version.split_once('.')?;
    let (major, minor): (u32, u32) = (major.parse().ok()?, minor.parse().ok()?);
    let source = (0..minor)
        .rev()
        .map(|m| base.join(format!("{major}.{m}")))
        .find(|p| p.is_dir())?;
    if let Err(e) = copy_dir(&source, &target) {
        log::warn!(
            "VPX settings: cannot copy {} to {}: {e}",
            source.display(),
            target.display()
        );
        return None;
    }
    log::info!(
        "VPX settings: {} created from {}",
        target.display(),
        source.display()
    );
    Some(source)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

pub struct VpxConfig {
    path: PathBuf,
    ini: Ini,
}

impl VpxConfig {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = path.map(PathBuf::from).unwrap_or_else(default_ini_path);

        let ini = if path.exists() {
            Ini::load(&path).map_err(|e| anyhow::anyhow!("{e}"))?
        } else {
            Ini::new()
        };

        Ok(Self { path, ini })
    }

    pub fn save(&self) -> Result<()> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }
        // If self.path is a symlink, resolve to the real target so that
        // ini-preserve's atomic rename doesn't clobber the symlink itself.
        let write_path = if self.path.is_symlink() {
            let target = std::fs::read_link(&self.path).with_context(|| {
                format!("Failed to read symlink target: {}", self.path.display())
            })?;
            if target.is_absolute() {
                target
            } else {
                self.path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join(target)
            }
        } else {
            self.path.clone()
        };

        let content = self.ini.to_string();
        let line_count = content.split('\n').count();
        log::info!(
            "Saving ini: {} bytes, {} lines to {}{}",
            content.len(),
            line_count,
            self.path.display(),
            if self.path.is_symlink() { "@" } else { "" }
        );
        self.ini
            .save(&write_path)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Path of the ini file this config reads from / writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        self.ini.get(section, key).map(|s| s.to_string())
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        self.ini.set(section, key, value);
    }

    pub fn get_i32(&self, section: &str, key: &str) -> Option<i32> {
        self.get(section, key)?.parse().ok()
    }

    pub fn get_f32(&self, section: &str, key: &str) -> Option<f32> {
        self.get(section, key)?.parse().ok()
    }

    pub fn set_i32(&mut self, section: &str, key: &str, value: i32) {
        self.set(section, key, &value.to_string());
    }

    pub fn set_f32(&mut self, section: &str, key: &str, value: f32) {
        self.set(section, key, &format!("{value}"));
    }

    // --- Screen configuration ---

    /// Generic display configuration. `section` and `prefix` vary per role:
    /// Playfield: ("Player", "Playfield"), Backglass: ("Backglass", "Backglass"),
    /// DMD: ("ScoreView", "ScoreView"), Topper: ("Topper", "Topper").
    pub fn set_display(
        &mut self,
        section: &str,
        prefix: &str,
        name: &str,
        w: i32,
        h: i32,
        enable_output: bool,
    ) {
        if enable_output {
            self.set_i32(section, &format!("{prefix}Output"), 1);
        }
        self.set(section, &format!("{prefix}Display"), name);
        // Borderless fullscreen on the assigned display. VPX's default is
        // 0 = Windowed, which scatters the windows wherever the compositor
        // puts them — assigning a whole monitor per role is the entire point
        // of the wizard, so pin 1 = Borderless Fullscreen explicitly.
        self.set_i32(section, &format!("{prefix}FullScreen"), 1);
        self.set(section, &format!("{prefix}WndX"), "");
        self.set(section, &format!("{prefix}WndY"), "");
        self.set_i32(section, &format!("{prefix}Width"), w);
        self.set_i32(section, &format!("{prefix}Height"), h);
    }

    pub fn set_view_mode(&mut self, mode: i32) {
        self.set_i32("Player", "BGSet", mode);
    }

    /// The `[TableOverride]` key holding the projection mode of a given view
    /// set. VPX keeps one per set and picks it from `BGSet`
    /// (`ViewSetup::ApplyTableOverrideSettings`), so writing the wrong one is
    /// a silent no-op.
    fn projection_mode_key(view_mode: i32) -> &'static str {
        match view_mode {
            0 => "ViewDTMode",
            2 => "ViewFSSMode",
            _ => "ViewCabMode",
        }
    }

    /// Pin the "Window" projection (`VLM_WINDOW` = 2) on the current view set.
    ///
    /// Head tracking feeds VPX an eye position, and only this projection reads
    /// it: Legacy and Camera build the frustum from a fixed camera, so the
    /// plugin would run and change nothing on screen. Writing the key matters
    /// even though Window is VPX's own default for the cabinet set — when the
    /// key is absent each table's saved mode wins instead.
    pub fn set_window_projection(&mut self, view_mode: i32) {
        self.set_i32("TableOverride", Self::projection_mode_key(view_mode), 2);
    }

    // --- Input mapping ---

    pub fn set_input_mapping(&mut self, action_id: &str, mapping: &str) {
        let key = format!("Mapping.{action_id}");
        self.set("Input", &key, mapping);
    }

    pub fn get_input_mapping(&self, action_id: &str) -> Option<String> {
        let key = format!("Mapping.{action_id}");
        self.get("Input", &key)
    }

    // --- Tilt / Nudge ---

    pub fn set_plumb_damping(&mut self, value: f32) {
        self.set_f32("Player", "PlumbDamping", value);
    }

    pub fn set_plumb_threshold_angle(&mut self, value: f32) {
        self.set_f32("Player", "PlumbThresholdAngle", value);
    }

    // --- Audio ---

    pub fn set_sound_device_bg(&mut self, device: &str) {
        self.set("Player", "SoundDeviceBG", device);
    }

    pub fn set_sound_device_pf(&mut self, device: &str) {
        self.set("Player", "SoundDevice", device);
    }

    pub fn set_sound_3d_mode(&mut self, mode: i32) {
        self.set_i32("Player", "Sound3D", mode);
    }

    pub fn set_music_volume(&mut self, volume: i32) {
        self.set_i32("Player", "MusicVolume", volume);
    }

    pub fn set_sound_volume(&mut self, volume: i32) {
        self.set_i32("Player", "SoundVolume", volume);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_version_from_tags() {
        assert_eq!(
            settings_version_of_tag("v10.9.0-6988-556075741").as_deref(),
            Some("10.9")
        );
        assert_eq!(
            settings_version_of_tag("v10.8.1-6823-1f940f946").as_deref(),
            Some("10.8")
        );
        assert_eq!(settings_version_of_tag("10.9.0").as_deref(), Some("10.9"));
        assert_eq!(settings_version_of_tag("nightly"), None);
    }

    #[test]
    fn a_new_settings_folder_starts_from_the_previous_one() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join("10.8");
        std::fs::create_dir_all(old.join("directoutputconfig")).unwrap();
        std::fs::write(old.join("VPinballX.ini"), "[Player]\nBGSet = 1\n").unwrap();
        std::fs::write(old.join("directoutputconfig/directoutputconfig30.ini"), "x").unwrap();
        assert_eq!(migrate_settings(base.path(), "10.9"), Some(old.clone()));
        let new = base.path().join("10.9");
        assert_eq!(
            std::fs::read_to_string(new.join("VPinballX.ini")).unwrap(),
            "[Player]\nBGSet = 1\n"
        );
        assert!(
            new.join("directoutputconfig/directoutputconfig30.ini")
                .is_file()
        );
        // done once: a second call leaves the folder alone
        std::fs::write(new.join("VPinballX.ini"), "changed").unwrap();
        assert_eq!(migrate_settings(base.path(), "10.9"), None);
        assert_eq!(
            std::fs::read_to_string(new.join("VPinballX.ini")).unwrap(),
            "changed"
        );
        // nothing older: nothing to do
        assert_eq!(migrate_settings(base.path(), "10.0"), None);
    }
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn config_from_str(content: &str) -> VpxConfig {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(content.as_bytes()).unwrap();
        VpxConfig::load(Some(tmp.path())).unwrap()
    }

    #[test]
    fn load_nonexistent_creates_empty() {
        let cfg = VpxConfig::load(Some(Path::new("/tmp/pinready_test_nonexistent.ini"))).unwrap();
        assert!(cfg.get("Player", "BGSet").is_none());
    }

    #[test]
    fn get_set_roundtrip() {
        let mut cfg = config_from_str("");
        cfg.set("Player", "BGSet", "2");
        assert_eq!(cfg.get("Player", "BGSet"), Some("2".to_string()));
    }

    #[test]
    fn get_i32_valid() {
        let cfg = config_from_str("[Player]\nBGSet = 2\n");
        assert_eq!(cfg.get_i32("Player", "BGSet"), Some(2));
    }

    #[test]
    fn get_i32_invalid_returns_none() {
        let cfg = config_from_str("[Player]\nBGSet = abc\n");
        assert_eq!(cfg.get_i32("Player", "BGSet"), None);
    }

    #[test]
    fn get_f32_valid() {
        let cfg = config_from_str("[Player]\nAAFactor = 1.5\n");
        assert!((cfg.get_f32("Player", "AAFactor").unwrap() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn get_f32_missing_returns_none() {
        let cfg = config_from_str("");
        assert_eq!(cfg.get_f32("Player", "AAFactor"), None);
    }

    #[test]
    fn set_i32_and_read_back() {
        let mut cfg = config_from_str("");
        cfg.set_i32("Player", "Sound3D", 4);
        assert_eq!(cfg.get_i32("Player", "Sound3D"), Some(4));
    }

    #[test]
    fn set_f32_and_read_back() {
        let mut cfg = config_from_str("");
        cfg.set_f32("Player", "NudgeStrength", 0.02);
        let val = cfg.get_f32("Player", "NudgeStrength").unwrap();
        assert!((val - 0.02).abs() < 0.001);
    }

    #[test]
    fn set_display_writes_all_keys() {
        let mut cfg = config_from_str("");
        cfg.set_display("Player", "Playfield", "Samsung U28E590", 3840, 2160, false);
        assert_eq!(
            cfg.get("Player", "PlayfieldDisplay"),
            Some("Samsung U28E590".to_string())
        );
        assert_eq!(cfg.get_i32("Player", "PlayfieldWidth"), Some(3840));
        assert_eq!(cfg.get_i32("Player", "PlayfieldHeight"), Some(2160));
        // Borderless fullscreen must be pinned: VPX defaults to 0 = Windowed
        // and scatters the windows (cab regression found by config diff).
        assert_eq!(cfg.get_i32("Player", "PlayfieldFullScreen"), Some(1));
    }

    #[test]
    fn set_display_with_output_enabled() {
        let mut cfg = config_from_str("");
        cfg.set_display("Backglass", "Backglass", "LG 27", 2560, 1440, true);
        assert_eq!(cfg.get_i32("Backglass", "BackglassOutput"), Some(1));
    }

    #[test]
    fn set_display_without_output() {
        let mut cfg = config_from_str("");
        cfg.set_display("Player", "Playfield", "Test", 1920, 1080, false);
        assert_eq!(cfg.get_i32("Player", "PlayfieldOutput"), None);
    }

    #[test]
    fn set_view_mode() {
        let mut cfg = config_from_str("");
        cfg.set_view_mode(1);
        assert_eq!(cfg.get_i32("Player", "BGSet"), Some(1));
    }

    #[test]
    fn window_projection_follows_the_view_set() {
        // VPX keeps one projection mode per view set and reads the one that
        // matches BGSet, so head tracking must land on the right key or it
        // changes nothing at all.
        for (bg_set, key) in [(0, "ViewDTMode"), (1, "ViewCabMode"), (2, "ViewFSSMode")] {
            let mut cfg = config_from_str("");
            cfg.set_window_projection(bg_set);
            assert_eq!(cfg.get_i32("TableOverride", key), Some(2), "BGSet {bg_set}");
        }
    }

    #[test]
    fn input_mapping_roundtrip() {
        let mut cfg = config_from_str("");
        cfg.set_input_mapping("LeftFlipper", "Key;42");
        assert_eq!(
            cfg.get_input_mapping("LeftFlipper"),
            Some("Key;42".to_string())
        );
    }

    #[test]
    fn input_mapping_missing() {
        let cfg = config_from_str("");
        assert_eq!(cfg.get_input_mapping("NonExistent"), None);
    }

    #[test]
    fn audio_config_setters() {
        let mut cfg = config_from_str("");
        cfg.set_sound_device_bg("HD Audio");
        cfg.set_sound_device_pf("USB Audio");
        cfg.set_sound_3d_mode(5);
        cfg.set_music_volume(80);
        cfg.set_sound_volume(60);
        assert_eq!(
            cfg.get("Player", "SoundDeviceBG"),
            Some("HD Audio".to_string())
        );
        assert_eq!(
            cfg.get("Player", "SoundDevice"),
            Some("USB Audio".to_string())
        );
        assert_eq!(cfg.get_i32("Player", "Sound3D"), Some(5));
        assert_eq!(cfg.get_i32("Player", "MusicVolume"), Some(80));
        assert_eq!(cfg.get_i32("Player", "SoundVolume"), Some(60));
    }

    #[test]
    fn save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ini");
        {
            let mut cfg = VpxConfig::load(Some(&path)).unwrap();
            cfg.set("Player", "BGSet", "2");
            cfg.set_i32("Player", "Sound3D", 4);
            cfg.save().unwrap();
        }
        let cfg = VpxConfig::load(Some(&path)).unwrap();
        assert_eq!(cfg.get("Player", "BGSet"), Some("2".to_string()));
        assert_eq!(cfg.get_i32("Player", "Sound3D"), Some(4));
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/dir/test.ini");
        let mut cfg = VpxConfig::load(Some(&path)).unwrap();
        cfg.set("Test", "Key", "Value");
        cfg.save().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn preserves_existing_ini_content() {
        let content = "[Player]\nBGSet = 0\nSound3D = 2\n";
        let mut cfg = config_from_str(content);
        cfg.set("Player", "BGSet", "1");
        // Sound3D should remain untouched
        assert_eq!(cfg.get_i32("Player", "Sound3D"), Some(2));
        assert_eq!(cfg.get_i32("Player", "BGSet"), Some(1));
    }
}
