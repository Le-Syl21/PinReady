/// Tilt/nudge sensitivity configuration state.
/// All three main parameters are 0–100% sliders for consistency.
#[derive(Debug, Clone)]
pub struct TiltConfig {
    /// Tilt sensitivity 0–100% — 0 insensitive (large angle to trigger tilt), 100 ultra-sensitive (tiny angle triggers).
    /// Written as PlumbThresholdAngle in INI: range 0.15°..4° (VPX upstream widened the
    /// floor from 0.5° and moved the default 2°→1° in Aug 2026 — commit 05dd11dc6 era).
    pub tilt_sensitivity_pct: f32,
    /// PlumbDamping: tilt plumb simulation damping (0..2, VPX default 1.0). Replaces the older PlumbInertia.
    pub plumb_damping: f32,
    /// Nudge sensitivity 0–100% — written as `Mapping.Nudge0.Strength`
    /// (0..2, neutral at 1.0), *not* as the mapping's unit scale.
    pub nudge_scale_pct: f32,
    /// Accelerometer full-scale range in g (1, 2, 4 or 8) — written as the
    /// mapping's scale, converted to m/s². mjr recommends 1 g for a cabinet:
    /// "a 1G acceleration is pretty strong in this context".
    pub nudge_range_g: f32,
    /// Nudge deadzone 0–100% — movements below this are ignored (anti-noise)
    pub nudge_deadzone_pct: f32,
    /// Nudge sensor type — VPX `Mapping.Nudge0.Type` enum after the nudge-handler
    /// rewrite (10.8.1 rev 5277+): 0 = Game Controller, 1 = Intent Sensor,
    /// 2 = Cabinet Sensor. Default 1 (recommended for HID accelerometers like
    /// Pinscape — not high-frequency/noise-free enough for direct Cabinet Sensor).
    pub nudge_sensor_type: i32,
}

const TILT_ANGLE_MIN: f32 = 0.15;
const TILT_ANGLE_MAX: f32 = 4.0;
const TILT_ANGLE_RANGE: f32 = TILT_ANGLE_MAX - TILT_ANGLE_MIN;

/// VPX splits two things PinReady used to conflate. A mapping's `scale` is a
/// **unit conversion**: the accelerometer's physical range, so that a
/// full-scale reading becomes the right number of m/s² (VPX's own sensor page
/// offers 1/2/4/8 g, "which is what Pinscape boards propose"). Sensitivity —
/// how hard a given shake should hit the ball — has its own field,
/// `Mapping.Nudge0.Strength`, running 0..2 around a neutral 1.
///
/// Driving sensitivity through `scale` meant lying to the engine about what
/// the sensor is, and made a 4 g or 8 g board impossible to describe.
const GRAVITY: f32 = 9.806_65;
/// Slider percent → Strength. 50 % is neutral (1.0), 100 % is the 2.0 ceiling
/// VPX accepts.
const NUDGE_STRENGTH_PER_PCT: f32 = 2.0 / 100.0;

impl Default for TiltConfig {
    fn default() -> Self {
        Self {
            // Cabinet-tuned defaults, in the order the sliders appear on
            // the tilt page: nudge 50 %, deadzone 10 %, tilt 75 %.
            tilt_sensitivity_pct: 75.0,
            plumb_damping: 1.0,
            nudge_scale_pct: 50.0,
            nudge_range_g: 1.0,
            nudge_deadzone_pct: 10.0,
            nudge_sensor_type: 1,
        }
    }
}

impl TiltConfig {
    pub fn load_from_config(&mut self, config: &crate::config::VpxConfig) {
        if let Some(v) = config.get_f32("Player", "PlumbThresholdAngle") {
            // Inverted mapping: small angle = high sensitivity, large angle = low sensitivity.
            // 0.15° → 100% (ultra-sensitive), 4° → 0% (insensitive).
            self.tilt_sensitivity_pct =
                ((TILT_ANGLE_MAX - v) / TILT_ANGLE_RANGE * 100.0).clamp(0.0, 100.0);
        }
        if let Some(v) = config.get_f32("Player", "PlumbDamping") {
            self.plumb_damping = v;
        }
        // Parse deadzone + scale from the accelerometer axis of the new nudge
        // sensor schema: "device;axis;type;deadZone;scale;limit".
        if let Some(mapping) = config.get("Input", "Mapping.Nudge0.AccX") {
            let parts: Vec<&str> = mapping.split(';').collect();
            if parts.len() >= 5 {
                if let Ok(dz) = parts[3].parse::<f32>() {
                    self.nudge_deadzone_pct = dz * 100.0;
                }
                if let Ok(s) = parts[4].parse::<f32>() {
                    // Snap to the nearest range VPX offers; anything else was
                    // written by hand and is best left readable as its
                    // closest neighbour.
                    let g = (s / GRAVITY).max(0.0);
                    self.nudge_range_g = [1.0, 2.0, 4.0, 8.0]
                        .into_iter()
                        .min_by(|a: &f32, b: &f32| (a - g).abs().total_cmp(&(b - g).abs()))
                        .unwrap_or(1.0);
                }
            }
        }
        if let Some(v) = config.get_i32("Input", "Mapping.Nudge0.Type") {
            self.nudge_sensor_type = v;
        }
        if let Some(v) = config.get_f32("Input", "Mapping.Nudge0.Strength") {
            self.nudge_scale_pct = (v / NUDGE_STRENGTH_PER_PCT).clamp(0.0, 100.0);
        }
    }

