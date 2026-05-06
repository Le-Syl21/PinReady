//! Launcher input → action layer.
//!
//! Single source of truth for "what does the user want the launcher to
//! do right now". Keyboard, mouse wheel and joystick all funnel into
//! the same `LauncherAction` enum and the same dispatch routine
//! (`App::apply_launcher_action`). The only desktop-vs-cabinet
//! difference is that the mouse wheel is wired into navigation only
//! when the launcher is in cabinet mode (rotated playfield); on the
//! desktop the wheel keeps its native ScrollArea behaviour.

use egui::{Event, Key, Vec2};
use egui_rotate::Rotation;

/// What the user wants the launcher to do — independent of input
/// source. Joystick/keyboard/wheel all map to one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherAction {
    /// Previous card on the same row (visible-grid-aware).
    PrevCard,
    /// Next card on the same row.
    NextCard,
    /// Same column, row above.
    PrevRow,
    /// Same column, row below.
    NextRow,
    /// Launch the currently-selected table.
    Launch,
    /// Clear the search filter if non-empty, otherwise quit the launcher.
    Cancel,
}

impl LauncherAction {
    /// Translate a VPX input action name (as written in the user's
    /// `[Input] Mapping.<Action> = ...` lines and surfaced by the
    /// joystick thread) to a launcher-level action. Returns `None`
    /// for VPX actions that have no launcher meaning.
    pub fn from_vpx_action(name: &str) -> Option<Self> {
        match name {
            "LeftFlipper" => Some(Self::PrevCard),
            "RightFlipper" => Some(Self::NextCard),
            "LeftMagna" => Some(Self::PrevRow),
            "RightMagna" => Some(Self::NextRow),
            "Start" | "LaunchBall" => Some(Self::Launch),
            "ExitGame" => Some(Self::Cancel),
            _ => None,
        }
    }

    /// True for the four directional actions — these get key-repeat
    /// when held on a joystick.
    pub fn is_directional(self) -> bool {
        matches!(
            self,
            Self::PrevCard | Self::NextCard | Self::PrevRow | Self::NextRow
        )
    }
}

/// Inverse of `egui_rotate::Rotation::transform_vec`. egui-rotate
/// rotates `MouseWheel` deltas into logical UI space before our app
/// sees them; for navigation we want the user's *physical* scroll
/// direction (vertical wheel = vertical row navigation) regardless
/// of how the playfield is rotated.
fn unrotate_vec(rotation: Rotation, v: Vec2) -> Vec2 {
    match rotation {
        Rotation::None => v,
        Rotation::CW90 => Vec2::new(v.y, -v.x),
        Rotation::CW180 => Vec2::new(-v.x, -v.y),
        Rotation::CW270 => Vec2::new(-v.y, v.x),
    }
}

/// Drain the current frame's keyboard + wheel events into a list of
/// launcher actions. Wheel events are only consumed when the launcher
/// is rotated (cabinet mode); on desktop the wheel keeps its native
/// ScrollArea scrolling. Modifier keys (Shift/Ctrl) are suppressed
/// while a text edit has focus so search-bar typing doesn't leak
/// into navigation.
pub fn collect_actions(ui: &egui::Ui, rotation: Rotation) -> Vec<LauncherAction> {
    let cabinet_mode = !rotation.is_none();
    let text_focused = ui.ctx().text_edit_focused();
    ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key {
                    key, pressed: true, ..
                } => match key {
                    Key::ArrowLeft => Some(LauncherAction::PrevCard),
                    Key::ArrowRight => Some(LauncherAction::NextCard),
                    Key::ArrowUp => Some(LauncherAction::PrevRow),
                    Key::ArrowDown => Some(LauncherAction::NextRow),
                    // Pincab keyboard wired to cab buttons — VPX defaults:
                    // LShift = LeftFlipper, RShift = RightFlipper,
                    // LCtrl = LeftMagna, RCtrl = RightMagna.
                    Key::ShiftLeft if !text_focused => Some(LauncherAction::PrevCard),
                    Key::ShiftRight if !text_focused => Some(LauncherAction::NextCard),
                    Key::ControlLeft if !text_focused => Some(LauncherAction::PrevRow),
                    Key::ControlRight if !text_focused => Some(LauncherAction::NextRow),
                    Key::Enter => Some(LauncherAction::Launch),
                    Key::Escape => Some(LauncherAction::Cancel),
                    _ => None,
                },
                Event::MouseWheel { delta, .. } if cabinet_mode => {
                    let phys = unrotate_vec(rotation, *delta);
                    if phys.y > 0.0 {
                        Some(LauncherAction::PrevRow)
                    } else if phys.y < 0.0 {
                        Some(LauncherAction::NextRow)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_vpx_action_known_mappings() {
        assert_eq!(
            LauncherAction::from_vpx_action("LeftFlipper"),
            Some(LauncherAction::PrevCard)
        );
        assert_eq!(
            LauncherAction::from_vpx_action("RightMagna"),
            Some(LauncherAction::NextRow)
        );
        assert_eq!(
            LauncherAction::from_vpx_action("Start"),
            Some(LauncherAction::Launch)
        );
        assert_eq!(
            LauncherAction::from_vpx_action("LaunchBall"),
            Some(LauncherAction::Launch)
        );
        assert_eq!(
            LauncherAction::from_vpx_action("ExitGame"),
            Some(LauncherAction::Cancel)
        );
        assert_eq!(LauncherAction::from_vpx_action("Tilt"), None);
    }

    #[test]
    fn unrotate_inverts_egui_rotate_transform() {
        // egui_rotate::Rotation::transform_vec under CW90: (x, y) → (-y, x).
        // unrotate_vec(CW90, .) must invert that round-trip.
        for &(x, y) in &[(1.0, 0.0), (0.0, 1.0), (1.0, -1.0), (-2.5, 3.5)] {
            let physical = Vec2::new(x, y);
            for rot in [
                Rotation::None,
                Rotation::CW90,
                Rotation::CW180,
                Rotation::CW270,
            ] {
                let logical = rot.transform_vec(physical);
                let recovered = unrotate_vec(rot, logical);
                let ok = (recovered.x - physical.x).abs() < 1e-4
                    && (recovered.y - physical.y).abs() < 1e-4;
                assert!(ok, "rotation {:?} did not round-trip", rot);
            }
        }
    }

    #[test]
    fn directional_actions() {
        assert!(LauncherAction::PrevCard.is_directional());
        assert!(LauncherAction::NextCard.is_directional());
        assert!(LauncherAction::PrevRow.is_directional());
        assert!(LauncherAction::NextRow.is_directional());
        assert!(!LauncherAction::Launch.is_directional());
        assert!(!LauncherAction::Cancel.is_directional());
    }
}
