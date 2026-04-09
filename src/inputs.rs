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
            Self::JoystickButton { device_id, button, .. } => {
                format!("{device_id};{button}")
            }
            Self::JoystickAxis { device_id, axis, .. } => {
                format!("{device_id};{axis}")
            }
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::Keyboard { name, .. } => name,
            Self::JoystickButton { name, .. } => name,
            Self::JoystickAxis { name, .. } => name,
        }
    }
}

/// Joystick events sent from the SDL3 joystick thread to the UI.
#[derive(Clone)]
pub enum JoystickEvent {
    ButtonDown { device_id: String, button: u8, name: String },
    #[allow(dead_code)]
    AxisMotion { device_id: String, axis: u8, name: String },
    /// Live accelerometer/axis data for visualization (axis_id, normalized value -1.0 to 1.0)
    AccelUpdate { x: f32, y: f32 },
    /// A Pinscape controller was detected with this VPX device ID
    PinscapeDetected { vpx_id: String },
    /// A generic gamepad was detected with this VPX device ID
    GamepadDetected { vpx_id: String, name: String },
}

/// Build the VPX-compatible device ID for an SDL joystick.
/// VPX format: `SDLJoy_<serial>` where serial comes from SDL_GetJoystickSerial.
/// Falls back to GUID string if no serial is available.
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

/// Get a human-readable name for an SDL scancode.
pub fn scancode_name(scancode: SDL_Scancode) -> String {
    unsafe {
        let name_ptr = SDL_GetScancodeName(scancode);
        if !name_ptr.is_null() {
            let s = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
            if !s.is_empty() {
                return s;
            }
        }
    }
    format!("Key {}", scancode.0)
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
        Key::Minus => SDL_SCANCODE_MINUS,
        Key::Comma => SDL_SCANCODE_COMMA,
        Key::Period => SDL_SCANCODE_PERIOD,
        _ => return None,
    };
    Some(sc)
}

