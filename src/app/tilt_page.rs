use super::*;

const NOTICE_AMBER_TILT: egui::Color32 = egui::Color32::from_rgb(255, 200, 80);

impl App {
    pub(super) fn render_tilt_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tilt_heading"));
        ui.add_space(4.0);
        ui.label(t!("tilt_desc"));
        ui.add_space(12.0);

        // Request repaint for live accelerometer data
        ui.ctx().request_repaint();

        // --- Nudge section ---
        ui.separator();
        ui.strong(t!("tilt_nudge"));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(t!("tilt_sensitivity"));
            help_marker(ui, &t!("tilt_sensitivity_help"));
        });
        ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Slider::new(&mut self.tilt.nudge_scale_pct, 0.0..=200.0)
                .custom_formatter(|v, _| format!("{:.0}%", v)),
        );
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(t!("tilt_deadzone"));
            help_marker(ui, &t!("tilt_deadzone_help"));
        });
        ui.add_sized(
            [ui.available_width(), 24.0],
            // VPX caps this at 0.3 of the axis range ("relative amount of
            // the axis range to nullify to avoid noise at rest position"), so
            // offering more would write values its own UI cannot show.
            egui::Slider::new(&mut self.tilt.nudge_deadzone_pct, 0.0..=30.0)
                .custom_formatter(|v, _| format!("{v:.0}%")),
        );
        ui.add_space(8.0);

        // Accelerometer range — read from the board, not asked. The
        // firmware knows it (Pinscape config variable 4) and the user
        // generally does not, so this is a readout, not a question. The peak
        // below stays: it says whether the cabinet actually exercises that
        // range.
        if let Some(rx) = &self.pinscape_cfg_rx {
            if let Ok(cfg) = rx.try_recv() {
                self.pinscape_cfg_rx = None;
                if let Some(range) = cfg.and_then(|c| c.accel_range_g) {
                    self.tilt.nudge_range_g = range;
                }
                self.pinscape_cfg = cfg;
            }
        }
        ui.add_enabled_ui(false, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(t!("tilt_range"))
                        .on_hover_text(t!("tilt_range_help"));
                    // A Pico answers about its plunger but keeps its
                    // accelerometer settings in a file we can't reach over
                    // HID, so "a board is present" is not "the range is
                    // known".
                    match self.pinscape_cfg.and_then(|c| c.accel_range_g) {
                        Some(range) => ui.label(
                            egui::RichText::new(t!(
                                "tilt_range_detected",
                                range = format!("{range:.0}"),
                                orientation = self
                                    .pinscape_cfg
                                    .map(|c| c.orientation_label())
                                    .unwrap_or_default()
                            ))
                            .strong(),
                        ),
                        // Naming the board that stayed silent beats implying
                        // there is none.
                        None => match self.pinscape_cfg.map(|c| c.board) {
                            Some(board) => ui.label(t!(
                                "tilt_range_unsupported",
                                range = format!("{:.0}", self.tilt.nudge_range_g),
                                board = match board {
                                    crate::pinscape_config::Board::Kl25z => "Pinscape KL25Z",
                                    crate::pinscape_config::Board::Pico => "Pinscape Pico",
                                    crate::pinscape_config::Board::Opaque(name) => name,
                                }
                            )),
                            None => ui.label(t!(
                                "tilt_range_default",
                                range = format!("{:.0}", self.tilt.nudge_range_g)
                            )),
                        },
                    };
                });
                // Peak of |x|,|y| since entering the page, as a share of full
                // scale: a shake that never passes ~20 % means the board is
                // set wider than the cabinet ever exercises. It doubles as a
                // presence test — an axis that never leaves zero is either
                // unmapped or has nothing wired to it, and telling that apart
                // from "badly scaled" is the first question to answer.
                let peak = self.accel_x.abs().max(self.accel_y.abs());
                self.nudge_peak = self.nudge_peak.max(peak);
                ui.label(if self.nudge_peak > 0.004 {
                    egui::RichText::new(t!("tilt_accel_present"))
                        .color(egui::Color32::from_rgb(120, 200, 120))
                } else {
                    egui::RichText::new(t!("tilt_accel_absent")).color(NOTICE_AMBER_TILT)
                });
                ui.label(
                    egui::RichText::new(t!(
                        "tilt_peak",
                        pct = format!("{:.0}", self.nudge_peak * 100.0),
                        g = format!("{:.2}", self.nudge_peak * self.tilt.nudge_range_g)
                    ))
                    .weak(),
                );
            });
        });
        if ui.small_button(t!("tilt_peak_reset")).clicked() {
            self.nudge_peak = 0.0;
        }
        ui.add_space(8.0);

        // What the three modes actually mean, and in what order the filters
        // apply — none of which VPX documents, and all of which decides
        // whether a nudge reaches the ball.
        ui.collapsing(t!("tilt_modes_title"), |ui| {
            ui.label(t!("tilt_modes_gamepad"));
            ui.add_space(4.0);
            ui.label(t!("tilt_modes_intent"));
            ui.add_space(4.0);
            ui.label(t!("tilt_modes_cabinet"));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t!("tilt_modes_chain")).strong());
            ui.label(t!("tilt_modes_chain_detail"));
        });
        ui.add_space(6.0);

        // Nudge sensor type (VPX new sensor schema: Mapping.Nudge0.Type).
        let sensor_types = [
            (0_i32, t!("tilt_nudge_type_game")),
            (1, t!("tilt_nudge_type_intent")),
            (2, t!("tilt_nudge_type_cabinet")),
        ];
        ui.horizontal(|ui| {
            ui.label(t!("tilt_nudge_type"));
            help_marker(ui, &t!("tilt_nudge_type_help"));
        });
        let selected = sensor_types
            .iter()
            .find(|(v, _)| *v == self.tilt.nudge_sensor_type)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("nudge_sensor_type")
            .selected_text(selected)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (val, label) in &sensor_types {
                    ui.selectable_value(&mut self.tilt.nudge_sensor_type, *val, label.clone());
                }
            });
        ui.add_space(8.0);

        // --- Tilt section ---
        ui.separator();
        ui.strong(t!("tilt_section"));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(t!("tilt_threshold"));
            help_marker(ui, &t!("tilt_threshold_help"));
        });
        ui.add_sized(
            [ui.available_width(), 24.0],
            // The percentage is ours; the degrees are what VPX stores and
            // what its own UI shows, so print both rather than make anyone
            // convert between the two.
            egui::Slider::new(&mut self.tilt.tilt_sensitivity_pct, 0.0..=100.0).custom_formatter(
                |v, _| {
                    format!(
                        "{v:.0}% ({:.2}°)",
                        crate::tilt::TiltConfig::threshold_angle(v as f32)
                    )
                },
            ),
        );
        // The scale runs backwards from the angle it writes, so label both
        // ends rather than leave anyone to work out which way is which.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t!("tilt_threshold_low")).weak().small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(t!("tilt_threshold_high"))
                        .weak()
                        .small(),
                );
            });
        });
        ui.add_space(8.0);

        // Warning if deadzone >= tilt threshold
        if self.tilt.nudge_deadzone_pct >= self.tilt.tilt_sensitivity_pct {
            ui.colored_label(
                egui::Color32::from_rgb(255, 180, 50),
                t!("tilt_deadzone_warning"),
            );
        }
        ui.add_space(4.0);

        // Visualization: deadzone (green ring) + tilt (red ring) + live dot
        ui.label(t!("tilt_visualization"));
        ui.add_space(4.0);
        let viz_size = egui::vec2(240.0, 240.0);
        let (rect, _response) = ui.allocate_exact_size(viz_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        // Snap to the pixel grid: a centre landing on a half pixel smears the
        // 1px crosshair over two rows and the rings read as off-centre.
        let center = {
            use egui::emath::GuiRounding as _;
            rect.center().round_to_pixel_center(ui.pixels_per_point())
        };
        let radius = 110.0;

        // Outer circle (max range)
        painter.circle_stroke(center, radius, egui::Stroke::new(2.0, egui::Color32::GRAY));
        // Cross hairs
        painter.line_segment(
            [
                center - egui::vec2(radius, 0.0),
                center + egui::vec2(radius, 0.0),
            ],
            egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
        );
        painter.line_segment(
            [
                center - egui::vec2(0.0, radius),
                center + egui::vec2(0.0, radius),
            ],
            egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
        );

        // One scale for every ring: the fraction of the accelerometer's own
        // range, magnified so the small thresholds are aimable. Each ring is
        // the shake it takes to reach that limit, which is the only way to
        // compare a deadzone (a threshold on the signal) with a tilt angle (a
        // pendulum's state). The tilt rings are converted through
        // tan θ = a/g — a steady push that would settle the plumb there.
        const FULL_SCALE: f32 = 0.25; // rim = a quarter of the sensor range
        let to_radius = |fraction: f32| radius * (fraction / FULL_SCALE).clamp(0.0, 1.0);
        let angle_to_fraction = |deg: f32| {
            // Acceleration that holds the plumb at this angle, back to a
            // fraction of the sensor's range.
            let accel = deg.to_radians().tan() * 9.806_65;
            accel / (self.tilt.nudge_range_g * 9.806_65).max(0.001)
        };
        let strength = (self.tilt.nudge_scale_pct / 100.0).max(0.01);

        // Outermost meaningful ring: the widest tilt VPX allows.
        let max_tilt_radius = to_radius(angle_to_fraction(crate::tilt::TILT_ANGLE_MAX));
        painter.circle_stroke(
            center,
            max_tilt_radius,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(150, 90, 90)),
        );

        // The tilt threshold in force.
        let threshold_radius = to_radius(angle_to_fraction(
            crate::tilt::TiltConfig::threshold_angle(self.tilt.tilt_sensitivity_pct),
        ));
        painter.circle_stroke(
            center,
            threshold_radius,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 120, 120)),
        );

        // Intent threshold — fixed at 1 m/s², but Strength is applied before
        // the comparison, so raising it moves this ring inwards.
        if self.tilt.nudge_sensor_type == 1 {
            let intent_fraction = crate::nudge_sim::INTENT_THRESHOLD_MS2
                / (self.tilt.nudge_range_g * 9.806_65 * strength);
            painter.circle_stroke(
                center,
                to_radius(intent_fraction),
                egui::Stroke::new(2.0, egui::Color32::from_rgb(230, 160, 60)),
            );
        }

        // Deadzone, innermost by nature: it gates the raw signal first.
        let deadzone_radius = to_radius(self.tilt.nudge_deadzone_pct / 100.0);
        if deadzone_radius > 1.0 {
            painter.circle_filled(
                center,
                deadzone_radius,
                egui::Color32::from_rgba_unmultiplied(80, 200, 80, 40),
            );
            painter.circle_stroke(
                center,
                deadzone_radius,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 200, 80)),
            );
        }

        // The live sensor reading, thin and translucent — a laser dot that
        // shows movement everywhere, including inside the deadzone, which is
        // the one place the plumb can never show anything.
        let dot_pos = egui::pos2(
            center.x + (self.accel_x / FULL_SCALE).clamp(-1.05, 1.05) * radius,
            center.y + (self.accel_y / FULL_SCALE).clamp(-1.05, 1.05) * radius,
        );
        let laser = egui::Color32::from_rgba_unmultiplied(255, 60, 60, 210);
        painter.circle_filled(dot_pos, 7.0, laser.gamma_multiply(0.18));
        painter.circle_filled(dot_pos, 2.0, laser);

        // A banner naming what just happened. Rising is instant, falling
        // waits a second: a tilt that lasts one millisecond would otherwise
        // be gone before the eye arrives, and the point of shaking the
        // cabinet here is to see what the shake did.
        let now = std::time::Instant::now();
        let sensor_magnitude = self.accel_x.abs().max(self.accel_y.abs());
        let instant_state = if self.plumb_tilt_until.is_some_and(|t| now < t) {
            2
        } else if sensor_magnitude > self.tilt.nudge_deadzone_pct / 100.0
            || self.nudge_sim.impulse_active()
        {
            // Either the raw signal cleared the deadzone, or the intent
            // detector recognised a nudge and is delivering its impulse.
            1
        } else {
            0
        };
        let calmed_down = self
            .nudge_state_since
            .is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_secs(1));
        if instant_state >= self.nudge_state || calmed_down {
            self.nudge_state = instant_state;
            self.nudge_state_since = Some(now);
        }

        ui.add_space(6.0);
        let (bg, label) = match self.nudge_state {
            2 => (egui::Color32::from_rgb(200, 60, 60), t!("tilt_state_tilt")),
            1 => (
                egui::Color32::from_rgb(210, 140, 40),
                t!("tilt_state_nudge"),
            ),
            _ => (
                egui::Color32::from_rgb(70, 150, 70),
                t!("tilt_state_deadzone"),
            ),
        };
        egui::Frame::new()
            .fill(bg)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .corner_radius(4)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
            });
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(t!(
                "tilt_plumb_angle",
                angle = format!("{:.2}", self.plumb.angle_deg()),
                threshold = format!(
                    "{:.2}",
                    crate::tilt::TiltConfig::threshold_angle(self.tilt.tilt_sensitivity_pct)
                )
            ))
            .weak(),
        );
    }
}
