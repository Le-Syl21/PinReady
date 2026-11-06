//! The 3D page's settings: virtual reality (`[PlayerVR]`) and stereoscopic
//! rendering (`[Player]`). Both are off in a fresh setup, as in a cabinet.
//!
//! VPX's own default for VR is "Disabled" on a desktop build and its stereo
//! mode is "Disabled", so an ini that says nothing reads as both off.

use crate::config::VpxConfig;

/// `PlayerVR/AskToTurnOn`: 0 Enabled, 1 Autodetect, 2 Disabled.
const VR_ENABLED: i32 = 0;
const VR_AUTODETECT: i32 = 1;
const VR_DISABLED: i32 = 2;

/// `Player/Stereo3D` value of the first stereo mode (0 is "Disabled").
pub const STEREO_FIRST: i32 = 1;
/// The last mode PinReady offers: the ten built-in formats. The four custom
/// anaglyph slots (11 to 14) need their own filter matrices and stay in VPX.
pub const STEREO_LAST: i32 = 10;

#[derive(Clone, Debug, PartialEq)]
pub struct ThreeD {
    pub vr: bool,
    /// With VR on: start it always (true) or only when a headset is found.
    pub vr_always: bool,
    /// 0 runtime default, then 72, 80, 90 and 120 Hz.
    pub vr_refresh: i32,
    /// 0 nothing, 1 playfield and other screens, 2 playfield only, 3 left
    /// eye, 4 right eye, 5 both eyes.
    pub vr_desktop: i32,
    pub vr_shrink: bool,
    pub vr_backglass: bool,
    pub vr_audio_backglass: bool,
    pub vr_audio_playfield: bool,
    /// 0 off, otherwise a `Player/Stereo3D` mode in `STEREO_FIRST..=STEREO_LAST`.
    pub stereo: i32,
    /// A mode set by hand in VPX that this page does not offer (a custom
    /// anaglyph slot). Shown as off, and left alone when saving unless a
    /// mode is chosen here.
    pub stereo_custom: Option<i32>,
    pub stereo_eye_mm: f32,
    pub stereo_brightness: f32,
    pub stereo_saturation: f32,
}

impl Default for ThreeD {
    fn default() -> Self {
        Self {
            vr: false,
            vr_always: false,
            vr_refresh: 0,
            vr_desktop: 1,
            vr_shrink: false,
            vr_backglass: false,
            vr_audio_backglass: true,
            vr_audio_playfield: true,
            stereo: 0,
            stereo_custom: None,
            stereo_eye_mm: 63.0,
            stereo_brightness: 1.0,
            stereo_saturation: 1.0,
        }
    }
}

impl ThreeD {
    pub fn load(config: &VpxConfig) -> Self {
        let d = Self::default();
        let flag =
            |key: &str, default: bool| config.get_i32("PlayerVR", key).map_or(default, |v| v != 0);
        let ask = config
            .get_i32("PlayerVR", "AskToTurnOn")
            .unwrap_or(VR_DISABLED);
        // Modes PinReady does not offer (the custom slots) read as off, so
        // the page never shows a mode it cannot save back; they are
        // remembered so saving does not wipe them.
        let mode = config.get_i32("Player", "Stereo3D").unwrap_or(0);
        let offered = (STEREO_FIRST..=STEREO_LAST).contains(&mode);
        let stereo = if offered { mode } else { 0 };
        let stereo_custom = (mode > STEREO_LAST).then_some(mode);
        Self {
            vr: ask == VR_ENABLED || ask == VR_AUTODETECT,
            vr_always: ask == VR_ENABLED,
            vr_refresh: config
                .get_i32("PlayerVR", "DisplayRefreshRate")
                .unwrap_or(d.vr_refresh)
                .clamp(0, 4),
            vr_desktop: config
                .get_i32("PlayerVR", "DesktopDisplay")
                .unwrap_or(d.vr_desktop)
                .clamp(0, 5),
            vr_shrink: flag("ShrinkDesktopDisplay", d.vr_shrink),
            vr_backglass: flag("AddBackglass", d.vr_backglass),
            vr_audio_backglass: flag("SpatialAudioBackglass", d.vr_audio_backglass),
            vr_audio_playfield: flag("SpatialAudioPlayfield", d.vr_audio_playfield),
            stereo,
            stereo_custom,
            stereo_eye_mm: config
                .get_f32("Player", "Stereo3DEyeSeparation")
                .unwrap_or(d.stereo_eye_mm)
                .clamp(5.0, 200.0),
            stereo_brightness: config
                .get_f32("Player", "Stereo3DBrightness")
                .unwrap_or(d.stereo_brightness)
                .clamp(0.0, 2.0),
            stereo_saturation: config
                .get_f32("Player", "Stereo3DSaturation")
                .unwrap_or(d.stereo_saturation)
                .clamp(0.0, 2.0),
        }
    }

