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
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
        }
        let content = self.ini.to_string();
        let line_count = content.split('\n').count();
        log::info!("Saving ini: {} bytes, {} lines to {}", content.len(), line_count, self.path.display());
        self.ini.save(&self.path).map_err(|e| anyhow::anyhow!("{e}"))
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
    pub fn set_display(&mut self, section: &str, prefix: &str, name: &str, w: i32, h: i32, enable_output: bool) {
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
        self.set_i32("Player", &key, if enabled { 1 } else { 0 });
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
