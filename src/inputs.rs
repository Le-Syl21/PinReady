use rust_i18n::t;
use sdl3_sys::everything::*;
use std::ffi::CStr;
use std::thread;

/// An input action that can be mapped to a key or button.
#[derive(Clone)]
pub struct InputAction {
    pub setting_id: &'static str,
    pub label: &'static str,
    pub default_scancode: SDL_Scancode,
    pub essential: bool,
    pub mapping: Option<CapturedInput>,
}

/// A captured input event (keyboard or joystick).
#[derive(Clone)]
pub enum CapturedInput {
    Keyboard {
        scancode: SDL_Scancode,
        name: String,
    },
    JoystickButton {
        device_id: String,
        button: u8,
        name: String,
    },
    #[allow(dead_code)]
    JoystickAxis {
        device_id: String,
        axis: u8,
        name: String,
    },
}

impl CapturedInput {
    /// Format as VPX ini mapping string.
    pub fn to_mapping_string(&self) -> String {
        match self {
            Self::Keyboard { scancode, .. } => {
                format!("Key;{}", scancode.0)
            }
            Self::JoystickButton {
                device_id, button, ..
            } => {
                format!("{device_id};{button}")
            }
            Self::JoystickAxis {
                device_id, axis, ..
            } => {
                format!("{device_id};{axis}")
            }
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::Keyboard { name, .. }
            | Self::JoystickButton { name, .. }
            | Self::JoystickAxis { name, .. } => name,
        }
    }
}

/// Joystick events sent from the SDL3 joystick thread to the UI.
#[derive(Clone)]
pub enum JoystickEvent {
    ButtonDown {
        device_id: String,
        button: u8,
        name: String,
    },
    ButtonUp {
        #[allow(dead_code)] // carried for symmetry with ButtonDown, reserved for future use
        device_id: String,
        button: u8,
    },
    #[allow(dead_code)]
    AxisMotion {
        device_id: String,
        axis: u8,
        name: String,
    },
    /// Live accelerometer/axis data for visualization (axis_id, normalized value -1.0 to 1.0)
    AccelUpdate { x: f32, y: f32 },
    /// A Pinscape controller was detected with this VPX device ID
    PinscapeDetected { vpx_id: String },
    /// A DudesCab controller was detected with this VPX device ID
    DudesCabDetected { vpx_id: String },
    /// A CSD PinOne controller was detected with this VPX device ID
    PinOneDetected { vpx_id: String },
    /// A generic gamepad was detected with this VPX device ID
    GamepadDetected { vpx_id: String, name: String },
}

/// Build the VPX-compatible device ID for an SDL joystick.
/// Uses serial (`SDLJoy_PSC...`) if available, else GUID.
/// Single ID used for both buttons and axes — VPX handles the rest.
unsafe fn vpx_device_id(joy: *mut sdl3_sys::everything::SDL_Joystick) -> String {
    let serial_ptr = SDL_GetJoystickSerial(joy);
    if !serial_ptr.is_null() {
        let serial = CStr::from_ptr(serial_ptr).to_string_lossy();
        if !serial.is_empty() {
            return format!("SDLJoy_{serial}");
        }
    }
    // Fallback: use GUID as hex string
    let guid = SDL_GetJoystickGUID(joy);
    let mut buf = [0u8; 64];
    SDL_GUIDToString(guid, buf.as_mut_ptr() as *mut _, buf.len() as i32);
    let guid_str = CStr::from_ptr(buf.as_ptr() as *const _).to_string_lossy();
    format!("SDLJoy_{guid_str}")
}

/// Get a human-readable label for an SDL scancode, localised both to
/// the user's current keyboard layout (character keys) and to their UI
/// language (named keys like Escape / Home / Shift).
///
/// Two passes:
///   1. `SDL_GetKeyName(SDL_GetKeyFromScancode(...))` — the standard SDL
///      layout-aware name. A FR AZERTY user sees `!` / `:` / `<`
///      directly for the character keys, instead of the US QWERTY
///      labels `/` / `.` / `NonUSBackslash`. VPX uses the same call.
///   2. `localize_key_name(...)` — post-process to translate the
///      English-only *named* keys SDL returns ("Escape", "Left Shift",
///      etc.) via rust-i18n. Character keys fall through unchanged.
///
/// Falls back to the raw scancode name, then to a numeric form, so we
/// always return something the UI can show.
pub fn scancode_name(scancode: SDL_Scancode) -> String {
    let raw = unsafe {
        let keycode = SDL_GetKeyFromScancode(scancode, SDL_KMOD_NONE, false);
        let mut out = String::new();
        if keycode.0 != 0 {
            let name_ptr = SDL_GetKeyName(keycode);
            if !name_ptr.is_null() {
                let s = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
                if !s.is_empty() {
                    out = s;
                }
            }
        }
        if out.is_empty() {
            // Fallback: physical-position label (`Comma`,
            // `NonUSBackslash`…) when no layout character is available.
            let name_ptr = SDL_GetScancodeName(scancode);
            if !name_ptr.is_null() {
                let s = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
                if !s.is_empty() {
                    out = s;
                }
            }
        }
        out
    };

    if raw.is_empty() {
        return format!("Key {}", scancode.0);
    }
    localize_key_name(&raw)
}

