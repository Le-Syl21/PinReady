/// Tilt/nudge sensitivity configuration state.
/// All three main parameters are 0–100% sliders for consistency.
#[derive(Debug, Clone)]
pub struct TiltConfig {
    /// Tilt sensitivity 0–100% — 0 insensitive (large angle to trigger tilt), 100 ultra-sensitive (tiny angle triggers).
    /// Written as PlumbThresholdAngle in INI: range 0.5°..4° (VPX upstream Player::Plumb after the nudge handler rewrite).
    pub tilt_sensitivity_pct: f32,
    /// PlumbDamping: tilt plumb simulation damping (0..2, VPX default 1.0). Replaces the older PlumbInertia.
    pub plumb_damping: f32,
    /// Nudge sensitivity 0–100% — written as scale in NudgeX/Y mapping
    pub nudge_scale_pct: f32,
    /// Nudge deadzone 0–100% — movements below this are ignored (anti-noise)
    pub nudge_deadzone_pct: f32,
}

const TILT_ANGLE_MIN: f32 = 0.5;
const TILT_ANGLE_MAX: f32 = 4.0;
const TILT_ANGLE_RANGE: f32 = TILT_ANGLE_MAX - TILT_ANGLE_MIN;

impl Default for TiltConfig {
    fn default() -> Self {
        Self {
            // User-tuned defaults: scale 50, deadzone 10, sensitivity 50
            // — matches the slider order on the tilt wizard page.
            tilt_sensitivity_pct: 50.0,
            plumb_damping: 1.0,
            nudge_scale_pct: 50.0,
            nudge_deadzone_pct: 10.0,
        }
    }
}

impl TiltConfig {
    pub fn load_from_config(&mut self, config: &crate::config::VpxConfig) {
        if let Some(v) = config.get_f32("Player", "PlumbThresholdAngle") {
            // Inverted mapping: small angle = high sensitivity, large angle = low sensitivity.
            // 0.5° → 100% (ultra-sensitive), 4° → 0% (insensitive).
            self.tilt_sensitivity_pct =
                ((TILT_ANGLE_MAX - v) / TILT_ANGLE_RANGE * 100.0).clamp(0.0, 100.0);
        }
        if let Some(v) = config.get_f32("Player", "PlumbDamping") {
            self.plumb_damping = v;
        }
        // Parse deadzone + scale from NudgeX1 mapping: "device;axis;type;deadZone;scale;limit"
        if let Some(mapping) = config.get("Input", "Mapping.NudgeX1") {
            let parts: Vec<&str> = mapping.split(';').collect();
            if parts.len() >= 5 {
                if let Ok(dz) = parts[3].parse::<f32>() {
                    self.nudge_deadzone_pct = dz * 100.0;
                }
                if let Ok(s) = parts[4].parse::<f32>() {
                    self.nudge_scale_pct = s * 100.0;
                }
            }
        }
    }