    pub fn save_to_config(&self, config: &mut crate::config::VpxConfig) {
        config.set_plumb_damping(self.plumb_damping);
        // Inverted mapping: 0% (insensitive) → 4°, 100% (ultra-sensitive) → 0.15°.
        config.set_plumb_threshold_angle(
            TILT_ANGLE_MAX - (self.tilt_sensitivity_pct / 100.0) * TILT_ANGLE_RANGE,
        );
        // Update scale and deadZone on the accelerometer axes of the new nudge
        // sensor schema, and persist the sensor type.
        self.update_nudge_mapping(config, "Nudge0.AccX");
        self.update_nudge_mapping(config, "Nudge0.AccY");
        config.set_i32("Input", "Mapping.Nudge0.Type", self.nudge_sensor_type);
        config.set(
            "Input",
            "Mapping.Nudge0.Strength",
            &format!("{:.6}", self.nudge_scale_pct * NUDGE_STRENGTH_PER_PCT),
        );
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
                    self.nudge_range_g * GRAVITY,
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
        assert!((tilt.tilt_sensitivity_pct - 75.0).abs() < f32::EPSILON);
        assert!((tilt.plumb_damping - 1.0).abs() < f32::EPSILON);
        assert!((tilt.nudge_scale_pct - 50.0).abs() < f32::EPSILON);
        assert!((tilt.nudge_deadzone_pct - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn load_from_config_reads_values() {
        // PlumbThresholdAngle=2.075 (mid range 0.15..4) → pct = (4 - 2.075)/3.85 * 100 = 50%
        let cfg = config_from_str(
            "[Player]\nPlumbThresholdAngle = 2.075\nPlumbDamping = 0.5\n\
             [Input]\nMapping.Nudge0.AccX = dev;512;A;0.1;9.80665;1.0\n",
        );
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.tilt_sensitivity_pct - 50.0).abs() < 0.1);
        assert!((tilt.plumb_damping - 0.5).abs() < f32::EPSILON);
        // 1g is mid-course on the slider.
        assert!((tilt.nudge_scale_pct - 50.0).abs() < 0.1);
        assert!((tilt.nudge_deadzone_pct - 10.0).abs() < 0.1);
    }

    #[test]
    fn load_from_config_keeps_defaults_when_empty() {
        let cfg = config_from_str("");
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!((tilt.tilt_sensitivity_pct - 75.0).abs() < f32::EPSILON);
        assert!((tilt.plumb_damping - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn load_from_config_inverted_sensitivity() {
        // 0.15° (min angle, ultra-sensitive) → 100%, 4° (max angle, insensitive) → 0%
        let ultra = config_from_str("[Player]\nPlumbThresholdAngle = 0.15\n");
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
            tilt_sensitivity_pct: 50.0, // mid → angle 2.075°
            plumb_damping: 0.7,
            nudge_scale_pct: 150.0,
            nudge_deadzone_pct: 20.0,
            ..Default::default()
        };
        tilt.save_to_config(&mut cfg);
        let angle = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((angle - 2.075).abs() < 0.01);
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
        assert!((a - 0.15).abs() < 0.01, "expected 0.15, got {a}");

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
                    [Input]\nMapping.Nudge0.AccX = dev;512;A;0.05;0.3;1.0\nMapping.Nudge0.AccY = dev;513;A;0.05;0.3;1.0\n";
        let mut cfg = config_from_str(ini);
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        tilt.nudge_scale_pct = 100.0;
        tilt.nudge_range_g = 2.0;
        tilt.save_to_config(&mut cfg);

        // The mapping scale is the sensor range in m/s^2 …
        let mapping = cfg.get("Input", "Mapping.Nudge0.AccX").unwrap();
        assert!(
            mapping.contains("19.613300"),
            "expected the 2g range in: {mapping}"
        );
        // … and the slider rides Strength, 100 % being VPX's 2.0 ceiling.
        let strength = cfg.get_f32("Input", "Mapping.Nudge0.Strength").unwrap();
        assert!(
            (strength - 2.0).abs() < 0.001,
            "expected 2.0, got {strength}"
        );

        let angle = cfg.get_f32("Player", "PlumbThresholdAngle").unwrap();
        assert!((angle - 3.0).abs() < 0.01);
    }

    #[test]
    fn update_nudge_mapping_preserves_device_and_axis() {
        let ini = "[Input]\nMapping.Nudge0.AccX = SDLJoy_PSC004;512;A;0.000000;0.300000;1.000000\n";
        let mut cfg = config_from_str(ini);
        let tilt = TiltConfig {
            nudge_scale_pct: 50.0,
            ..Default::default()
        };
        tilt.save_to_config(&mut cfg);
        let mapping = cfg.get("Input", "Mapping.Nudge0.AccX").unwrap();
        assert!(mapping.starts_with("SDLJoy_PSC004;512;A;"));
        assert!(mapping.contains("9.806650"));
        assert!(mapping.ends_with(";1.000000"));
    }

    #[test]
    fn nudge_scale_parsed_from_mapping() {
        // A mapping written for a 4g board reads back as 4g, and the
        // sensitivity comes from Strength rather than from that scale.
        let cfg = config_from_str(
            "[Input]\nMapping.Nudge0.AccX = dev;512;A;0.000000;39.226600;1.000000\n\
             Mapping.Nudge0.Strength = 1.500000\n",
        );
        let mut tilt = TiltConfig::default();
        tilt.load_from_config(&cfg);
        assert!(
            (tilt.nudge_range_g - 4.0).abs() < 0.01,
            "got {}",
            tilt.nudge_range_g
        );
        assert!(
            (tilt.nudge_scale_pct - 75.0).abs() < 0.1,
            "got {}",
            tilt.nudge_scale_pct
        );
    }
}
