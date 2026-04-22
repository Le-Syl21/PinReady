use super::*;

impl App {
    pub(super) fn render_inputs_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("inputs_heading"));
        ui.add_space(4.0);

        // Detected controllers info
        if self.pinscape_id.is_some() {
            ui.label(t!("inputs_pinscape").to_string());
            ui.horizontal(|ui| {
                ui.label(t!("inputs_pinscape_profile"));
                let prev_profile = self.pinscape_profile;
                egui::ComboBox::from_id_salt("pinscape_profile")
                    .selected_text(inputs::PINSCAPE_PROFILES[self.pinscape_profile])
                    .show_ui(ui, |ui| {
                        for (i, label) in inputs::PINSCAPE_PROFILES.iter().enumerate() {
                            ui.selectable_value(&mut self.pinscape_profile, i, *label);
                        }
                    });
                if self.pinscape_profile != prev_profile {
                    // Re-apply defaults with new profile
                    if let Some(vpx_id) = self.pinscape_id.clone() {
                        // Clear existing joystick mappings before re-applying
                        for action in &mut self.actions {
                            if matches!(&action.mapping, Some(CapturedInput::JoystickButton { .. }))
                            {
                                action.mapping = None;
                            }
                        }
                        self.apply_pinscape_defaults(&vpx_id);
                    }
                }
            });
        }
        if self.gamepad_id.is_some() {
            ui.checkbox(&mut self.use_gamepad, t!("inputs_gamepad").to_string());
        }

        ui.add_space(4.0);
        ui.label(t!("inputs_instructions").to_string());
        ui.add_space(8.0);

        // Process keyboard input via egui (has window focus)
        if let CaptureState::Capturing(idx) = self.capture_state {
            // Check for modifier-only presses (Shift, Ctrl, Alt)
            let modifiers = ui.input(|i| i.modifiers);
            let mut captured = false;

            // Check key events. We capture both the logical key (what the
            // user sees — e.g. "!" when pressing Shift+1 on QWERTY) and
            // the physical key (the unshifted position — Num1 in that
            // example). The physical key gets priority: it's what VPX
            // stores in the ini as an SDL scancode, and it's layout-
            // independent so users on AZERTY keep a consistent binding.
            let key_events: Vec<(egui::Key, Option<egui::Key>, bool)> = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| {
                        if let egui::Event::Key {
                            key,
                            physical_key,
                            pressed,
                            ..
                        } = e
                        {
                            Some((*key, *physical_key, *pressed))
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            for &(key, physical_key, pressed) in &key_events {
                if pressed {
                    if key == egui::Key::Escape {
                        self.capture_state = CaptureState::Idle;
                        captured = true;
                        break;
                    }
                    let sc = physical_key
                        .and_then(inputs::egui_key_to_scancode)
                        .or_else(|| inputs::egui_key_to_scancode(key));
                    if let Some(sc) = sc {
                        if idx < self.actions.len() {
                            self.actions[idx].mapping = Some(CapturedInput::Keyboard {
                                scancode: sc,
                                name: inputs::scancode_name(sc),
                            });
                        }
                        self.capture_state = CaptureState::Idle;
                        captured = true;
                        break;
                    }
                }
            }

            // Check modifier-only press (e.g., just Shift pressed alone)
            if !captured && (modifiers.shift || modifiers.ctrl || modifiers.alt) {
                // Wait for a key event to pair with the modifier, or capture modifier alone
                // We only capture modifier alone if no other key event came through
                if key_events.is_empty() {
                    if let Some(sc) = inputs::egui_modifiers_to_scancode(&modifiers) {
                        if idx < self.actions.len() {
                            self.actions[idx].mapping = Some(CapturedInput::Keyboard {
                                scancode: sc,
                                name: inputs::scancode_name(sc),
                            });
                        }
                        self.capture_state = CaptureState::Idle;
                    }
                }
            }

            // Joystick events are processed in the main ui() method

            // Request repaint while capturing to stay responsive
            ui.ctx().request_repaint();
        }

        // Conflicts
        let conflicts = inputs::find_conflicts(&self.actions);

        // Essential actions
        ui.strong(t!("inputs_essential").to_string());
        self.render_action_list(ui, true, &conflicts);

        ui.add_space(8.0);
        ui.checkbox(
            &mut self.show_advanced_inputs,
            t!("inputs_show_advanced").to_string(),
        );
        if self.show_advanced_inputs {
            ui.add_space(4.0);
            ui.strong(t!("inputs_advanced").to_string());
            self.render_action_list(ui, false, &conflicts);
        }
    }

    pub(super) fn render_action_list(
        &mut self,
        ui: &mut egui::Ui,
        essential: bool,
        conflicts: &[(usize, usize)],
    ) {
        egui::Grid::new(if essential {
            "essential_inputs"
        } else {
            "advanced_inputs"
        })
        .striped(true)
        .min_col_width(120.0)
        .show(ui, |ui| {
            ui.strong(t!("inputs_col_action").to_string());
            ui.strong(t!("inputs_col_binding").to_string());
            ui.strong("");
            ui.end_row();

            for (idx, action) in self.actions.iter().enumerate() {
                if action.essential != essential {
                    continue;
                }

                ui.label(t!(action.label));

                // Current binding display
                let is_capturing = self.capture_state == CaptureState::Capturing(idx);
                let binding_text = if is_capturing {
                    t!("inputs_capturing").to_string()
                } else if let Some(captured) = &action.mapping {
                    captured.display_name().to_string()
                } else if action.default_scancode != sdl3_sys::everything::SDL_SCANCODE_UNKNOWN {
                    format!(
                        "{}{}",
                        inputs::scancode_name(action.default_scancode),
                        t!("inputs_default_suffix")
                    )
                } else {
                    t!("inputs_unassigned").to_string()
                };

                // Conflict warning
                let has_conflict = conflicts.iter().any(|(a, b)| *a == idx || *b == idx);
                if has_conflict {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 165, 0),
                        format!("/!\\ {binding_text}"),
                    );
                } else {
                    ui.label(&binding_text);
                }

                // Capture button
                let btn_label = if is_capturing {
                    t!("inputs_cancel").to_string()
                } else {
                    t!("inputs_map").to_string()
                };
                if ui.button(btn_label).clicked() {
                    if is_capturing {
                        self.capture_state = CaptureState::Idle;
                    } else {
                        self.capture_state = CaptureState::Capturing(idx);
                    }
                }
                ui.end_row();
            }
        });
    }
}
