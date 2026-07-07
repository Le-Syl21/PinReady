use super::*;

// Row sizing for the manually-laid-out action list. The action/keyboard/button
// columns are fixed; the joystick column is elastic and absorbs the remaining
// window width, so the table spans the full page. Long bindings — SDL3 emits
// things like `SDLJoy_03004c8009120000eaea000011010000_1 ; Button 7` (≈ 50
// chars) — are truncated with an ellipsis rather than stretching the row, which
// kept the layout tidy no matter what value lands in the cell.
const COL_ACTION_WIDTH: f32 = 280.0;
const COL_KEYBOARD_WIDTH: f32 = 170.0;
/// Minimum for the elastic joystick column; it grows to fill the window.
const COL_JOYSTICK_MIN: f32 = 240.0;
const COL_BUTTON_WIDTH: f32 = 140.0;
const COL_SPACING: f32 = 8.0;
const ROW_HEIGHT: f32 = 28.0;
/// Width reserved inside each binding column for its × unmap button, so the
/// text stays aligned whether or not the button is shown.
const UNMAP_BTN_WIDTH: f32 = 22.0;
/// Smallest total list width (narrow window); above this the joystick column
/// takes all the extra space so the table reaches the page edge.
const LIST_MIN_WIDTH: f32 =
    COL_ACTION_WIDTH + COL_KEYBOARD_WIDTH + COL_JOYSTICK_MIN + COL_BUTTON_WIDTH + 3.0 * COL_SPACING;