/// Translate the English named keys SDL returns (e.g. "Escape", "Home",
/// "Left Shift") via rust-i18n. Character keys and unknown strings pass
/// through unchanged so the user still sees `!`, `:`, `<`, etc. on
/// non-US layouts.
fn localize_key_name(raw: &str) -> String {
    // Arrow keys — prefer symbols, universal across locales.
    match raw {
        "Left" => return "←".to_string(),
        "Right" => return "→".to_string(),
        "Up" => return "↑".to_string(),
        "Down" => return "↓".to_string(),
        _ => {}
    }
    let key = match raw {
        "Escape" => "key_escape",
        "Tab" => "key_tab",
        "Space" => "key_space",
        "Return" => "key_return",
        "Backspace" => "key_backspace",
        "Delete" => "key_delete",
        "Insert" => "key_insert",
        "Home" => "key_home",
        "End" => "key_end",
        "PageUp" => "key_pageup",
        "PageDown" => "key_pagedown",
        "CapsLock" => "key_capslock",
        "NumLockClear" => "key_numlock",
        "ScrollLock" => "key_scrolllock",
        "PrintScreen" => "key_printscreen",
        "Pause" => "key_pause",
        "Left Shift" => "key_lshift",
        "Right Shift" => "key_rshift",
        "Left Ctrl" => "key_lctrl",
        "Right Ctrl" => "key_rctrl",
        "Left Alt" => "key_lalt",
        "Right Alt" => "key_ralt",
        "Left GUI" => "key_lgui",
        "Right GUI" => "key_rgui",
        "NonUSBackslash" => "key_nonusbackslash",
        "Application" => "key_application",
        _ => return raw.to_string(),
    };
    t!(key).to_string()
}

/// Convert an egui::Key to an SDL_Scancode.
/// This allows capturing keyboard input via egui (which has the window focus)
/// instead of a separate SDL3 thread.
pub fn egui_key_to_scancode(key: egui::Key) -> Option<SDL_Scancode> {
    use egui::Key;
    let sc = match key {
        Key::A => SDL_SCANCODE_A,
        Key::B => SDL_SCANCODE_B,
        Key::C => SDL_SCANCODE_C,
        Key::D => SDL_SCANCODE_D,
        Key::E => SDL_SCANCODE_E,
        Key::F => SDL_SCANCODE_F,
        Key::G => SDL_SCANCODE_G,
        Key::H => SDL_SCANCODE_H,
        Key::I => SDL_SCANCODE_I,
        Key::J => SDL_SCANCODE_J,
        Key::K => SDL_SCANCODE_K,
        Key::L => SDL_SCANCODE_L,
        Key::M => SDL_SCANCODE_M,
        Key::N => SDL_SCANCODE_N,
        Key::O => SDL_SCANCODE_O,
        Key::P => SDL_SCANCODE_P,
        Key::Q => SDL_SCANCODE_Q,
        Key::R => SDL_SCANCODE_R,
        Key::S => SDL_SCANCODE_S,
        Key::T => SDL_SCANCODE_T,
        Key::U => SDL_SCANCODE_U,
        Key::V => SDL_SCANCODE_V,
        Key::W => SDL_SCANCODE_W,
        Key::X => SDL_SCANCODE_X,
        Key::Y => SDL_SCANCODE_Y,
        Key::Z => SDL_SCANCODE_Z,
        Key::Num0 => SDL_SCANCODE_0,
        Key::Num1 => SDL_SCANCODE_1,
        Key::Num2 => SDL_SCANCODE_2,
        Key::Num3 => SDL_SCANCODE_3,
        Key::Num4 => SDL_SCANCODE_4,
        Key::Num5 => SDL_SCANCODE_5,
        Key::Num6 => SDL_SCANCODE_6,
        Key::Num7 => SDL_SCANCODE_7,
        Key::Num8 => SDL_SCANCODE_8,
        Key::Num9 => SDL_SCANCODE_9,
        Key::Escape => SDL_SCANCODE_ESCAPE,
        Key::Tab => SDL_SCANCODE_TAB,
        Key::Space => SDL_SCANCODE_SPACE,
        Key::Enter => SDL_SCANCODE_RETURN,
        Key::Backspace => SDL_SCANCODE_BACKSPACE,
        Key::Delete => SDL_SCANCODE_DELETE,
        Key::Home => SDL_SCANCODE_HOME,
        Key::End => SDL_SCANCODE_END,
        Key::PageUp => SDL_SCANCODE_PAGEUP,
        Key::PageDown => SDL_SCANCODE_PAGEDOWN,
        Key::ArrowUp => SDL_SCANCODE_UP,
        Key::ArrowDown => SDL_SCANCODE_DOWN,
        Key::ArrowLeft => SDL_SCANCODE_LEFT,
        Key::ArrowRight => SDL_SCANCODE_RIGHT,
        Key::F1 => SDL_SCANCODE_F1,
        Key::F2 => SDL_SCANCODE_F2,
        Key::F3 => SDL_SCANCODE_F3,
        Key::F4 => SDL_SCANCODE_F4,
        Key::F5 => SDL_SCANCODE_F5,
        Key::F6 => SDL_SCANCODE_F6,
        Key::F7 => SDL_SCANCODE_F7,
        Key::F8 => SDL_SCANCODE_F8,
        Key::F9 => SDL_SCANCODE_F9,
        Key::F10 => SDL_SCANCODE_F10,
        Key::F11 => SDL_SCANCODE_F11,
        Key::F12 => SDL_SCANCODE_F12,
        // Punctuation — physical positions on a US QWERTY keyboard.
        // These are what egui delivers via `physical_key` regardless of
        // keyboard layout.
        Key::Minus => SDL_SCANCODE_MINUS,
        Key::Equals => SDL_SCANCODE_EQUALS,
        Key::Comma => SDL_SCANCODE_COMMA,
        Key::Period => SDL_SCANCODE_PERIOD,
        Key::Slash => SDL_SCANCODE_SLASH,
        Key::Semicolon => SDL_SCANCODE_SEMICOLON,
        Key::Quote => SDL_SCANCODE_APOSTROPHE,
        Key::Backslash => SDL_SCANCODE_BACKSLASH,
        Key::Backtick => SDL_SCANCODE_GRAVE,
        Key::OpenBracket => SDL_SCANCODE_LEFTBRACKET,
        Key::CloseBracket => SDL_SCANCODE_RIGHTBRACKET,
        // Shifted-logical fallbacks — if for some reason only the logical
        // `key` is available (no physical), map to the un-shifted physical
        // scancode of the key that produces that character on US QWERTY.
        // Example: user pressed Shift+1 → logical key is Exclamationmark,
        // the underlying physical key is Num1 → SDL_SCANCODE_1.
        Key::Exclamationmark => SDL_SCANCODE_1,
        Key::Questionmark => SDL_SCANCODE_SLASH,
        Key::Pipe => SDL_SCANCODE_BACKSLASH,
        Key::Colon => SDL_SCANCODE_SEMICOLON,
        Key::Plus => SDL_SCANCODE_EQUALS,
        Key::OpenCurlyBracket => SDL_SCANCODE_LEFTBRACKET,
        Key::CloseCurlyBracket => SDL_SCANCODE_RIGHTBRACKET,

        // Physical L/R modifier keys — our egui fork surfaces these
        // distinctly so we no longer need device_query to resolve the
        // side from global OS state.
        Key::ShiftLeft => SDL_SCANCODE_LSHIFT,
        Key::ShiftRight => SDL_SCANCODE_RSHIFT,
        Key::ControlLeft => SDL_SCANCODE_LCTRL,
        Key::ControlRight => SDL_SCANCODE_RCTRL,
        Key::AltLeft => SDL_SCANCODE_LALT,
        Key::AltRight => SDL_SCANCODE_RALT,
        Key::SuperLeft => SDL_SCANCODE_LGUI,
        Key::SuperRight => SDL_SCANCODE_RGUI,

        // ISO 102nd key — FR AZERTY `<>|`, UK `\|`.
        Key::IntlBackslash => SDL_SCANCODE_NONUSBACKSLASH,

        _ => return None,
    };
    Some(sc)
}