    pub fn save(&self, config: &mut VpxConfig) {
        let b = |v: bool| i32::from(v);
        let ask = match (self.vr, self.vr_always) {
            (false, _) => VR_DISABLED,
            (true, true) => VR_ENABLED,
            (true, false) => VR_AUTODETECT,
        };
        config.set_i32("PlayerVR", "AskToTurnOn", ask);
        config.set_i32(
            "PlayerVR",
            "DisplayRefreshRate",
            self.vr_refresh.clamp(0, 4),
        );
        config.set_i32("PlayerVR", "DesktopDisplay", self.vr_desktop.clamp(0, 5));
        config.set_i32("PlayerVR", "ShrinkDesktopDisplay", b(self.vr_shrink));
        config.set_i32("PlayerVR", "AddBackglass", b(self.vr_backglass));
        config.set_i32(
            "PlayerVR",
            "SpatialAudioBackglass",
            b(self.vr_audio_backglass),
        );
        config.set_i32(
            "PlayerVR",
            "SpatialAudioPlayfield",
            b(self.vr_audio_playfield),
        );

        // A custom mode set by hand in VPX stays as it is until a mode is
        // chosen here. "Enable stereo rendering" is VPX's temporary switch:
        // on whenever a mode is chosen, so choosing one is enough.
        if self.stereo > 0 || self.stereo_custom.is_none() {
            config.set_i32("Player", "Stereo3D", self.stereo.clamp(0, STEREO_LAST));
            config.set_i32("Player", "Stereo3DEnabled", b(self.stereo > 0));
        }
        config.set_f32(
            "Player",
            "Stereo3DEyeSeparation",
            self.stereo_eye_mm.clamp(5.0, 200.0),
        );
        config.set_f32(
            "Player",
            "Stereo3DBrightness",
            self.stereo_brightness.clamp(0.0, 2.0),
        );
        config.set_f32(
            "Player",
            "Stereo3DSaturation",
            self.stereo_saturation.clamp(0.0, 2.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> VpxConfig {
        let dir = std::env::temp_dir().join(format!("pinready-3d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("VPinballX.ini");
        std::fs::write(&path, "[Player]\n").unwrap();
        VpxConfig::load(Some(&path)).unwrap()
    }

    #[test]
    fn an_empty_ini_has_vr_and_stereo_off() {
        let t = ThreeD::load(&config());
        assert_eq!(t, ThreeD::default());
        assert!(!t.vr);
        assert_eq!(t.stereo, 0);
    }

    #[test]
    fn what_is_saved_is_read_back() {
        let mut c = config();
        let t = ThreeD {
            vr: true,
            vr_always: true,
            vr_refresh: 3,
            vr_desktop: 5,
            vr_shrink: true,
            vr_backglass: true,
            vr_audio_backglass: false,
            vr_audio_playfield: false,
            stereo: 5,
            stereo_custom: None,
            stereo_eye_mm: 60.0,
            stereo_brightness: 1.25,
            stereo_saturation: 0.75,
        };
        t.save(&mut c);
        assert_eq!(ThreeD::load(&c), t);
    }

    #[test]
    fn vr_modes_map_to_vpxs_three_values() {
        for (vr, always, want) in [(false, false, 2), (true, true, 0), (true, false, 1)] {
            let mut c = config();
            ThreeD {
                vr,
                vr_always: always,
                ..ThreeD::default()
            }
            .save(&mut c);
            assert_eq!(c.get_i32("PlayerVR", "AskToTurnOn"), Some(want));
        }
    }

    #[test]
    fn disabled_vr_keeps_its_other_choices() {
        let mut c = config();
        let t = ThreeD {
            vr: false,
            vr_refresh: 2,
            ..ThreeD::default()
        };
        t.save(&mut c);
        assert_eq!(ThreeD::load(&c).vr_refresh, 2);
    }

    #[test]
    fn a_custom_anaglyph_slot_reads_as_off_and_survives_a_save() {
        let mut c = config();
        c.set_i32("Player", "Stereo3D", 12);
        let t = ThreeD::load(&c);
        assert_eq!((t.stereo, t.stereo_custom), (0, Some(12)));
        t.save(&mut c);
        assert_eq!(c.get_i32("Player", "Stereo3D"), Some(12));
        ThreeD { stereo: 3, ..t }.save(&mut c);
        assert_eq!(c.get_i32("Player", "Stereo3D"), Some(3));
    }

    #[test]
    fn choosing_a_stereo_mode_turns_the_stereo_switch_on() {
        let mut c = config();
        ThreeD {
            stereo: 4,
            ..ThreeD::default()
        }
        .save(&mut c);
        assert_eq!(c.get_i32("Player", "Stereo3DEnabled"), Some(1));
        ThreeD::default().save(&mut c);
        assert_eq!(c.get_i32("Player", "Stereo3DEnabled"), Some(0));
    }
}
