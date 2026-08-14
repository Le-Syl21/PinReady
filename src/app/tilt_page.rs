use super::*;
use crate::tilt::SENSOR_RANGES_G;

const NOTICE_AMBER_TILT: egui::Color32 = egui::Color32::from_rgb(255, 200, 80);
const RING_TILT: egui::Color32 = egui::Color32::from_rgb(255, 120, 120);
const RING_INTENT: egui::Color32 = egui::Color32::from_rgb(230, 160, 60);
const RING_DEADZONE: egui::Color32 = egui::Color32::from_rgb(80, 200, 80);
/// The sensor pinned at its own ceiling — not a level, a loss of information.
const RING_CLIPPED: egui::Color32 = egui::Color32::from_rgb(255, 210, 60);

/// How long a clipping mark stays on screen. A nudge is over in milliseconds,
/// long before anyone looks up from the cabinet.
const CLIP_MARK_LIFE: std::time::Duration = std::time::Duration::from_secs(4);

/// How many marks to keep. Enough to show where a shove saturates, few enough
/// that a violent session does not paint the whole rim.
const CLIP_MARKS_KEPT: usize = 24;

/// One place, and one moment, where the sensor ran out of range.
pub struct ClipMark {
    at: egui::Pos2,
    seen: std::time::Instant,
}
const STRENGTH_RED: egui::Color32 = egui::Color32::from_rgb(235, 90, 90);

impl App {
    /// Keep the dial's rings in step with the sliders. Placing them runs the
    /// cabinet physics a few dozen times — cheap, but not per-frame cheap.
    fn refresh_tilt_rings(&mut self) {
        let key = self.tilt.rings_key();
        if key != self.tilt_rings_key {
            self.tilt_rings = self.tilt.rings();
            self.tilt_rings_key = key;
        }
    }

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

        // Sensor range — read from the board, not asked. The firmware knows it
        // (Pinscape config variable 4) and the user generally does not, so the
        // detected value is adopted and the choice stays available for boards
        // that will not answer.
        if let Some(rx) = &self.pinscape_cfg_rx {
            if let Ok(cfg) = rx.try_recv() {
                self.pinscape_cfg_rx = None;
                if let Some(range) = cfg.and_then(|c| c.accel_range_g) {
                    self.tilt.nudge_range_g = range;
                }
                self.pinscape_cfg = cfg;
            }
        }
        let detected_range = self.pinscape_cfg.and_then(|c| c.accel_range_g);
        self.refresh_tilt_rings();

        ui.horizontal(|ui| {
            ui.label(t!("tilt_range"));
            help_marker(ui, &t!("tilt_range_help"));
        });
        ui.horizontal(|ui| {
            for range in SENSOR_RANGES_G {
                let selected = (self.tilt.nudge_range_g - range).abs() < 0.01;
                if ui
                    .selectable_label(selected, format!("{range:.0} G"))
                    .clicked()
                {
                    self.tilt.nudge_range_g = range;
                }
            }
        });
        match detected_range {
            Some(detected) if (detected - self.tilt.nudge_range_g).abs() < 0.01 => {
                ui.label(
                    egui::RichText::new(t!("tilt_range_matches", range = format!("{detected:.0}")))
                        .color(egui::Color32::from_rgb(120, 200, 120)),
                );
            }
            // A range that disagrees with the firmware is the single most
            // damaging setting on this page: it is a unit conversion, so it
            // scales nudge and tilt together, and nothing downstream can undo
            // it. Name the factor — that is what makes the symptom
            // recognisable.
            Some(detected) => {
                let factor = self.tilt.nudge_range_g / detected;
                ui.colored_label(
                    egui::Color32::from_rgb(255, 140, 60),
                    t!(
                        if factor > 1.0 {
                            "tilt_range_mismatch_high"
                        } else {
                            "tilt_range_mismatch_low"
                        },
                        detected = format!("{detected:.0}"),
                        chosen = format!("{:.0}", self.tilt.nudge_range_g),
                        factor = if factor > 1.0 {
                            format!("×{factor:.0}")
                        } else {
                            format!("÷{:.0}", 1.0 / factor)
                        }
                    ),
                );
                if ui
                    .button(t!("tilt_range_adopt", range = format!("{detected:.0}")))
                    .clicked()
                {
                    self.tilt.nudge_range_g = detected;
                }
            }
            None => {
                // Naming the board that stayed silent beats implying there is
                // none.
                ui.label(
                    egui::RichText::new(match self.pinscape_cfg.map(|c| c.board) {
                        Some(board) => t!(
                            "tilt_range_unsupported",
                            board = match board {
                                crate::pinscape_config::Board::Kl25z => "Pinscape KL25Z",
                                crate::pinscape_config::Board::Pico => "Pinscape Pico",
                                crate::pinscape_config::Board::Opaque(name) => name,
                            }
                        ),
                        None => t!("tilt_range_unknown"),
                    })
                    .weak(),
                );
            }
        }
        // How the board is fitted. Not a setting — the firmware rotates its
        // readings into cabinet axes itself, so nothing here is written to the
        // ini. It is shown because a board declared one way and screwed down
        // another mirrors the nudge, and no other screen would ever say so.
        if let Some(key) = self.pinscape_cfg.and_then(|c| c.orientation_key()) {
            ui.label(egui::RichText::new(t!("tilt_orientation", side = t!(key))).weak());
        }
        self.render_range_tables(ui);
        ui.add_space(8.0);