/// Check for modifier keys pressed in egui and return matching scancode.
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
        // Essentiels — flippers, staged, magna, commandes de jeu
        InputAction { setting_id: "LeftFlipper", label: "Flipper gauche", default_scancode: SDL_SCANCODE_LSHIFT, essential: true, mapping: None },
        InputAction { setting_id: "RightFlipper", label: "Flipper droit", default_scancode: SDL_SCANCODE_RSHIFT, essential: true, mapping: None },
        InputAction { setting_id: "LeftStagedFlipper", label: "Flipper staged gauche", default_scancode: SDL_SCANCODE_LSHIFT, essential: true, mapping: None },
        InputAction { setting_id: "RightStagedFlipper", label: "Flipper staged droit", default_scancode: SDL_SCANCODE_RSHIFT, essential: true, mapping: None },
        InputAction { setting_id: "LeftMagna", label: "MagnaSave gauche", default_scancode: SDL_SCANCODE_LCTRL, essential: true, mapping: None },
        InputAction { setting_id: "RightMagna", label: "MagnaSave droit", default_scancode: SDL_SCANCODE_RCTRL, essential: true, mapping: None },
        InputAction { setting_id: "Lockbar", label: "Lockbar", default_scancode: SDL_SCANCODE_LALT, essential: true, mapping: None },
        InputAction { setting_id: "ExtraBall", label: "Extra Ball", default_scancode: SDL_SCANCODE_B, essential: true, mapping: None },
        InputAction { setting_id: "LaunchBall", label: "Lance-bille", default_scancode: SDL_SCANCODE_RETURN, essential: true, mapping: None },
        InputAction { setting_id: "Start", label: "Start", default_scancode: SDL_SCANCODE_1, essential: true, mapping: None },
        InputAction { setting_id: "Credit1", label: "Ajouter credit", default_scancode: SDL_SCANCODE_5, essential: true, mapping: None },
        InputAction { setting_id: "ExitGame", label: "Quitter le jeu", default_scancode: SDL_SCANCODE_ESCAPE, essential: true, mapping: None },
        // Avances — coin door, services, nudge clavier, etc.
        InputAction { setting_id: "Credit2", label: "Credit (2)", default_scancode: SDL_SCANCODE_4, essential: false, mapping: None },
        InputAction { setting_id: "Credit3", label: "Credit (3)", default_scancode: SDL_SCANCODE_3, essential: false, mapping: None },
        InputAction { setting_id: "Credit4", label: "Credit (4)", default_scancode: SDL_SCANCODE_6, essential: false, mapping: None },
        InputAction { setting_id: "CoinDoor", label: "Porte monnayeur", default_scancode: SDL_SCANCODE_END, essential: false, mapping: None },
        InputAction { setting_id: "SlamTilt", label: "Slam Tilt", default_scancode: SDL_SCANCODE_HOME, essential: false, mapping: None },
        InputAction { setting_id: "Reset", label: "Reset", default_scancode: SDL_SCANCODE_F3, essential: false, mapping: None },
        InputAction { setting_id: "Service1", label: "Service Annuler/Quitter", default_scancode: SDL_SCANCODE_7, essential: false, mapping: None },
        InputAction { setting_id: "Service2", label: "Service Bas (-)", default_scancode: SDL_SCANCODE_8, essential: false, mapping: None },
        InputAction { setting_id: "Service3", label: "Service Haut (+)", default_scancode: SDL_SCANCODE_9, essential: false, mapping: None },
        InputAction { setting_id: "Service4", label: "Service Entree/OK", default_scancode: SDL_SCANCODE_0, essential: false, mapping: None },
        InputAction { setting_id: "Service5", label: "Service #5 (script table)", default_scancode: SDL_SCANCODE_6, essential: false, mapping: None },
        InputAction { setting_id: "Service6", label: "Service #6 (script table)", default_scancode: SDL_SCANCODE_PAGEUP, essential: false, mapping: None },
        InputAction { setting_id: "Service7", label: "Service #7 (script table)", default_scancode: SDL_SCANCODE_MINUS, essential: false, mapping: None },
        InputAction { setting_id: "Service8", label: "Service #8 (script table)", default_scancode: SDL_SCANCODE_UNKNOWN, essential: false, mapping: None },
        InputAction { setting_id: "LeftNudge", label: "Nudge gauche", default_scancode: SDL_SCANCODE_Z, essential: false, mapping: None },
        InputAction { setting_id: "RightNudge", label: "Nudge droit", default_scancode: SDL_SCANCODE_SLASH, essential: false, mapping: None },
        InputAction { setting_id: "CenterNudge", label: "Nudge centre", default_scancode: SDL_SCANCODE_SPACE, essential: false, mapping: None },
        InputAction { setting_id: "Tilt", label: "Tilt", default_scancode: SDL_SCANCODE_T, essential: false, mapping: None },
        InputAction { setting_id: "Pause", label: "Pause", default_scancode: SDL_SCANCODE_P, essential: false, mapping: None },
        InputAction { setting_id: "VolumeDown", label: "Volume -", default_scancode: SDL_SCANCODE_MINUS, essential: false, mapping: None },
        InputAction { setting_id: "VolumeUp", label: "Volume +", default_scancode: SDL_SCANCODE_EQUALS, essential: false, mapping: None },
        InputAction { setting_id: "Custom1", label: "Custom #1 (mod table)", default_scancode: SDL_SCANCODE_UNKNOWN, essential: false, mapping: None },
        InputAction { setting_id: "Custom2", label: "Custom #2 (mod table)", default_scancode: SDL_SCANCODE_UNKNOWN, essential: false, mapping: None },
        InputAction { setting_id: "Custom3", label: "Custom #3 (mod table)", default_scancode: SDL_SCANCODE_UNKNOWN, essential: false, mapping: None },
        InputAction { setting_id: "Custom4", label: "Custom #4 (mod table)", default_scancode: SDL_SCANCODE_UNKNOWN, essential: false, mapping: None },
    ]
}

/// Check if any two actions share the same key binding.
pub fn find_conflicts(actions: &[InputAction]) -> Vec<(usize, usize)> {
    let mut conflicts = Vec::new();
    for i in 0..actions.len() {
        for j in (i + 1)..actions.len() {
            let a = effective_scancode(&actions[i]);
            let b = effective_scancode(&actions[j]);
            if a != SDL_SCANCODE_UNKNOWN && a == b {
                conflicts.push((i, j));
            }
        }
    }
    conflicts
}

fn effective_scancode(action: &InputAction) -> SDL_Scancode {
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
                log::error!("Joystick thread: SDL_InitSubSystem failed: {:?}", CStr::from_ptr(SDL_GetError()));
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
                        log::info!("Opened joystick: {} (vpx_id={}, buttons={}, axes={})", name, dev_id, num_buttons, num_axes);
                        // Detect Pinscape controller
                        if name.contains("Pinscape") || dev_id.contains("PSC") {
                            log::info!("Pinscape controller detected: {}", dev_id);
                            let _ = evt_tx.send(JoystickEvent::PinscapeDetected { vpx_id: dev_id.clone() });
                        } else if SDL_IsGamepad(jid) {
                            // Generic gamepad (Xbox, PS, etc.)
                            log::info!("Gamepad detected: {} ({})", name, dev_id);
                            let _ = evt_tx.send(JoystickEvent::GamepadDetected { vpx_id: dev_id.clone(), name: name.clone() });
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
            log::info!("Joystick thread started, {} joystick(s) found", joysticks.len());

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