/// Check for modifier keys pressed in egui and return the matching SDL
/// scancode. Kept as a fallback for the case where the capture code
/// sees `modifiers.shift/ctrl/alt` set but no `Event::Key` with the
/// corresponding `Key::ShiftLeft/Right/...` reached us (e.g. a very old
/// winit build, or a platform where physical keys aren't surfaced).
/// With the current egui fork, `Event::Key` should always carry the
/// distinct side, so this path is rarely hit — when it is, we default
/// to the left variant.
pub fn egui_modifiers_to_scancode(modifiers: &egui::Modifiers) -> Option<SDL_Scancode> {
    if modifiers.shift && !modifiers.ctrl && !modifiers.alt {
        Some(SDL_SCANCODE_LSHIFT)
    } else if modifiers.ctrl && !modifiers.shift && !modifiers.alt {
        Some(SDL_SCANCODE_LCTRL)
    } else if modifiers.alt && !modifiers.shift && !modifiers.ctrl {
        Some(SDL_SCANCODE_LALT)
    } else {
        None
    }
}

/// Build the list of all mappable actions with their defaults.
pub fn default_actions() -> Vec<InputAction> {
    vec![
        // Essential — flippers, staged, magna, game commands
        InputAction {
            setting_id: "LeftFlipper",
            label: "input_left_flipper",
            default_scancode: SDL_SCANCODE_LSHIFT,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "RightFlipper",
            label: "input_right_flipper",
            default_scancode: SDL_SCANCODE_RSHIFT,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "LeftStagedFlipper",
            label: "input_left_staged_flipper",
            default_scancode: SDL_SCANCODE_LSHIFT,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "RightStagedFlipper",
            label: "input_right_staged_flipper",
            default_scancode: SDL_SCANCODE_RSHIFT,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "LeftMagna",
            label: "input_left_magna",
            default_scancode: SDL_SCANCODE_LCTRL,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "RightMagna",
            label: "input_right_magna",
            default_scancode: SDL_SCANCODE_RCTRL,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "Lockbar",
            label: "input_lockbar",
            default_scancode: SDL_SCANCODE_LALT,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "ExtraBall",
            label: "input_extra_ball",
            default_scancode: SDL_SCANCODE_B,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "LaunchBall",
            label: "input_launch_ball",
            default_scancode: SDL_SCANCODE_RETURN,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "Start",
            label: "input_start",
            default_scancode: SDL_SCANCODE_1,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "Credit1",
            label: "input_add_credit",
            default_scancode: SDL_SCANCODE_5,
            essential: true,
            mapping: None,
        },
        InputAction {
            setting_id: "ExitGame",
            label: "input_exit_game",
            default_scancode: SDL_SCANCODE_ESCAPE,
            essential: true,
            mapping: None,
        },
        // Advanced — coin door, services, keyboard nudge, etc.
        InputAction {
            setting_id: "Credit2",
            label: "input_credit_2",
            default_scancode: SDL_SCANCODE_4,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Credit3",
            label: "input_credit_3",
            default_scancode: SDL_SCANCODE_3,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Credit4",
            label: "input_credit_4",
            default_scancode: SDL_SCANCODE_6,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "CoinDoor",
            label: "input_coin_door",
            default_scancode: SDL_SCANCODE_END,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "SlamTilt",
            label: "input_slam_tilt",
            default_scancode: SDL_SCANCODE_HOME,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Reset",
            label: "input_reset",
            default_scancode: SDL_SCANCODE_F3,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service1",
            label: "input_service1",
            default_scancode: SDL_SCANCODE_7,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service2",
            label: "input_service2",
            default_scancode: SDL_SCANCODE_8,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service3",
            label: "input_service3",
            default_scancode: SDL_SCANCODE_9,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service4",
            label: "input_service4",
            default_scancode: SDL_SCANCODE_0,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service5",
            label: "input_service5",
            default_scancode: SDL_SCANCODE_6,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service6",
            label: "input_service6",
            default_scancode: SDL_SCANCODE_PAGEUP,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service7",
            label: "input_service7",
            default_scancode: SDL_SCANCODE_MINUS,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Service8",
            label: "input_service8",
            default_scancode: SDL_SCANCODE_UNKNOWN,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "LeftNudge",
            label: "input_left_nudge",
            default_scancode: SDL_SCANCODE_Z,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "RightNudge",
            label: "input_right_nudge",
            default_scancode: SDL_SCANCODE_SLASH,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "CenterNudge",
            label: "input_center_nudge",
            default_scancode: SDL_SCANCODE_SPACE,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Tilt",
            label: "input_tilt",
            default_scancode: SDL_SCANCODE_T,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Pause",
            label: "input_pause",
            default_scancode: SDL_SCANCODE_P,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "VolumeDown",
            label: "input_volume_down",
            default_scancode: SDL_SCANCODE_MINUS,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "VolumeUp",
            label: "input_volume_up",
            default_scancode: SDL_SCANCODE_EQUALS,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Custom1",
            label: "input_custom1",
            default_scancode: SDL_SCANCODE_UNKNOWN,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Custom2",
            label: "input_custom2",
            default_scancode: SDL_SCANCODE_UNKNOWN,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Custom3",
            label: "input_custom3",
            default_scancode: SDL_SCANCODE_UNKNOWN,
            essential: false,
            mapping: None,
        },
        InputAction {
            setting_id: "Custom4",
            label: "input_custom4",
            default_scancode: SDL_SCANCODE_UNKNOWN,
            essential: false,
            mapping: None,
        },
    ]
}

