//! The value VPX reads from a `*Display=` key.
//!
//! Up to VPX 10.8.1 rev 5514 it is the SDL display name (`DP-1 42"`). From rev
//! 5515 (vpinball/vpinball 2ba461a98, the fix for two identical screens) it is
//! a display id: the same name followed by the display's position in the
//! desktop layout, `DP-1 42" [3840, 0]`. A bare name no longer matches any
//! display there, so VPX falls back to the playfield screen for every window:
//! the backglass, the DMD and the topper all open on top of the playfield.

use crate::config::VpxConfig;
use crate::db::Database;

/// The first VPX revision that takes a display id rather than a name.
pub const FIRST_REVISION_WITH_IDS: u32 = 5515;

/// The git revision count in a `[Version] VPinball` stamp (`10816501` → 6501):
/// the stamp is major, minor and patch, one digit each, then the revision.
pub fn revision_from_stamp(stamp: &str) -> Option<u32> {
    let stamp = stamp.trim();
    if stamp.len() <= 4 || !stamp.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stamp[4..].parse().ok()
}

/// The revision in a PinReady VPX tag (`v10.8.1-6501-3d9540861` → 6501).
pub fn revision_from_tag(tag: &str) -> Option<u32> {
    tag.trim().split('-').nth(1)?.parse().ok()
}

/// The revision of the VPX that will read the ini. The installed build's tag
/// says it best (the ini stamp lags until that build has run), then the stamp.
pub fn revision(config: &VpxConfig, db: &Database) -> Option<u32> {
    db.get_config("vpx_installed_tag")
        .and_then(|t| revision_from_tag(&t))
        .or_else(|| {
            config
                .get("Version", "VPinball")
                .and_then(|s| revision_from_stamp(&s))
        })
}

/// Whether that VPX takes display ids; unknown means the builds PinReady
/// installs today, which do.
pub fn uses_ids(config: &VpxConfig, db: &Database) -> bool {
    revision(config, db).is_none_or(|r| r >= FIRST_REVISION_WITH_IDS)
}

/// The first VPX revision without the B2SLegacy plugin (c88057303): its DMD
/// overlay keys and the `Priority.B2SLegacyDMD` renderer are gone. The B2S
/// plugin that replaces it shows the DMD on the score view by itself.
pub const FIRST_REVISION_WITHOUT_B2S_LEGACY: u32 = 6293;

/// Priorities of the renderers on the score view window: PUP first, then B2S,
/// Nixie last (it keeps the default 0).
///
/// VPX draws an ancillary window with the first of its renderers (PUP, Nixie,
/// B2S) that has something to show, in priority order. All three default to 0,
/// so a tie goes to whichever plugin loaded first. On a table with segment
/// displays that left the score view showing Nixie's own segments over the
/// black image PinReady puts in the backglass file: the scores twice, instead
/// of a black screen. B2S before Nixie fixes it. But B2S alone first would
/// also hide a PuP pack that takes the DMD screen, so PUP goes before B2S.
/// Checked on the cab with Monster Bash's pack (DMD screen on, DMD screen off)
/// and with a game that has no pack: the pack's video where it asks for the
/// screen, B2S's DMD where it does not, a black screen for the segment games.
/// A renderer with nothing to show costs nothing: VPX moves on to the next.
pub const SCORE_VIEW_PUP_PRIORITY: i32 = 20;
pub const SCORE_VIEW_B2S_PRIORITY: i32 = 10;

/// Whether a priority should be written: there is a score view screen, the VPX
/// build has the renderers (no B2SLegacy), and the player has not set a value
/// of their own.
fn wants_priority(legacy: bool, on_screen: bool, current: Option<&str>) -> bool {
    !legacy && on_screen && current.is_none_or(|v| v.trim().is_empty())
}

