//! The visual effects VPX lets a player turn on or off, kept apart from the
//! rest of the rendering page: each one is a plain value read from, and
//! written back to, the `[Player]` section of the ini.
//!
//! Every default below is VPX's own, so a fresh setup changes nothing until
//! someone moves a control.

use crate::config::VpxConfig;

/// Ambient occlusion, as the three states VPX's two keys (`DisableAO`,
/// `DynamicAO`) can describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbientOcclusion {
    Off,
    Fixed,
    Moving,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effects {
    pub ao: AmbientOcclusion,
    pub bloom: bool,
    pub motion_blur: bool,
    pub anisotropic: bool,
    pub screen_reflections: bool,
    /// Frames the GPU may queue: 1 to 3.
    pub prerendered_frames: i32,
    /// Level of detail of balls and ramps: 1 to 10.
    pub detail: i32,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            ao: AmbientOcclusion::Moving,
            bloom: true,
            motion_blur: true,
            anisotropic: true,
            screen_reflections: false,
            prerendered_frames: 1,
            detail: 10,
        }
    }
}

impl Effects {
    pub fn load(config: &VpxConfig) -> Self {
        let d = Self::default();
        let flag =
            |key: &str, default: bool| config.get_i32("Player", key).map_or(default, |v| v != 0);
        let ao = if flag("DisableAO", false) {
            AmbientOcclusion::Off
        } else if flag("DynamicAO", true) {
            AmbientOcclusion::Moving
        } else {
            AmbientOcclusion::Fixed
        };
        Self {
            ao,
            bloom: !flag("ForceBloomOff", !d.bloom),
            motion_blur: !flag("ForceMotionBlurOff", !d.motion_blur),
            anisotropic: flag("ForceAnisotropicFiltering", d.anisotropic),
            screen_reflections: flag("SSRefl", d.screen_reflections),
            prerendered_frames: config
                .get_i32("Player", "MaxPrerenderedFrames")
                .unwrap_or(d.prerendered_frames)
                .clamp(1, 3),
            detail: config
                .get_i32("Player", "AlphaRampAccuracy")
                .unwrap_or(d.detail)
                .clamp(1, 10),
        }
    }

    pub fn save(&self, config: &mut VpxConfig) {
        let b = |v: bool| i32::from(v);
        config.set_i32("Player", "DisableAO", b(self.ao == AmbientOcclusion::Off));
        config.set_i32("Player", "DynamicAO", b(self.ao != AmbientOcclusion::Fixed));
        config.set_i32("Player", "ForceBloomOff", b(!self.bloom));
        config.set_i32("Player", "ForceMotionBlurOff", b(!self.motion_blur));
        config.set_i32("Player", "ForceAnisotropicFiltering", b(self.anisotropic));
        config.set_i32("Player", "SSRefl", b(self.screen_reflections));
        config.set_i32(
            "Player",
            "MaxPrerenderedFrames",
            self.prerendered_frames.clamp(1, 3),
        );
        config.set_i32("Player", "AlphaRampAccuracy", self.detail.clamp(1, 10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> VpxConfig {
        let dir = std::env::temp_dir().join(format!("pinready-effects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("VPinballX.ini");
        std::fs::write(&path, "[Player]\n").unwrap();
        VpxConfig::load(Some(&path)).unwrap()
    }

    #[test]
    fn an_empty_ini_reads_as_vpx_defaults() {
        assert_eq!(Effects::load(&config()), Effects::default());
    }

    #[test]
    fn what_is_saved_is_read_back() {
        let mut c = config();
        let e = Effects {
            ao: AmbientOcclusion::Fixed,
            bloom: false,
            motion_blur: false,
            anisotropic: false,
            screen_reflections: true,
            prerendered_frames: 2,
            detail: 6,
        };
        e.save(&mut c);
        assert_eq!(Effects::load(&c), e);
    }

    #[test]
    fn the_three_ambient_occlusion_states_survive_a_round_trip() {
        for ao in [
            AmbientOcclusion::Off,
            AmbientOcclusion::Fixed,
            AmbientOcclusion::Moving,
        ] {
            let mut c = config();
            Effects {
                ao,
                ..Effects::default()
            }
            .save(&mut c);
            assert_eq!(Effects::load(&c).ao, ao);
        }
    }

    #[test]
    fn out_of_range_numbers_are_brought_back_in() {
        let mut c = config();
        c.set_i32("Player", "MaxPrerenderedFrames", 9);
        c.set_i32("Player", "AlphaRampAccuracy", 0);
        let e = Effects::load(&c);
        assert_eq!((e.prerendered_frames, e.detail), (3, 1));
    }
}