/// Pinball controller profile names for UI display.
pub const PINSCAPE_PROFILES: &[&str] = &[
    "KL25Z (KL Shield / Brain / Rig Master)",
    "Pico (OpenPinballDevice)",
    "DudesCab (Arnoz)",
    "CSD PinOne",
];

/// Default button mappings for each Pinscape profile.
/// Returns (action_setting_id, button_number) pairs.
///
/// KL25Z mapping verified with jstest on KL Shield V5.1 (Arnoz default config):
/// 0=START, 1=EXTRA-B, 2=COIN1, 3=COIN2, 4=L BALL, 5=EXIT, 6=QUIT,
/// 7=L FLIPP, 8=R FLIPP, 9=L MAGNA, 10=R MAGNA, 11=FIRE, 12=TILT,
/// 13=DOOR, 14=SERVICE EXIT, 15=SERVICE -, 16=SERVICE +, 17=ENTER,
/// 18=N.M., 19=VOL-, 20=VOL+
pub fn pinscape_button_defaults(profile: usize) -> &'static [(&'static str, u8)] {
    match profile {
        // KL25Z with KL Shield V5.1 / Brain / Rig Master (Arnoz default config)
        // Verified via jstest on physical hardware
        0 => &[
            ("Start", 0),
            ("ExtraBall", 1),
            ("Credit1", 2),
            ("Credit2", 3),
            ("LaunchBall", 4),
            ("ExitGame", 5),
            // 6 = QUIT (VP editor only, not mapped in game)
            ("LeftFlipper", 7),
            ("RightFlipper", 8),
            ("LeftMagna", 9),
            ("RightMagna", 10),
            ("LeftStagedFlipper", 7), // same physical button as flipper (double switch stack)
            ("RightStagedFlipper", 8), // same physical button as flipper (double switch stack)
            ("Lockbar", 11),
            ("Tilt", 12),
            ("CoinDoor", 13),
            ("Service1", 14),
            ("Service2", 15),
            ("Service3", 16),
            ("Service4", 17),
            // 18 = Night Mode (not a VPX action)
            ("VolumeDown", 19),
            ("VolumeUp", 20),
        ],
        // Pinscape Pico — OpenPinballDevice standard
        1 => &[
            ("Start", 0),
            ("ExitGame", 1),
            ("ExtraBall", 2),
            ("Credit1", 3),
            ("Credit2", 4),
            ("Credit3", 5),
            ("Credit4", 6),
            ("LaunchBall", 7),
            ("Lockbar", 8),
            ("LeftFlipper", 9),
            ("RightFlipper", 10),
            ("LeftStagedFlipper", 11),
            ("RightStagedFlipper", 12),
            ("LeftMagna", 13),
            ("RightMagna", 14),
            ("Tilt", 15),
            ("SlamTilt", 16),
            ("CoinDoor", 17),
            ("Service1", 18),
            ("Service2", 19),
            ("Service3", 20),
            ("Service4", 21),
            ("LeftNudge", 22),
            ("CenterNudge", 23),
            ("RightNudge", 24),
            ("VolumeUp", 25),
            ("VolumeDown", 26),
        ],
        // DudesCab (Arnoz) — from official mapping table
        // Note: DudesCab numbers buttons from 1, SDL3 from 0, so btn_gamepad - 1 = SDL button
        2 => &[
            ("Start", 0),      // Gamepad 1
            ("ExtraBall", 1),  // Gamepad 2
            ("Credit1", 2),    // Gamepad 3
            ("Credit2", 3),    // Gamepad 4
            ("LaunchBall", 4), // Gamepad 5
            ("ExitGame", 5),   // Gamepad 6 (Return = exit to menu)
            // 6 = Exit/Quit to editor (X key) — not mapped in game
            ("LeftFlipper", 7),        // Gamepad 8
            ("RightFlipper", 8),       // Gamepad 9
            ("LeftMagna", 9),          // Gamepad 10
            ("RightMagna", 10),        // Gamepad 11
            ("LeftStagedFlipper", 7),  // same physical button as flipper
            ("RightStagedFlipper", 8), // same physical button as flipper
            ("Tilt", 11),              // Gamepad 12
            ("Lockbar", 12),           // Gamepad 13 (Fire)
            ("CoinDoor", 13),          // Gamepad 14
            ("Service1", 14),          // Gamepad 15 (ROM Exit)
            ("Service2", 15),          // Gamepad 16 (ROM -)
            ("Service3", 16),          // Gamepad 17 (ROM +)
            ("Service4", 17),          // Gamepad 18 (ROM Enter)
            ("VolumeDown", 18),        // Gamepad 19
            ("VolumeUp", 19),          // Gamepad 20
                                       // 20-23 = DPAD (handled as hat, not buttons)
                                       // 24 = NightMode (DO NOT REMAP)
                                       // 25-30 = Spare 1-6
                                       // 31 = Calib (DO NOT REMAP)
        ],
        // CSD PinOne — from VPX calibration screenshot (VPX buttons are 1-indexed, SDL 0-indexed)
        // Device identifies as "Clev Soft PinOne" (VID 0x0E8F, PID 0x0792)
        // Axes: X=nudge L/R (Accel), Y=nudge U/D (Accel), Z=plunger (Position)
        _ => &[
            ("RightFlipper", 0), // VPX Button 1
            ("RightMagna", 1),   // VPX Button 2
            ("LeftFlipper", 2),  // VPX Button 3
            ("LeftMagna", 3),    // VPX Button 4
            ("ExtraBall", 4),    // VPX Button 5 (EB BuyIn)
            ("Start", 5),        // VPX Button 6
            ("Credit1", 6),      // VPX Button 7
            ("ExitGame", 7),     // VPX Button 8
            ("Lockbar", 8),      // VPX Button 9 (Menu / Fire)
            ("VolumeUp", 15),    // VPX Button 16
            ("VolumeDown", 16),  // VPX Button 17
            ("CoinDoor", 17),    // VPX Button 18
            ("Service1", 18),    // VPX Button 19 (Cancel)
            ("Service2", 19),    // VPX Button 20 (Down)
            ("Service3", 20),    // VPX Button 21 (Up)
            ("Service4", 21),    // VPX Button 22 (Enter)
            ("Tilt", 22),        // VPX Button 23 (Mech Tilt)
            ("LaunchBall", 23),  // VPX Button 24 (Plunger digital)
        ],
    }
}

