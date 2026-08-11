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

        // Both rings and the bob share one space: the plumb's angle, with the
        // rim standing for the widest threshold VPX allows (4°).
        let angle_to_radius = |deg: f32| {
            radius * deg.to_radians().sin() / crate::tilt::TILT_ANGLE_MAX.to_radians().sin()
        };

        // Deadzone ring (green) — movements inside are ignored
        // The deadzone is a threshold on the sensor, but the circle speaks in
        // plumb angles — so express it as the angle a steady push right at
        // that threshold would settle the bob at: tan(angle) = a / g. Below
        // this, nothing reaches the physics at all.
        let deadzone_accel = (self.tilt.nudge_deadzone_pct / 100.0)
            * self.tilt.nudge_range_g
            * 9.806_65
            * (self.tilt.nudge_scale_pct / 100.0);
        let deadzone_angle = (deadzone_accel / 9.806_65).atan().to_degrees();
        let deadzone_radius = angle_to_radius(deadzone_angle);
        if deadzone_radius > 1.0 {
            painter.circle_filled(
                center,
                deadzone_radius,
                egui::Color32::from_rgba_unmultiplied(80, 200, 80, 30),
            );
            painter.circle_stroke(
                center,
                deadzone_radius,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 200, 80)),
            );
        }

        // Advance the plumb with what the sensor is actually reporting, put
        // through the same chain VPX applies: deadzone, then the range as a
        // unit conversion to m/s², then this sensor's strength factor.
        {
            let raw = |v: f32| {
                let dz = self.tilt.nudge_deadzone_pct / 100.0;
                let magnitude = v.abs();
                if magnitude <= dz {
                    0.0
                } else {
                    v.signum() * (magnitude - dz) / (1.0 - dz)
                }
            };
            let to_ms2 = self.tilt.nudge_range_g * 9.806_65 * (self.tilt.nudge_scale_pct / 100.0);
            let accel = (raw(self.accel_x) * to_ms2, raw(self.accel_y) * to_ms2);
            let threshold =
                crate::tilt::TiltConfig::threshold_angle(self.tilt.tilt_sensitivity_pct);
            // Catch up in 1 ms steps, VPX's own integration period, capped so
            // a stalled frame doesn't spin here.
            let now = std::time::Instant::now();
            let elapsed = self
                .plumb_last_step
                .map_or(1, |t| now.duration_since(t).as_millis().min(50) as u32);
            self.plumb_last_step = Some(now);
            for _ in 0..elapsed {
                self.plumb.step(accel, self.tilt.plumb_damping, threshold);
                if self.plumb.tilted {
                    self.plumb_tilt_until = Some(now + std::time::Duration::from_millis(700));
                }
            }
        }

        // TILT threshold ring (red) — beyond this = TILT
        // The ring is where a tilt triggers, so it must follow the *angle*,
        // not the sensitivity percentage: full sensitivity is the smallest
        // angle and therefore the tightest ring. Drawing it from the
        // percentage put the ring at the rim exactly when the tilt was at its
        // most touchy.
        // Both ring and bob are drawn in the plumb's own space — the bob's
        // offset as a fraction of the rod length — so the circle finally
        // means one thing throughout. The rim is the widest threshold VPX
        // allows (4°), which is what makes a shake readable at any setting.
        let threshold_radius = angle_to_radius(crate::tilt::TiltConfig::threshold_angle(
            self.tilt.tilt_sensitivity_pct,
        ));
        painter.circle_stroke(
            center,
            threshold_radius,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 80, 80)),
        );
        painter.text(
            center + egui::vec2(threshold_radius + 4.0, -10.0),
            egui::Align2::LEFT_CENTER,
            "TILT",
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(255, 80, 80),
        );

        // The bob itself. Its offset is already a fraction of the rod length,
        // and the rim stands for a 4° swing, so the dot and the ring are
        // directly comparable — which is the whole point of the picture.
        let (bob_x, bob_y) = self.plumb.plane_offset();
        let rim = crate::tilt::TILT_ANGLE_MAX.to_radians().sin();
        let dot_x = center.x + (bob_x / rim).clamp(-1.2, 1.2) * radius;
        let dot_y = center.y + (bob_y / rim).clamp(-1.2, 1.2) * radius;
        let dot_pos = egui::pos2(dot_x, dot_y);
        let dist = ((dot_x - center.x).powi(2) + (dot_y - center.y).powi(2)).sqrt();
        let dot_color = if dist > threshold_radius {
            egui::Color32::from_rgba_unmultiplied(255, 50, 50, 200) // in TILT zone
        } else if dist < deadzone_radius {
            egui::Color32::from_rgba_unmultiplied(150, 150, 150, 200) // ignored
        } else {
            egui::Color32::from_rgba_unmultiplied(100, 220, 100, 200) // active
        };
        // Small and translucent, like a laser dot: a 7px opaque disc covered
        // the deadzone ring whole at the low percentages that actually make
        // sense, hiding the very thing being adjusted. The halo keeps it
        // findable at a glance without painting over what is underneath.
        painter.circle_filled(dot_pos, 8.0, dot_color.gamma_multiply(0.25));
        painter.circle_filled(dot_pos, 3.5, dot_color);

        // A banner naming what just happened. Rising is instant, falling
        // waits a second: a tilt that lasts one millisecond would otherwise
        // be gone before the eye arrives, and the point of shaking the
        // cabinet here is to see what the shake did.
        let now = std::time::Instant::now();
        let sensor_magnitude = self.accel_x.abs().max(self.accel_y.abs());
        let instant_state = if self.plumb_tilt_until.is_some_and(|t| now < t) {
            2
        } else if sensor_magnitude > self.tilt.nudge_deadzone_pct / 100.0 {
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