        // Strength — in practice the only setting worth touching once the
        // range is read off the board, so it is the one that stands out.
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("tilt_sensitivity"))
                    .strong()
                    .color(STRENGTH_RED),
            );
            help_marker(ui, &t!("tilt_sensitivity_help"));
        });
        ui.scope(|ui| {
            let visuals = &mut ui.style_mut().visuals;
            visuals.selection.bg_fill = STRENGTH_RED;
            visuals.widgets.inactive.fg_stroke.color = STRENGTH_RED;
            ui.add_sized(
                [ui.available_width(), 24.0],
                egui::Slider::new(&mut self.tilt.nudge_scale_pct, 0.0..=200.0)
                    .custom_formatter(|v, _| format!("{v:.0}%")),
            );
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(t!("tilt_deadzone"));
            help_marker(ui, &t!("tilt_deadzone_help"));
        });
        let full_scale = self.tilt.full_scale_ms2();
        ui.add_sized(
            [ui.available_width(), 24.0],
            // VPX caps this at 0.3 of the axis range ("relative amount of
            // the axis range to nullify to avoid noise at rest position"), so
            // offering more would write values its own UI cannot show. The
            // m/s² is not decoration: this is the one setting whose real
            // effect changes with the sensor range.
            egui::Slider::new(&mut self.tilt.nudge_deadzone_pct, 0.0..=30.0).custom_formatter(
                move |v, _| format!("{v:.0}% ({:.2} m/s²)", (v as f32 / 100.0) * full_scale),
            ),
        );
        ui.add_space(8.0);

        ui.add_enabled_ui(false, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
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
                        accel = format!("{:.2}", self.nudge_peak * self.tilt.full_scale_ms2())
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
        ui.add_space(6.0);

        self.refresh_tilt_rings();
        // Comparable at last: both are accelerations now, so this says
        // something true. The old version compared a dead-zone percentage
        // against a sensitivity percentage — two different scales, and the
        // warning fired on settings that worked perfectly.
        if self.tilt_rings.deadzone >= self.tilt_rings.tilt {
            ui.colored_label(
                egui::Color32::from_rgb(255, 180, 50),
                t!("tilt_deadzone_warning"),
            );
        }
        self.render_out_of_reach_notice(ui);
        ui.add_space(4.0);

        self.render_tilt_dial(ui);
    }

    /// Warn — and offer the two ways out — when the configured tilt threshold
    /// needs a harder shove than the sensor can report. Raising the range is
    /// the honest fix and costs nothing; Strength only compensates.
    fn render_out_of_reach_notice(&mut self, ui: &mut egui::Ui) {
        if !self.tilt_rings.tilt_out_of_reach() {
            return;
        }
        let wider = SENSOR_RANGES_G
            .iter()
            .copied()
            .find(|r| *r > self.tilt.nudge_range_g && self.reach_covers(*r));
        egui::Frame::group(ui.style())
            .fill(egui::Color32::from_rgb(70, 45, 20))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(t!(
                        "tilt_unreachable",
                        range = format!("{:.0}", self.tilt.nudge_range_g),
                        angle = format!("{:.2}", self.tilt.threshold_deg())
                    ))
                    .color(NOTICE_AMBER_TILT)
                    .strong(),
                );
                if let Some(range) = wider {
                    ui.label(t!("tilt_unreachable_range", range = format!("{range:.0}")));
                }
                ui.horizontal(|ui| {
                    if let Some(range) = wider {
                        if ui
                            .button(t!("tilt_range_adopt", range = format!("{range:.0}")))
                            .clicked()
                        {
                            self.tilt.nudge_range_g = range;
                        }
                    }
                    if let Some(pct) = self.tilt_rings.strength_to_reach_pct {
                        if ui
                            .button(t!("tilt_unreachable_strength", pct = format!("{pct:.0}")))
                            .clicked()
                        {
                            self.tilt.nudge_scale_pct = pct;
                        }
                    }
                });
            });
    }

    /// Whether a given sensor range still reaches the configured threshold.
    fn reach_covers(&self, range_g: f32) -> bool {
        SENSOR_RANGES_G
            .iter()
            .position(|r| (*r - range_g).abs() < 0.01)
            .is_some_and(|i| self.tilt_rings.reach[i].1 >= self.tilt.threshold_deg())
    }

    /// The two tables that make the range choice decidable rather than a
    /// guess: what a wrong declaration does, and what each range can reach.
    fn render_range_tables(&mut self, ui: &mut egui::Ui) {
        ui.collapsing(t!("tilt_range_tables_title"), |ui| {
            ui.label(t!("tilt_range_table_mismatch_intro"));
            ui.add_space(4.0);
            egui::Grid::new("tilt_range_mismatch")
                .striped(true)
                .show(ui, |ui| {
                    ui.label("");
                    for vpx in SENSOR_RANGES_G {
                        ui.label(egui::RichText::new(format!("VPX {vpx:.0} G")).strong());
                    }
                    ui.end_row();
                    for firmware in SENSOR_RANGES_G {
                        ui.label(
                            egui::RichText::new(t!(
                                "tilt_range_table_firmware",
                                range = format!("{firmware:.0}")
                            ))
                            .strong(),
                        );
                        for vpx in SENSOR_RANGES_G {
                            let factor = vpx / firmware;
                            ui.label(if (factor - 1.0).abs() < 0.01 {
                                egui::RichText::new(t!("tilt_range_table_exact"))
                                    .color(egui::Color32::from_rgb(120, 200, 120))
                            } else if factor > 1.0 {
                                egui::RichText::new(format!("×{factor:.0}"))
                                    .color(egui::Color32::from_rgb(255, 140, 60))
                            } else {
                                egui::RichText::new(format!("÷{:.0}", 1.0 / factor))
                                    .color(NOTICE_AMBER_TILT)
                            });
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            ui.label(t!("tilt_range_table_mismatch_read"));
            ui.add_space(10.0);

            ui.label(t!("tilt_range_table_reach_intro"));
            ui.add_space(4.0);
            egui::Grid::new("tilt_range_reach")
                .striped(true)
                .show(ui, |ui| {
                    ui.label("");
                    ui.label(egui::RichText::new(t!("tilt_range_table_full_scale")).strong());
                    ui.label(egui::RichText::new(t!("tilt_range_table_min")).strong());
                    ui.label(egui::RichText::new(t!("tilt_range_table_max")).strong());
                    ui.end_row();
                    for (i, range) in SENSOR_RANGES_G.into_iter().enumerate() {
                        let (min, max) = self.tilt_rings.reach[i];
                        let current = (range - self.tilt.nudge_range_g).abs() < 0.01;
                        let cell = |text: String| {
                            let rich = egui::RichText::new(text);
                            if current {
                                rich.strong()
                            } else {
                                rich
                            }
                        };
                        ui.label(cell(format!("{range:.0} G")));
                        ui.label(cell(format!("{:.1} m/s²", range * crate::tilt::GRAVITY)));
                        ui.label(cell(format!("{min:.2}°")));
                        ui.label(cell(format!("{max:.2}°")));
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(t!(
                    "tilt_range_table_reach_note",
                    pct = format!("{:.0}", self.tilt.nudge_scale_pct)
                ))
                .weak(),
            );
        });
    }

    /// The dial. Everything on it — dead zone, intent gate, tilt threshold,
    /// live reading — is drawn in one unit, acceleration as the sensor reads
    /// it, so distances on screen mean the same thing everywhere. The rim is
    /// full scale, which is the hard ceiling: a ring outside it names a
    /// setting the sensor can never satisfy.
    fn render_tilt_dial(&mut self, ui: &mut egui::Ui) {
        ui.label(t!("tilt_visualization"));
        ui.add_space(4.0);
        let viz_size = egui::vec2(260.0, 260.0);
        let (rect, _response) = ui.allocate_exact_size(viz_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        // Snap to the pixel grid: a centre landing on a half pixel smears the
        // 1px crosshair over two rows and the rings read as off-centre.
        let center = {
            use egui::emath::GuiRounding as _;
            rect.center().round_to_pixel_center(ui.pixels_per_point())
        };
        let radius = 118.0;
        let rings = self.tilt_rings;
        let to_radius = |accel: f32| radius * (accel / rings.full_scale).clamp(0.0, 1.0);

        // Rim: full scale.
        painter.circle_stroke(center, radius, egui::Stroke::new(2.0, egui::Color32::GRAY));
        for (dx, dy) in [(1.0, 0.0), (0.0, 1.0)] {
            painter.line_segment(
                [
                    center - egui::vec2(radius * dx, radius * dy),
                    center + egui::vec2(radius * dx, radius * dy),
                ],
                egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
            );
        }

        // Tilt threshold. Beyond the rim it is unreachable, so it is drawn on
        // the rim as a dashed ring rather than silently clamped — the ring has
        // to say "no shove gets here", not "shove this hard".
        if rings.tilt_out_of_reach() {
            painter.circle_stroke(
                center,
                radius - 2.0,
                egui::Stroke::new(2.0, RING_TILT.gamma_multiply(0.55)),
            );
            for step in 0..48 {
                let angle = std::f32::consts::TAU * (step as f32 + 0.25) / 48.0;
                let dir = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment(
                    [center + dir * (radius - 8.0), center + dir * (radius - 2.0)],
                    egui::Stroke::new(1.5, RING_TILT),
                );
            }
        } else {
            painter.circle_stroke(
                center,
                to_radius(rings.tilt),
                egui::Stroke::new(2.0, RING_TILT),
            );
        }

        // Intent gate — fixed at 1 m/s², but Strength applies before the
        // comparison, so raising it moves this ring inwards.
        if let Some(intent) = rings.intent {
            painter.circle_stroke(
                center,
                to_radius(intent),
                egui::Stroke::new(2.0, RING_INTENT),
            );
        }

        // Dead zone, innermost by nature: it gates the raw signal first.
        let deadzone_radius = to_radius(rings.deadzone);
        if deadzone_radius > 1.0 {
            painter.circle_filled(
                center,
                deadzone_radius,
                egui::Color32::from_rgba_unmultiplied(80, 200, 80, 40),
            );
            painter.circle_stroke(
                center,
                deadzone_radius,
                egui::Stroke::new(2.0, RING_DEADZONE),
            );
        }

        // The live sensor reading, thin and translucent — a laser dot that
        // shows movement everywhere, including inside the dead zone, which is
        // the one place the plumb can never show anything.
        let dot_pos = egui::pos2(
            center.x + self.accel_x.clamp(-1.05, 1.05) * radius,
            center.y + self.accel_y.clamp(-1.05, 1.05) * radius,
        );

        // The rim is the sensor's own ceiling: an axis reading its full scale
        // is a reading that was cut off there, and the board cannot say how
        // hard the real shove was. Sitting exactly on the rim and being clipped
        // by it look identical otherwise — the dot simply stops moving — so
        // clipping gets its own colour, and a mark that outlives the shove.
        let clipped = self.accel_x.abs() >= 1.0 || self.accel_y.abs() >= 1.0;
        if clipped {
            self.clip_marks.push(ClipMark {
                at: dot_pos,
                seen: std::time::Instant::now(),
            });
            if self.clip_marks.len() > CLIP_MARKS_KEPT {
                self.clip_marks.remove(0);
            }
        }

        // Marks fade out on their own: a nudge lasts a few milliseconds, far
        // less than it takes to look up from the cabinet.
        self.clip_marks
            .retain(|mark| mark.seen.elapsed() < CLIP_MARK_LIFE);
        for mark in &self.clip_marks {
            let left = 1.0 - mark.seen.elapsed().as_secs_f32() / CLIP_MARK_LIFE.as_secs_f32();
            painter.circle_stroke(
                mark.at,
                6.0,
                egui::Stroke::new(1.5, RING_CLIPPED.gamma_multiply(left.clamp(0.0, 1.0))),
            );
        }

        let laser = if clipped {
            RING_CLIPPED
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 60, 60, 210)
        };
        painter.circle_filled(
            dot_pos,
            if clipped { 9.0 } else { 7.0 },
            laser.gamma_multiply(0.18),
        );
        painter.circle_filled(dot_pos, if clipped { 3.0 } else { 2.0 }, laser);

        // A legend, because a ring nobody can name is decoration.
        ui.add_space(2.0);
        let legend = |ui: &mut egui::Ui, color: egui::Color32, text: String| {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter()
                    .circle_stroke(rect.center(), 5.0, egui::Stroke::new(2.0, color));
                ui.label(egui::RichText::new(text).small());
            });
        };
        legend(
            ui,
            egui::Color32::GRAY,
            t!(
                "tilt_legend_rim",
                accel = format!("{:.1}", rings.full_scale),
                range = format!("{:.0}", self.tilt.nudge_range_g)
            )
            .to_string(),
        );
        // Only worth a line when it has actually happened: a legend entry for
        // something nobody ever sees is noise.
        if !self.clip_marks.is_empty() {
            legend(ui, RING_CLIPPED, t!("tilt_legend_clipped").to_string());
        }
        legend(
            ui,
            RING_TILT,
            if rings.tilt_out_of_reach() {
                t!(
                    "tilt_legend_tilt_unreachable",
                    angle = format!("{:.2}", self.tilt.threshold_deg())
                )
                .to_string()
            } else {
                t!(
                    "tilt_legend_tilt",
                    angle = format!("{:.2}", self.tilt.threshold_deg()),
                    accel = format!("{:.1}", rings.tilt),
                    pct = format!("{:.0}", 100.0 * rings.tilt / rings.full_scale)
                )
                .to_string()
            },
        );
        if let Some(intent) = rings.intent {
            legend(
                ui,
                RING_INTENT,
                t!("tilt_legend_intent", accel = format!("{intent:.1}")).to_string(),
            );
        }
        legend(
            ui,
            RING_DEADZONE,
            t!(
                "tilt_legend_deadzone",
                accel = format!("{:.2}", rings.deadzone),
                pct = format!("{:.0}", self.tilt.nudge_deadzone_pct)
            )
            .to_string(),
        );

        // A banner naming what just happened. Rising is instant, falling
        // waits a second: a tilt that lasts one millisecond would otherwise
        // be gone before the eye arrives, and the point of shaking the
        // cabinet here is to see what the shake did.
        let now = std::time::Instant::now();
        let sensor_magnitude = self.accel_x.abs().max(self.accel_y.abs()) * rings.full_scale;
        let instant_state = if self.plumb_tilt_until.is_some_and(|t| now < t) {
            2
        } else if sensor_magnitude > rings.deadzone || self.nudge_sim.impulse_active() {
            // Either the raw signal cleared the dead zone, or the intent
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
                threshold = format!("{:.2}", self.tilt.threshold_deg())
            ))
            .weak(),
        );
    }
}