/// Check if any two actions share the same key binding.
/// Pairs of action ids that are *expected* to share the same physical
/// key — pincab staged flippers are literally a second switch on the
/// same flipper button stack, so the default mapping collides with the
/// flipper by design. Skip these in conflict reporting so the wizard
/// doesn't flag them as errors.
const INTENTIONAL_COLLISIONS: &[(&str, &str)] = &[
    ("LeftFlipper", "LeftStagedFlipper"),
    ("RightFlipper", "RightStagedFlipper"),
];

fn is_intentional_collision(a: &str, b: &str) -> bool {
    INTENTIONAL_COLLISIONS
        .iter()
        .any(|&(x, y)| (a == x && b == y) || (a == y && b == x))
}

pub fn find_conflicts(actions: &[InputAction]) -> Vec<(usize, usize)> {
    let mut conflicts = Vec::new();
    for i in 0..actions.len() {
        for j in (i + 1)..actions.len() {
            let a = effective_scancode(&actions[i]);
            let b = effective_scancode(&actions[j]);
            if a != SDL_SCANCODE_UNKNOWN
                && a == b
                && !is_intentional_collision(actions[i].setting_id, actions[j].setting_id)
            {
                conflicts.push((i, j));
            }
        }
    }
    conflicts
}

pub(crate) fn effective_scancode(action: &InputAction) -> SDL_Scancode {
    match &action.mapping {
        Some(CapturedInput::Keyboard { scancode, .. }) => *scancode,
        None => action.default_scancode,
        _ => SDL_SCANCODE_UNKNOWN,
    }
}

/// Info about an opened joystick for direct polling.
struct OpenedJoystick {
    handle: *mut sdl3_sys::everything::SDL_Joystick,
    vpx_id: String,
    num_buttons: i32,
    num_axes: i32,
    prev_buttons: Vec<bool>,
    /// Pre-built button names to avoid format!() in the 100Hz polling loop
    button_names: Vec<String>,
    /// Pre-built axis names
    axis_names: Vec<String>,
}