/// Order the score view renderers (PUP, then B2S, then Nixie), unless the
/// player chose otherwise: each priority is only written when it is empty.
pub fn order_score_view_renderers(config: &mut VpxConfig, db: &Database) {
    let legacy = revision(config, db).is_some_and(|r| r < FIRST_REVISION_WITHOUT_B2S_LEGACY);
    let on_screen = config
        .get("ScoreView", "ScoreViewDisplay")
        .is_some_and(|v| !v.is_empty());
    for (key, value) in [
        ("Priority.PUP", SCORE_VIEW_PUP_PRIORITY),
        ("Priority.B2S", SCORE_VIEW_B2S_PRIORITY),
    ] {
        let current = config.get("ScoreView", key);
        if wants_priority(legacy, on_screen, current.as_deref()) {
            config.set_i32("ScoreView", key, value);
        }
    }
}

/// The `*Display=` value for the display `name` at `(x, y)`.
pub fn value(name: &str, x: i32, y: i32, ids: bool) -> String {
    if ids {
        format!("{name} [{x}, {y}]")
    } else {
        name.to_string()
    }
}

/// The display name in a `*Display=` value, whichever form it has.
pub fn name_of(value: &str) -> &str {
    let v = value.trim_end();
    if let Some(open) = v.rfind(" [")
        && v.ends_with(']')
    {
        let inner = &v[open + 2..v.len() - 1];
        let mut parts = inner.split(',').map(str::trim);
        let is_int = |s: Option<&str>| s.is_some_and(|s| s.parse::<i32>().is_ok());
        if is_int(parts.next()) && is_int(parts.next()) && parts.next().is_none() {
            return &v[..open];
        }
    }
    v
}

/// Whether a `*Display=` value designates the display `name` at `(x, y)`:
/// the name must match, and the position too when the value carries one.
pub fn matches(value: &str, name: &str, x: i32, y: i32) -> bool {
    let v = value.trim_end();
    if name_of(v) != name {
        return false;
    }
    v == name || v == self::value(name, x, y, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_comes_from_the_stamp_and_the_tag() {
        assert_eq!(revision_from_stamp("10815430"), Some(5430));
        assert_eq!(revision_from_stamp(" 10816501 "), Some(6501));
        assert_eq!(revision_from_stamp("1081"), None);
        assert_eq!(revision_from_stamp("10.8.1"), None);
        assert_eq!(revision_from_tag("v10.8.1-6501-3d9540861"), Some(6501));
        assert_eq!(revision_from_tag("v10.8.1"), None);
    }

    #[test]
    fn the_score_view_priorities_are_written_unless_the_player_chose() {
        // A score view screen, a current VPX, nothing set: put B2S first.
        assert!(wants_priority(false, true, None));
        assert!(wants_priority(false, true, Some("")));
        assert!(wants_priority(false, true, Some("  ")));
        // The player's own value, even 0 or a negative one, is theirs.
        assert!(!wants_priority(false, true, Some("0")));
        assert!(!wants_priority(false, true, Some("-1")));
        assert!(!wants_priority(false, true, Some("50")));
        // No score view screen: nothing to prioritise.
        assert!(!wants_priority(false, false, None));
        // The builds that still carry B2SLegacy have no such renderer.
        assert!(!wants_priority(true, true, None));
    }

    #[test]
    fn ids_carry_the_position() {
        assert_eq!(value("DP-1 42\"", 3840, 0, true), "DP-1 42\" [3840, 0]");
        assert_eq!(value("DP-1 42\"", 3840, 0, false), "DP-1 42\"");
        assert_eq!(value("HDMI-1", -1920, 0, true), "HDMI-1 [-1920, 0]");
    }

    #[test]
    fn the_name_reads_back_from_both_forms() {
        assert_eq!(name_of("DP-1 42\" [3840, 0]"), "DP-1 42\"");
        assert_eq!(name_of("HDMI-1 [-1920, 0]"), "HDMI-1");
        assert_eq!(name_of("DP-1 42\""), "DP-1 42\"");
        // A name that merely ends in brackets is left alone
        assert_eq!(name_of("Panel [rev B]"), "Panel [rev B]");
    }

    #[test]
    fn two_identical_screens_are_told_apart_by_position() {
        let v = "MetaVendor [3840, 0]";
        assert!(matches(v, "MetaVendor", 3840, 0));
        assert!(!matches(v, "MetaVendor", 0, 0));
        assert!(matches("MetaVendor", "MetaVendor", 0, 0));
        assert!(!matches(v, "Other", 3840, 0));
    }
}