    pub fn save_to_config(&self, config: &mut crate::config::VpxConfig) {
        config.set_plumb_damping(self.plumb_damping);
        // Inverted mapping: 0% (insensitive) → 4°, 100% (ultra-sensitive) → 0.5°.
        config.set_plumb_threshold_angle(
            TILT_ANGLE_MAX - (self.tilt_sensitivity_pct / 100.0) * TILT_ANGLE_RANGE,
        );
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
                    "{};{};{};{:.6};{:.6};{}",
                    parts[0],
                    parts[1],
                    parts[2],
                    self.nudge_deadzone_pct / 100.0,
                    self.nudge_scale_pct / 100.0,
                    parts[5]
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
        assert!((tilt.tilt_sensitivity_pct - 50.0).abs() < f32::EPSILON);
        assert!((tilt.plumb_damping - 1.0).abs() < f32::EPSILON);
        assert!((tilt.nudge_scale_pct - 50.0).abs() < f32::EPSILON);
        assert!((tilt.nudge_deadzone_pct - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn load_from_config_reads_values() {
        // PlumbThresholdAngle=2.25 (mid range 0.5..4) → pct = (4 - 2.25)/3.5 * 100 = 50%
        let cfg = config_from_str(
            "[Player]\nPlumbThresholdAngle = 2.25\nPlumbDamping = 0.5\n\
             [Input]\nMapping.NudgeX1 = dev;512;A;0.1;0.8;1.0\n",
        );
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.tilt_sensitivity_pct - 50.0).abs() < 0.1);
        assert!((tilt.plumb_damping - 0.5).abs() < f32::EPSILON);
        assert!((tilt.nudge_scale_pct - 80.0).abs() < 0.1);
        assert!((tilt.nudge_deadzone_pct - 10.0).abs() < 0.1);
    }

    #[test]
    fn load_from_config_keeps_defaults_when_empty() {
        let cfg = config_from_str("");
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.tilt_sensitivity_pct - 50.0).abs() < f32::EPSILON);
        assert!((tilt.plumb_damping - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn load_from_config_inverted_sensitivity() {
        // 0.5° (min angle, ultra-sensitive) → 100%, 4° (max angle, insensitive) → 0%
        let ultra = config_from_str("[Player]\nPlumbThresholdAngle = 0.5\n");
        let mut t = TiltConfig::default();
        t.load_from_config(&ultra);
        assert!((t.tilt_sensitivity_pct - 100.0).abs() < 0.1);

        let insens = config_from_str("[Player]\nPlumbThresholdAngle = 4.0\n");
        let mut t = TiltConfig::default();
        t.load_from_config(&insens);
        assert!(t.tilt_sensitivity_pct.abs() < 0.1);
    }

    #[test]
    fn save_to_config_writes_values() {
        let mut cfg = config_from_str("");
        let tilt = TiltConfig {
            tilt_sensitivity_pct: 50.0, // mid → angle 2.25°
            plumb_damping: 0.7,
            nudge_scale_pct: 150.0,
            nudge_deadzone_pct: 20.0,
        };
        tilt.save_to_config(&mut cfg);
        let angle = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((angle - 2.25).abs() < 0.01);
        let damping = cfg.get_f32("Player", "PlumbDamping").unwrap();
        assert!((damping - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn save_to_config_inverted_sensitivity() {
        // pct=100 (ultra) → smallest angle, pct=0 (insensitive) → largest angle
        let mut cfg = config_from_str("");
        let ultra = TiltConfig {
            tilt_sensitivity_pct: 100.0,
            ..Default::default()
        };
        ultra.save_to_config(&mut cfg);
        let a = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((a - 0.5).abs() < 0.01, "expected 0.5, got {a}");

        let insens = TiltConfig {
            tilt_sensitivity_pct: 0.0,
            ..Default::default()
        };
        insens.save_to_config(&mut cfg);
        let a = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((a - 4.0).abs() < 0.01, "expected 4.0, got {a}");
    }

    #[test]
    fn save_roundtrip() {
        // PlumbThresholdAngle=3.0 → pct ≈ 28.57 → save back → 3.0
        let ini = "[Player]\nPlumbThresholdAngle = 3.0\nPlumbDamping = 0.8\n\
                    [Input]\nMapping.NudgeX1 = dev;512;A;0.05;0.3;1.0\nMapping.NudgeY1 = dev;513;A;0.05;0.3;1.0\n";
        let mut cfg = config_from_str(ini);
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        tilt.nudge_scale_pct = 120.0;
        tilt.save_to_config(&mut cfg);

        let mapping = cfg.get("Input", "Mapping.NudgeX1").unwrap();
        assert!(
            mapping.contains("1.200000"),
            "expected scale 1.2 in: {mapping}"
        );

        let angle = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((angle - 3.0).abs() < 0.01);
    }

    #[test]
    fn update_nudge_mapping_preserves_device_and_axis() {
        let ini = "[Input]\nMapping.NudgeX1 = SDLJoy_PSC004;512;A;0.000000;0.300000;1.000000\n";
        let mut cfg = config_from_str(ini);
        let tilt = TiltConfig {
            nudge_scale_pct: 50.0,
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
            config_from_str("[Input]\nMapping.NudgeX1 = dev;512;A;0.000000;1.750000;1.000000\n");
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.nudge_scale_pct - 175.0).abs() < 0.1);
    }
}