// Higher-contrast stripes than the previous (40/28) — a 4K cabinet
// playfield washes out subtle deltas, so we go full ±8 around mid-grey.
const BG_EVEN: egui::Color32 = egui::Color32::from_rgb(48, 48, 54);
const BG_ODD: egui::Color32 = egui::Color32::from_rgb(22, 22, 26);
const BG_HOVER: egui::Color32 = egui::Color32::from_rgb(78, 78, 92);
const BG_CAPTURING: egui::Color32 = egui::Color32::from_rgb(60, 110, 200);
const BG_CONFLICT: egui::Color32 = egui::Color32::from_rgb(140, 70, 35);
/// Thin horizontal divider painted between every row so the eye gets a
/// clean rule line, not just a colour gradient.
const ROW_BORDER: egui::Color32 = egui::Color32::from_rgb(95, 95, 110);

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
                // Product names, plus a localized "None" sentinel entry that
                // clears the defaults and opts out of detection re-applying
                // them.
                let profile_label = |i: usize| -> String {
                    inputs::PINSCAPE_PROFILES
                        .get(i)
                        .map(|s| (*s).to_string())
                        .unwrap_or_else(|| t!("inputs_profile_none").to_string())
                };
                egui::ComboBox::from_id_salt("pinscape_profile")
                    .selected_text(profile_label(self.pinscape_profile))
                    .show_ui(ui, |ui| {
                        for i in 0..=inputs::PINSCAPE_PROFILE_NONE {
                            ui.selectable_value(&mut self.pinscape_profile, i, profile_label(i));
                        }
                    })
                    .response
                    .on_hover_text(t!("inputs_pinscape_profile_hint"));
                if self.pinscape_profile != prev_profile {
                    if let Some(vpx_id) = self.pinscape_id.clone() {
                        for action in &mut self.actions {
                            action.joystick = None;
                        }
                        self.apply_pinscape_defaults(&vpx_id);
                    }
                }
            });
        }
        if self.gamepad_id.is_some() {
            ui.checkbox(&mut self.use_gamepad, t!("inputs_gamepad").to_string())
                .on_hover_text(t!("inputs_gamepad_hint"));
        }

        ui.add_space(4.0);
        ui.label(t!("inputs_instructions").to_string());
        ui.add_space(8.0);

        // Auto-map controls — walks every action in turn. "Skip" leaves the
        // current action's binding untouched and moves to the next; "Cancel"
        // bails out entirely. (Escape is a mappable key, not a skip.)
        ui.horizontal(|ui| {
            if !self.auto_map_active {
                if ui
                    .button(egui::RichText::new(t!("inputs_auto_map")).strong())
                    .on_hover_text(t!("inputs_auto_map_hint"))
                    .clicked()
                {
                    self.auto_map_active = true;
                    if !self.actions.is_empty() {
                        self.capture_state = CaptureState::Capturing(0);
                    }
                }
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 80),
                    t!("inputs_auto_map_running"),
                );
                if ui
                    .button(t!("inputs_auto_map_skip"))
                    .on_hover_text(t!("inputs_auto_map_skip_hint"))
                    .clicked()
                {
                    // Keep the current binding, advance to the next action.
                    self.advance_capture_or_finish();
                }
                if ui
                    .button(t!("inputs_auto_map_cancel"))
                    .on_hover_text(t!("inputs_auto_map_cancel_hint"))
                    .clicked()
                {
                    self.auto_map_active = false;
                    self.capture_state = CaptureState::Idle;
                }
            }
        });
        ui.add_space(8.0);

        // Process keyboard input via egui (has window focus)
        if let CaptureState::Capturing(idx) = self.capture_state {
            let modifiers = ui.input(|i| i.modifiers);
            let mut captured_or_skipped = false;

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
                    // Escape is a mappable key (VPX's default ExitGame) — it is
                    // captured like any other. Skipping an action is done only
                    // via the "Skip" button, never a key.
                    let sc = physical_key
                        .and_then(inputs::egui_key_to_scancode)
                        .or_else(|| inputs::egui_key_to_scancode(key));
                    if let Some(sc) = sc {
                        if idx < self.actions.len() {
                            // A key fills the keyboard slot only — any
                            // joystick binding stays alongside (VPX runs
                            // both as alternatives).
                            self.actions[idx].keyboard = Some(CapturedInput::Keyboard {
                                scancode: sc,
                                name: inputs::scancode_name(sc),
                            });
                        }
                        captured_or_skipped = true;
                        break;
                    }
                }
            }

            // Modifier-only press fallback (Shift/Ctrl/Alt by themselves).
            if !captured_or_skipped
                && (modifiers.shift || modifiers.ctrl || modifiers.alt)
                && key_events.is_empty()
            {
                if let Some(sc) = inputs::egui_modifiers_to_scancode(&modifiers) {
                    if idx < self.actions.len() {
                        self.actions[idx].keyboard = Some(CapturedInput::Keyboard {
                            scancode: sc,
                            name: inputs::scancode_name(sc),
                        });
                    }
                    captured_or_skipped = true;
                }
            }

            if captured_or_skipped {
                self.advance_capture_or_finish();
            }

            // Joystick events are processed in the main ui() method.

            // Request repaint while capturing to stay responsive.
            ui.ctx().request_repaint();
        }

        // Conflicts
        let conflicts = inputs::find_conflicts(&self.actions);

        // Single unified action list — no more "advanced" toggle.
        ui.add_space(4.0);
        self.render_action_list_unified(ui, &conflicts);
    }

    /// Move to the next action in auto-map mode, or fall back to Idle.
    /// Called after a successful capture or an Escape skip.
    pub(super) fn advance_capture_or_finish(&mut self) {
        if self.auto_map_active {
            if let CaptureState::Capturing(idx) = self.capture_state {
                let next = idx + 1;
                if next < self.actions.len() {
                    self.capture_state = CaptureState::Capturing(next);
                    return;
                }
                self.auto_map_active = false;
            }
        }
        self.capture_state = CaptureState::Idle;
    }

    fn render_action_list_unified(&mut self, ui: &mut egui::Ui, conflicts: &[(usize, usize)]) {
        // Span the full page width (down to a sensible minimum on narrow
        // windows). Tighter gap between cells — egui's default `item_spacing`
        // is 8 and our column widths already include their own padding.
        let width = ui.available_width().max(LIST_MIN_WIDTH);
        ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(COL_SPACING, 0.0);
                self.render_action_list_inner(ui, conflicts);
            },
        );
    }

    fn render_action_list_inner(&mut self, ui: &mut egui::Ui, conflicts: &[(usize, usize)]) {
        // Elastic joystick column: whatever width is left after the fixed
        // columns and gaps. Long bindings truncate inside it (see cell code).
        let joystick_w = (ui.available_width()
            - COL_ACTION_WIDTH
            - COL_KEYBOARD_WIDTH
            - COL_BUTTON_WIDTH
            - 3.0 * COL_SPACING)
            .max(COL_JOYSTICK_MIN);

        // Header row.
        ui.horizontal(|ui| {
            ui.add_sized(
                [COL_ACTION_WIDTH, ROW_HEIGHT],
                egui::Label::new(egui::RichText::new(t!("inputs_col_action")).strong()),
            );
            ui.add_sized(
                [COL_KEYBOARD_WIDTH, ROW_HEIGHT],
                egui::Label::new(egui::RichText::new(t!("inputs_col_keyboard")).strong()),
            );
            ui.add_sized(
                [joystick_w, ROW_HEIGHT],
                egui::Label::new(egui::RichText::new(t!("inputs_col_joystick")).strong()),
            );
            ui.add_sized([COL_BUTTON_WIDTH, ROW_HEIGHT], egui::Label::new(""));
        });

        let n = self.actions.len();
        for idx in 0..n {
            // Snapshot what we need from this action so we can take
            // `&mut self` later for the click handler.
            let label = self.actions[idx].label;
            let keyboard = self.actions[idx].keyboard.clone();
            let joystick = self.actions[idx].joystick.clone();
            let default_scancode = self.actions[idx].default_scancode;
            let is_capturing = self.capture_state == CaptureState::Capturing(idx);
            let has_conflict = conflicts.iter().any(|(a, b)| *a == idx || *b == idx);

            // Reserve a paint slot before the row so we can fill the
            // background based on hover / capturing state once we know
            // the row's rect.
            let bg_idx = ui.painter().add(egui::Shape::Noop);

            let row_resp = ui
                .horizontal(|ui| {
                    ui.set_min_height(ROW_HEIGHT);

                    let action_text = if is_capturing {
                        egui::RichText::new(t!(label))
                            .strong()
                            .color(egui::Color32::WHITE)
                    } else {
                        egui::RichText::new(t!(label))
                    };
                    ui.add_sized(
                        [COL_ACTION_WIDTH, ROW_HEIGHT],
                        egui::Label::new(action_text).truncate(),
                    );

                    // Keyboard column: custom key, else the VPX default.
                    // While capturing, the hint sits here (a key press fills
                    // this column, a joystick button the other one).
                    let keyboard_text = if is_capturing {
                        t!("inputs_capturing").to_string()
                    } else if let Some(ref captured) = keyboard {
                        captured.display_name().to_string()
                    } else if default_scancode != sdl3_sys::everything::SDL_SCANCODE_UNKNOWN {
                        format!(
                            "{}{}",
                            inputs::scancode_name(default_scancode),
                            t!("inputs_default_suffix")
                        )
                    } else {
                        t!("inputs_unassigned").to_string()
                    };
                    let keyboard_rich = if is_capturing {
                        egui::RichText::new(keyboard_text)
                            .strong()
                            .color(egui::Color32::WHITE)
                    } else if has_conflict {
                        egui::RichText::new(format!("/!\\ {keyboard_text}"))
                            .color(egui::Color32::from_rgb(255, 165, 0))
                    } else {
                        egui::RichText::new(keyboard_text)
                    };
                    ui.add_sized(
                        [
                            COL_KEYBOARD_WIDTH - UNMAP_BTN_WIDTH - COL_SPACING,
                            ROW_HEIGHT,
                        ],
                        egui::Label::new(keyboard_rich).truncate(),
                    );
                    // ✕ = drop the custom key, back to the VPX default.
                    // `add_visible` keeps the slot allocated when absent so
                    // the columns stay aligned.
                    if ui
                        .add_visible(
                            keyboard.is_some(),
                            egui::Button::new("×").min_size(egui::vec2(UNMAP_BTN_WIDTH, 0.0)),
                        )
                        .on_hover_text(t!("inputs_unmap_keyboard"))
                        .clicked()
                    {
                        self.actions[idx].keyboard = None;
                    }

                    // Joystick column: assigned button or em-dash. Lives
                    // alongside the keyboard binding — VPX runs both.
                    let joystick_text = if let Some(ref captured) = joystick {
                        captured.display_name().to_string()
                    } else {
                        "—".to_string()
                    };
                    let joystick_rich = if has_conflict {
                        egui::RichText::new(format!("/!\\ {joystick_text}"))
                            .color(egui::Color32::from_rgb(255, 165, 0))
                    } else if joystick.is_none() {
                        egui::RichText::new(joystick_text).weak()
                    } else {
                        egui::RichText::new(joystick_text)
                    };
                    ui.add_sized(
                        [joystick_w - UNMAP_BTN_WIDTH - COL_SPACING, ROW_HEIGHT],
                        egui::Label::new(joystick_rich).truncate(),
                    );
                    // ✕ = unassign the joystick button entirely.
                    if ui
                        .add_visible(
                            joystick.is_some(),
                            egui::Button::new("×").min_size(egui::vec2(UNMAP_BTN_WIDTH, 0.0)),
                        )
                        .on_hover_text(t!("inputs_unmap_joystick"))
                        .clicked()
                    {
                        self.actions[idx].joystick = None;
                    }

                    // Capture button.
                    let btn_label = if is_capturing {
                        t!("inputs_cancel").to_string()
                    } else {
                        t!("inputs_map").to_string()
                    };
                    let btn = egui::Button::new(btn_label)
                        .min_size(egui::vec2(COL_BUTTON_WIDTH, ROW_HEIGHT - 4.0));
                    if ui.add(btn).on_hover_text(t!("inputs_map_hint")).clicked() {
                        if is_capturing {
                            // Manual cancel — also bails out of auto-map.
                            self.auto_map_active = false;
                            self.capture_state = CaptureState::Idle;
                        } else {
                            self.auto_map_active = false;
                            self.capture_state = CaptureState::Capturing(idx);
                        }
                    }
                })
                .response;

            // Row background: capturing > conflict > hover > stripe,
            // plus a 1 px bottom rule between rows so the boundaries
            // are visible even on a glossy 4K playfield.
            let hovered = ui.rect_contains_pointer(row_resp.rect);
            let bg = if is_capturing {
                BG_CAPTURING
            } else if has_conflict {
                BG_CONFLICT
            } else if hovered {
                BG_HOVER
            } else if idx % 2 == 0 {
                BG_EVEN
            } else {
                BG_ODD
            };
            // Build a two-shape group: filled rect + bottom rule.
            // Using `Shape::Vec` keeps the single bg_idx slot we reserved.
            let group = vec![
                egui::Shape::rect_filled(row_resp.rect, 0.0, bg),
                egui::Shape::line_segment(
                    [
                        egui::pos2(row_resp.rect.left() + 2.0, row_resp.rect.bottom()),
                        egui::pos2(row_resp.rect.right() - 2.0, row_resp.rect.bottom()),
                    ],
                    egui::Stroke::new(1.0, ROW_BORDER),
                ),
            ];
            ui.painter().set(bg_idx, egui::Shape::Vec(group));
        }
    }
}
