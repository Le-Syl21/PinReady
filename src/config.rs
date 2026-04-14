use anyhow::{Context, Result};
use ini_preserve::Ini;
use std::path::{Path, PathBuf};

fn default_ini_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/VPinballX/10.8/VPinballX.ini")
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
        let content = self.ini.to_string();
        let line_count = content.split('\n').count();
        log::info!(
            "Saving ini: {} bytes, {} lines to {}",
            content.len(),
            line_count,
            self.path.display()
        );
        self.ini
            .save(&self.path)
            .map_err(|e| anyhow::anyhow!("{e}"))
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
        self.set(section, &format!("{prefix}WndX"), "");
        self.set(section, &format!("{prefix}WndY"), "");
        self.set_i32(section, &format!("{prefix}Width"), w);
        self.set_i32(section, &format!("{prefix}Height"), h);
    }

    pub fn set_view_mode(&mut self, mode: i32) {
        self.set_i32("Player", "BGSet", mode);
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

    pub fn set_plumb_inertia(&mut self, value: f32) {
        self.set_f32("Player", "PlumbInertia", value);
    }

    pub fn set_plumb_threshold_angle(&mut self, value: f32) {
        self.set_f32("Player", "PlumbThresholdAngle", value);
    }

    pub fn set_nudge_filter(&mut self, sensor: u8, enabled: bool) {
        let key = format!("NudgeFilter{sensor}");
        self.set_i32("Player", &key, i32::from(enabled));
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
    fn set_nudge_filter() {
        let mut cfg = config_from_str("");
        cfg.set_nudge_filter(0, true);
        assert_eq!(cfg.get_i32("Player", "NudgeFilter0"), Some(1));
        cfg.set_nudge_filter(0, false);
        assert_eq!(cfg.get_i32("Player", "NudgeFilter0"), Some(0));
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
