/// Tilt/nudge sensitivity configuration state.
#[derive(Debug, Clone)]
pub struct TiltConfig {
    /// PlumbThresholdAngle: angle in degrees that triggers TILT (5–60)
    pub plumb_threshold_angle: f32,
    /// PlumbInertia: tilt plumb simulation inertia (0.001–1.0)
    pub plumb_inertia: f32,
    /// NudgeFilter0: anti-noise filter on accelerometer
    pub nudge_filter: bool,
    /// Scale factor for NudgeX/Y analog mappings (0.1–2.0)
    pub nudge_scale: f32,
}

impl Default for TiltConfig {
    fn default() -> Self {
        Self {
            plumb_threshold_angle: 35.0,
            plumb_inertia: 0.35,
            nudge_filter: true,
            nudge_scale: 0.3,
        }
    }
}

impl TiltConfig {
    pub fn load_from_config(&mut self, config: &crate::config::VpxConfig) {
        if let Some(v) = config.get_f32("Player", "PlumbThresholdAngle") {
            self.plumb_threshold_angle = v;
        }
        if let Some(v) = config.get_f32("Player", "PlumbInertia") {
            self.plumb_inertia = v;
        }
        if let Some(v) = config.get_i32("Player", "NudgeFilter0") {
            self.nudge_filter = v != 0;
        }
        // Parse scale from NudgeX1 mapping: "device;axis;type;deadZone;scale;limit"
        if let Some(mapping) = config.get("Input", "Mapping.NudgeX1") {
            let parts: Vec<&str> = mapping.split(';').collect();
            if parts.len() >= 5 {
                if let Ok(s) = parts[4].parse::<f32>() {
                    self.nudge_scale = s;
                }
            }
        }
    }

    pub fn save_to_config(&self, config: &mut crate::config::VpxConfig) {
        config.set_plumb_inertia(self.plumb_inertia);
        config.set_plumb_threshold_angle(self.plumb_threshold_angle);
        config.set_nudge_filter(0, self.nudge_filter);
        // Update scale and deadZone in NudgeX1/Y1 analog mappings
        self.update_nudge_mapping(config, "NudgeX1");
        self.update_nudge_mapping(config, "NudgeY1");
    }

    fn update_nudge_mapping(&self, config: &mut crate::config::VpxConfig, key: &str) {
        let mapping_key = format!("Mapping.{key}");
        if let Some(mapping) = config.get("Input", &mapping_key) {
            let parts: Vec<&str> = mapping.split(';').collect();
            // Format: device;axis;type;deadZone;scale;limit
            if parts.len() >= 6 {
                let new_mapping = format!(
                    "{};{};{};0.100000;{:.6};{}",
                    parts[0], parts[1], parts[2], self.nudge_scale, parts[5]
                );
                config.set("Input", &mapping_key, &new_mapping);
            }
        }
    }
}
