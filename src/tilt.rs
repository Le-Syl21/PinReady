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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VpxConfig;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn config_from_str(content: &str) -> VpxConfig {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(content.as_bytes()).unwrap();
        VpxConfig::load(Some(tmp.path())).unwrap()
    }

    #[test]
    fn default_values() {
        let tilt = TiltConfig::default();
        assert!((tilt.plumb_threshold_angle - 35.0).abs() < f32::EPSILON);
        assert!((tilt.plumb_inertia - 0.35).abs() < f32::EPSILON);
        assert!(tilt.nudge_filter);
        assert!((tilt.nudge_scale - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn load_from_config_reads_values() {
        let cfg = config_from_str(
            "[Player]\nPlumbThresholdAngle = 45.0\nPlumbInertia = 0.5\nNudgeFilter0 = 0\n\
             [Input]\nMapping.NudgeX1 = dev;512;A;0.1;0.8;1.0\n",
        );
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.plumb_threshold_angle - 45.0).abs() < f32::EPSILON);
        assert!((tilt.plumb_inertia - 0.5).abs() < f32::EPSILON);
        assert!(!tilt.nudge_filter);
        assert!((tilt.nudge_scale - 0.8).abs() < 0.001);
    }

    #[test]
    fn load_from_config_keeps_defaults_when_empty() {
        let cfg = config_from_str("");
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.plumb_threshold_angle - 35.0).abs() < f32::EPSILON);
        assert!(tilt.nudge_filter);
    }

    #[test]
    fn save_to_config_writes_values() {
        let mut cfg = config_from_str("");
        let tilt = TiltConfig {
            plumb_threshold_angle: 50.0,
            plumb_inertia: 0.7,
            nudge_filter: false,
            nudge_scale: 1.5,
        };
        tilt.save_to_config(&mut cfg);
        let angle = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((angle - 50.0).abs() < f32::EPSILON);
        let inertia = cfg.get_f32("Player", "PlumbInertia").unwrap();
        assert!((inertia - 0.7).abs() < f32::EPSILON);
        assert_eq!(cfg.get_i32("Player", "NudgeFilter0"), Some(0));
    }

    #[test]
    fn save_roundtrip() {
        let ini = "[Player]\nPlumbThresholdAngle = 20.0\nPlumbInertia = 0.1\nNudgeFilter0 = 1\n\
                    [Input]\nMapping.NudgeX1 = dev;512;A;0.1;0.3;1.0\nMapping.NudgeY1 = dev;513;A;0.1;0.3;1.0\n";
        let mut cfg = config_from_str(ini);
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        tilt.nudge_scale = 1.2;
        tilt.plumb_threshold_angle = 25.0;
        tilt.save_to_config(&mut cfg);

        // Verify nudge mapping was updated with new scale
        let mapping = cfg.get("Input", "Mapping.NudgeX1").unwrap();
        assert!(
            mapping.contains("1.200000"),
            "expected scale 1.2 in: {mapping}"
        );

        let angle = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((angle - 25.0).abs() < f32::EPSILON);
    }

    #[test]
    fn update_nudge_mapping_preserves_device_and_axis() {
        let ini = "[Input]\nMapping.NudgeX1 = SDLJoy_PSC004;512;A;0.100000;0.300000;1.000000\n";
        let mut cfg = config_from_str(ini);
        let tilt = TiltConfig {
            nudge_scale: 0.5,
            ..Default::default()
        };
        tilt.save_to_config(&mut cfg);
        let mapping = cfg.get("Input", "Mapping.NudgeX1").unwrap();
        assert!(mapping.starts_with("SDLJoy_PSC004;512;A;"));
        assert!(mapping.contains("0.500000"));
        assert!(mapping.ends_with(";1.000000"));
    }

    #[test]
    fn nudge_scale_parsed_from_mapping() {
        let cfg =
            config_from_str("[Input]\nMapping.NudgeX1 = dev;512;A;0.100000;1.750000;1.000000\n");
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.nudge_scale - 1.75).abs() < 0.001);
    }
}