/// Spawn the SDL3 joystick polling thread (keyboard is handled via egui).
/// Uses direct state polling (SDL_GetJoystickButton/Axis) instead of SDL_PollEvent
/// to avoid the SDL3 main-thread assertion on event pumping.
/// Returns a receiver for joystick events.
pub fn spawn_joystick_thread() -> crossbeam_channel::Receiver<JoystickEvent> {
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<JoystickEvent>();

    thread::spawn(move || {
        unsafe {
            // Init joystick subsystem in this thread
            if !SDL_InitSubSystem(SDL_INIT_JOYSTICK) {
                log::error!(
                    "Joystick thread: SDL_InitSubSystem failed: {:?}",
                    CStr::from_ptr(SDL_GetError())
                );
                return;
            }

            // Open all connected joysticks
            let mut joy_count: i32 = 0;
            let joy_ids = SDL_GetJoysticks(&mut joy_count);
            let mut joysticks: Vec<OpenedJoystick> = Vec::with_capacity(joy_count as usize);
            if !joy_ids.is_null() {
                for i in 0..joy_count as usize {
                    let jid = *joy_ids.add(i);
                    let joy = SDL_OpenJoystick(jid);
                    if !joy.is_null() {
                        let name_ptr = SDL_GetJoystickName(joy);
                        let name = if !name_ptr.is_null() {
                            CStr::from_ptr(name_ptr).to_string_lossy().into_owned()
                        } else {
                            format!("Joystick {}", jid.0)
                        };
                        let dev_id = vpx_device_id(joy);
                        let num_buttons = SDL_GetNumJoystickButtons(joy);
                        let num_axes = SDL_GetNumJoystickAxes(joy);
                        log::info!(
                            "Opened joystick: {} (vpx_id={}, buttons={}, axes={})",
                            name,
                            dev_id,
                            num_buttons,
                            num_axes
                        );
                        // Detect pinball controllers
                        if name.contains("Pinscape") || dev_id.contains("PSC") {
                            log::info!("Pinscape controller detected: {}", dev_id);
                            let _ = evt_tx.send(JoystickEvent::PinscapeDetected {
                                vpx_id: dev_id.clone(),
                            });
                        } else if name.contains("DudesCab") {
                            log::info!("DudesCab controller detected: {}", dev_id);
                            let _ = evt_tx.send(JoystickEvent::DudesCabDetected {
                                vpx_id: dev_id.clone(),
                            });
                        } else if name.contains("PinOne") {
                            log::info!("CSD PinOne controller detected: {}", dev_id);
                            let _ = evt_tx.send(JoystickEvent::PinOneDetected {
                                vpx_id: dev_id.clone(),
                            });
                        } else if SDL_IsGamepad(jid) {
                            // Generic gamepad (Xbox, PS, etc.)
                            log::info!("Gamepad detected: {} ({})", name, dev_id);
                            let _ = evt_tx.send(JoystickEvent::GamepadDetected {
                                vpx_id: dev_id.clone(),
                                name: name.clone(),
                            });
                        }
                        // Pre-build names to avoid format!() in the hot polling loop
                        let button_names: Vec<String> = (0..num_buttons)
                            .map(|b| format!("{} Button {}", dev_id, b))
                            .collect();
                        let axis_names: Vec<String> = (0..num_axes)
                            .map(|a| format!("{} Axis {}", dev_id, a))
                            .collect();
                        joysticks.push(OpenedJoystick {
                            handle: joy,
                            vpx_id: dev_id,
                            num_buttons,
                            num_axes,
                            prev_buttons: vec![false; num_buttons as usize],
                            button_names,
                            axis_names,
                        });
                    }
                }
                SDL_free(joy_ids as *mut _);
            }
            log::info!(
                "Joystick thread started, {} joystick(s) found",
                joysticks.len()
            );

            loop {
                // Update joystick state (required before reading)
                SDL_UpdateJoysticks();

                for js in &mut joysticks {
                    // Poll buttons — detect newly pressed (only allocate on actual press)
                    for b in 0..js.num_buttons {
                        let pressed = SDL_GetJoystickButton(js.handle, b);
                        let idx = b as usize;
                        let was_pressed = js.prev_buttons[idx];
                        if pressed && !was_pressed {
                            let _ = evt_tx.send(JoystickEvent::ButtonDown {
                                device_id: js.vpx_id.clone(),
                                button: b as u8,
                                name: js.button_names[idx].clone(),
                            });
                        } else if !pressed && was_pressed {
                            let _ = evt_tx.send(JoystickEvent::ButtonUp {
                                device_id: js.vpx_id.clone(),
                                button: b as u8,
                            });
                        }
                        js.prev_buttons[idx] = pressed;
                    }

                    // Poll axes — capture big movements for input binding
                    for a in 0..js.num_axes {
                        let value = SDL_GetJoystickAxis(js.handle, a);
                        if value.abs() > 16000 {
                            let _ = evt_tx.send(JoystickEvent::AxisMotion {
                                device_id: js.vpx_id.clone(),
                                axis: a as u8,
                                name: js.axis_names[a as usize].clone(),
                            });
                        }
                    }

                    // Send raw normalized accel data for tilt visualization (axes 0+1)
                    if js.num_axes >= 2 {
                        let ax = SDL_GetJoystickAxis(js.handle, 0) as f32 / 32767.0;
                        let ay = SDL_GetJoystickAxis(js.handle, 1) as f32 / 32767.0;
                        let _ = evt_tx.send(JoystickEvent::AccelUpdate { x: ax, y: ay });
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    });

    evt_rx
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- CapturedInput::to_mapping_string ---

    #[test]
    fn keyboard_mapping_string() {
        let input = CapturedInput::Keyboard {
            scancode: SDL_SCANCODE_LSHIFT,
            name: "Left Shift".to_string(),
        };
        assert_eq!(
            input.to_mapping_string(),
            format!("Key;{}", SDL_SCANCODE_LSHIFT.0)
        );
    }

    #[test]
    fn joystick_button_mapping_string() {
        let input = CapturedInput::JoystickButton {
            device_id: "SDLJoy_PSC004".to_string(),
            button: 7,
            name: "Button 7".to_string(),
        };
        assert_eq!(input.to_mapping_string(), "SDLJoy_PSC004;7");
    }

    #[test]
    fn joystick_axis_mapping_string() {
        let input = CapturedInput::JoystickAxis {
            device_id: "SDLJoy_ABC".to_string(),
            axis: 2,
            name: "Axis 2".to_string(),
        };
        assert_eq!(input.to_mapping_string(), "SDLJoy_ABC;2");
    }

    // --- CapturedInput::display_name ---

    #[test]
    fn display_name_keyboard() {
        let input = CapturedInput::Keyboard {
            scancode: SDL_SCANCODE_A,
            name: "A".to_string(),
        };
        assert_eq!(input.display_name(), "A");
    }

    #[test]
    fn display_name_joystick() {
        let input = CapturedInput::JoystickButton {
            device_id: "dev".to_string(),
            button: 0,
            name: "Start Button".to_string(),
        };
        assert_eq!(input.display_name(), "Start Button");
    }

    // --- default_actions ---

    #[test]
    fn default_actions_has_essential_and_advanced() {
        let actions = default_actions();
        assert!(!actions.is_empty());
        let essential_count = actions.iter().filter(|a| a.essential).count();
        let advanced_count = actions.iter().filter(|a| !a.essential).count();
        assert!(
            essential_count >= 10,
            "expected >=10 essential actions, got {essential_count}"
        );
        assert!(
            advanced_count >= 10,
            "expected >=10 advanced actions, got {advanced_count}"
        );
    }

    #[test]
    fn default_actions_all_have_setting_id() {
        for action in default_actions() {
            assert!(!action.setting_id.is_empty(), "action has empty setting_id");
        }
    }

    #[test]
    fn default_actions_none_have_mapping() {
        for action in default_actions() {
            assert!(
                action.mapping.is_none(),
                "{} should have no mapping by default",
                action.setting_id
            );
        }
    }

    // --- effective_scancode ---

    #[test]
    fn effective_scancode_default() {
        let action = InputAction {
            setting_id: "Test",
            label: "test",
            default_scancode: SDL_SCANCODE_A,
            essential: true,
            mapping: None,
        };
        assert!(effective_scancode(&action) == SDL_SCANCODE_A);
    }

    #[test]
    fn effective_scancode_with_keyboard_mapping() {
        let action = InputAction {
            setting_id: "Test",
            label: "test",
            default_scancode: SDL_SCANCODE_A,
            essential: true,
            mapping: Some(CapturedInput::Keyboard {
                scancode: SDL_SCANCODE_B,
                name: "B".to_string(),
            }),
        };
        assert!(effective_scancode(&action) == SDL_SCANCODE_B);
    }

    #[test]
    fn effective_scancode_with_joystick_is_unknown() {
        let action = InputAction {
            setting_id: "Test",
            label: "test",
            default_scancode: SDL_SCANCODE_A,
            essential: true,
            mapping: Some(CapturedInput::JoystickButton {
                device_id: "dev".to_string(),
                button: 0,
                name: "B0".to_string(),
            }),
        };
        assert!(effective_scancode(&action) == SDL_SCANCODE_UNKNOWN);
    }

    // --- find_conflicts ---

    #[test]
    fn find_conflicts_no_conflicts() {
        let actions = vec![
            InputAction {
                setting_id: "A",
                label: "a",
                default_scancode: SDL_SCANCODE_A,
                essential: true,
                mapping: None,
            },
            InputAction {
                setting_id: "B",
                label: "b",
                default_scancode: SDL_SCANCODE_B,
                essential: true,
                mapping: None,
            },
        ];
        assert!(find_conflicts(&actions).is_empty());
    }

    #[test]
    fn find_conflicts_detects_default_conflict() {
        let actions = vec![
            InputAction {
                setting_id: "A",
                label: "a",
                default_scancode: SDL_SCANCODE_LSHIFT,
                essential: true,
                mapping: None,
            },
            InputAction {
                setting_id: "B",
                label: "b",
                default_scancode: SDL_SCANCODE_LSHIFT,
                essential: true,
                mapping: None,
            },
        ];
        let conflicts = find_conflicts(&actions);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0], (0, 1));
    }

    #[test]
    fn find_conflicts_unknown_scancode_not_conflicting() {
        let actions = vec![
            InputAction {
                setting_id: "A",
                label: "a",
                default_scancode: SDL_SCANCODE_UNKNOWN,
                essential: false,
                mapping: None,
            },
            InputAction {
                setting_id: "B",
                label: "b",
                default_scancode: SDL_SCANCODE_UNKNOWN,
                essential: false,
                mapping: None,
            },
        ];
        assert!(find_conflicts(&actions).is_empty());
    }

    #[test]
    fn find_conflicts_with_mapped_override() {
        let actions = vec![
            InputAction {
                setting_id: "A",
                label: "a",
                default_scancode: SDL_SCANCODE_A,
                essential: true,
                mapping: Some(CapturedInput::Keyboard {
                    scancode: SDL_SCANCODE_Z,
                    name: "Z".to_string(),
                }),
            },
            InputAction {
                setting_id: "B",
                label: "b",
                default_scancode: SDL_SCANCODE_Z,
                essential: true,
                mapping: None,
            },
        ];
        let conflicts = find_conflicts(&actions);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn find_conflicts_joystick_mapping_no_conflict_with_keyboard() {
        let actions = vec![
            InputAction {
                setting_id: "A",
                label: "a",
                default_scancode: SDL_SCANCODE_A,
                essential: true,
                mapping: Some(CapturedInput::JoystickButton {
                    device_id: "dev".to_string(),
                    button: 0,
                    name: "B0".to_string(),
                }),
            },
            InputAction {
                setting_id: "B",
                label: "b",
                default_scancode: SDL_SCANCODE_A,
                essential: true,
                mapping: None,
            },
        ];
        // A uses joystick (effective = UNKNOWN), B uses keyboard A → no conflict
        assert!(find_conflicts(&actions).is_empty());
    }

    // --- egui_key_to_scancode ---

    #[test]
    fn egui_key_letters() {
        assert!(egui_key_to_scancode(egui::Key::A) == Some(SDL_SCANCODE_A));
        assert!(egui_key_to_scancode(egui::Key::Z) == Some(SDL_SCANCODE_Z));
    }

    #[test]
    fn egui_key_numbers() {
        assert!(egui_key_to_scancode(egui::Key::Num0) == Some(SDL_SCANCODE_0));
        assert!(egui_key_to_scancode(egui::Key::Num9) == Some(SDL_SCANCODE_9));
    }

    #[test]
    fn egui_key_special() {
        assert!(egui_key_to_scancode(egui::Key::Escape) == Some(SDL_SCANCODE_ESCAPE));
        assert!(egui_key_to_scancode(egui::Key::Enter) == Some(SDL_SCANCODE_RETURN));
        assert!(egui_key_to_scancode(egui::Key::Space) == Some(SDL_SCANCODE_SPACE));
    }

    #[test]
    fn egui_key_function_keys() {
        assert!(egui_key_to_scancode(egui::Key::F1) == Some(SDL_SCANCODE_F1));
        assert!(egui_key_to_scancode(egui::Key::F12) == Some(SDL_SCANCODE_F12));
    }

    #[test]
    fn egui_key_unmapped_returns_none() {
        assert!(egui_key_to_scancode(egui::Key::Insert).is_none());
    }

    // --- egui_modifiers_to_scancode ---

    #[test]
    fn modifiers_shift_only() {
        let m = egui::Modifiers {
            shift: true,
            ctrl: false,
            alt: false,
            ..Default::default()
        };
        assert!(egui_modifiers_to_scancode(&m) == Some(SDL_SCANCODE_LSHIFT));
    }

    #[test]
    fn modifiers_ctrl_only() {
        let m = egui::Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            ..Default::default()
        };
        assert!(egui_modifiers_to_scancode(&m) == Some(SDL_SCANCODE_LCTRL));
    }

    #[test]
    fn modifiers_alt_only() {
        let m = egui::Modifiers {
            alt: true,
            shift: false,
            ctrl: false,
            ..Default::default()
        };
        assert!(egui_modifiers_to_scancode(&m) == Some(SDL_SCANCODE_LALT));
    }

    #[test]
    fn modifiers_multiple_returns_none() {
        let m = egui::Modifiers {
            shift: true,
            ctrl: true,
            alt: false,
            ..Default::default()
        };
        assert!(egui_modifiers_to_scancode(&m).is_none());
    }

    #[test]
    fn modifiers_none_returns_none() {
        let m = egui::Modifiers::default();
        assert!(egui_modifiers_to_scancode(&m).is_none());
    }

    // --- Known VPX default key conflicts ---

    #[test]
    fn default_actions_known_conflicts() {
        // Flipper/StagedFlipper sharing the same shift key is intentional
        // on a pincab (staged flippers are literally a second switch on
        // the same flipper button stack) — those collisions are filtered
        // out by `is_intentional_collision`, so they must NOT surface.
        // The two remaining unintentional collisions inherited from VPX's
        // own defaults (VolumeDown/Service7 on `-`, Credit4/Service5 on
        // `6`) must still be reported until upstream fixes them.
        let actions = default_actions();
        let conflicts = find_conflicts(&actions);
        assert!(
            !conflicts.is_empty(),
            "expected known default conflicts (MINUS, 6) inherited from VPX"
        );
        let find_pair = |id_a: &str, id_b: &str| -> bool {
            conflicts.iter().any(|&(i, j)| {
                (actions[i].setting_id == id_a && actions[j].setting_id == id_b)
                    || (actions[i].setting_id == id_b && actions[j].setting_id == id_a)
            })
        };
        assert!(
            !find_pair("LeftFlipper", "LeftStagedFlipper"),
            "LSHIFT flipper+staged collision should be whitelisted (intentional)"
        );
        assert!(
            !find_pair("RightFlipper", "RightStagedFlipper"),
            "RSHIFT flipper+staged collision should be whitelisted (intentional)"
        );
        assert!(
            find_pair("VolumeDown", "Service7"),
            "MINUS conflict missing"
        );
        assert!(find_pair("Credit4", "Service5"), "6 key conflict missing");
    }
}
