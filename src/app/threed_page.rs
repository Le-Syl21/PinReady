use super::*;

impl App {
    /// The 3D page: two blocks, virtual reality and stereoscopic 3D. Each is
    /// off by default and only shows its options once it is switched on.
    pub(super) fn render_threed_page(&mut self, ui: &mut egui::Ui) {
        theme::section(
            ui,
            &t!("threed_section_vr"),
            Some(&t!("threed_section_vr_intro")),
            |ui| {
                egui::Grid::new("threed_vr_grid")
                    .min_col_width(230.0)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_vr_enable"));
                            help_marker(ui, &t!("threed_vr_enable_help"));
                        });
                        ui.checkbox(&mut self.threed.vr, t!("threed_vr_enable_hint").to_string());
                        ui.end_row();

                        if !self.threed.vr {
                            return;
                        }

                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_vr_mode"));
                            help_marker(ui, &t!("threed_vr_mode_help"));
                        });
                        egui::ComboBox::from_id_salt("vr_mode")
                            .selected_text(if self.threed.vr_always {
                                t!("threed_vr_mode_always").to_string()
                            } else {
                                t!("threed_vr_mode_detect").to_string()
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.threed.vr_always,
                                    false,
                                    t!("threed_vr_mode_detect").to_string(),
                                );
                                ui.selectable_value(
                                    &mut self.threed.vr_always,
                                    true,
                                    t!("threed_vr_mode_always").to_string(),
                                );
                            });
                        ui.end_row();

                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_vr_refresh"));
                            help_marker(ui, &t!("threed_vr_refresh_help"));
                        });
                        let refresh_label = |i: i32| match i {
                            1 => "72 Hz".to_string(),
                            2 => "80 Hz".to_string(),
                            3 => "90 Hz".to_string(),
                            4 => "120 Hz".to_string(),
                            _ => t!("threed_vr_refresh_default").to_string(),
                        };
                        egui::ComboBox::from_id_salt("vr_refresh")
                            .selected_text(refresh_label(self.threed.vr_refresh))
                            .show_ui(ui, |ui| {
                                for i in 0..=4 {
                                    ui.selectable_value(
                                        &mut self.threed.vr_refresh,
                                        i,
                                        refresh_label(i),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_vr_desktop"));
                            help_marker(ui, &t!("threed_vr_desktop_help"));
                        });
                        let desktop_label = |i: i32| match i {
                            0 => t!("threed_vr_desktop_off").to_string(),
                            2 => t!("threed_vr_desktop_playfield").to_string(),
                            3 => t!("threed_vr_desktop_left").to_string(),
                            4 => t!("threed_vr_desktop_right").to_string(),
                            5 => t!("threed_vr_desktop_both").to_string(),
                            _ => t!("threed_vr_desktop_all").to_string(),
                        };
                        egui::ComboBox::from_id_salt("vr_desktop")
                            .selected_text(desktop_label(self.threed.vr_desktop))
                            .show_ui(ui, |ui| {
                                for i in 0..=5 {
                                    ui.selectable_value(
                                        &mut self.threed.vr_desktop,
                                        i,
                                        desktop_label(i),
                                    );
                                }
                            });
                        ui.end_row();

                        let toggle = |ui: &mut egui::Ui,
                                      value: &mut bool,
                                      label: String,
                                      help: String,
                                      hint: String| {
                            ui.horizontal(|ui| {
                                theme::field_label(ui, &label);
                                help_marker(ui, &help);
                            });
                            ui.checkbox(value, hint);
                            ui.end_row();
                        };
                        toggle(
                            ui,
                            &mut self.threed.vr_shrink,
                            t!("threed_vr_shrink").to_string(),
                            t!("threed_vr_shrink_help").to_string(),
                            t!("threed_vr_shrink_hint").to_string(),
                        );
                        toggle(
                            ui,
                            &mut self.threed.vr_backglass,
                            t!("threed_vr_backglass").to_string(),
                            t!("threed_vr_backglass_help").to_string(),
                            t!("threed_vr_backglass_hint").to_string(),
                        );
                        toggle(
                            ui,
                            &mut self.threed.vr_audio_backglass,
                            t!("threed_vr_audio_bg").to_string(),
                            t!("threed_vr_audio_bg_help").to_string(),
                            t!("threed_vr_audio_hint").to_string(),
                        );
                        toggle(
                            ui,
                            &mut self.threed.vr_audio_playfield,
                            t!("threed_vr_audio_pf").to_string(),
                            t!("threed_vr_audio_pf_help").to_string(),
                            t!("threed_vr_audio_hint").to_string(),
                        );
                    });
            },
        );

        theme::section(
            ui,
            &t!("threed_section_stereo"),
            Some(&t!("threed_section_stereo_intro")),
            |ui| {
                egui::Grid::new("threed_stereo_grid")
                    .min_col_width(230.0)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_stereo_enable"));
                            help_marker(ui, &t!("threed_stereo_enable_help"));
                        });
                        let mut on = self.threed.stereo > 0;
                        if ui
                            .checkbox(&mut on, t!("threed_stereo_enable_hint").to_string())
                            .changed()
                        {
                            self.threed.stereo = if on { threed::STEREO_FIRST } else { 0 };
                        }
                        ui.end_row();

                        if self.threed.stereo == 0 {
                            return;
                        }

                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_stereo_mode"));
                            help_marker(ui, &t!("threed_stereo_mode_help"));
                        });
                        let mode_label = |m: i32| match m {
                            1 => t!("threed_stereo_tb").to_string(),
                            2 => t!("threed_stereo_interlaced").to_string(),
                            3 => t!("threed_stereo_interlaced_flipped").to_string(),
                            4 => t!("threed_stereo_sbs").to_string(),
                            5 => t!("threed_stereo_red_cyan").to_string(),
                            6 => t!("threed_stereo_green_magenta").to_string(),
                            7 => t!("threed_stereo_blue_amber").to_string(),
                            8 => t!("threed_stereo_cyan_red").to_string(),
                            9 => t!("threed_stereo_magenta_green").to_string(),
                            _ => t!("threed_stereo_amber_blue").to_string(),
                        };
                        egui::ComboBox::from_id_salt("stereo_mode")
                            .selected_text(mode_label(self.threed.stereo))
                            .show_ui(ui, |ui| {
                                for m in threed::STEREO_FIRST..=threed::STEREO_LAST {
                                    ui.selectable_value(&mut self.threed.stereo, m, mode_label(m));
                                }
                            });
                        ui.end_row();

                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_stereo_eye"));
                            help_marker(ui, &t!("threed_stereo_eye_help"));
                        });
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.threed.stereo_eye_mm)
                                    .range(5.0..=200.0)
                                    .speed(0.5)
                                    .suffix(" mm"),
                            );
                            ui.label(t!("threed_stereo_eye_hint").to_string());
                        });
                        ui.end_row();

                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_stereo_brightness"));
                            help_marker(ui, &t!("threed_stereo_brightness_help"));
                        });
                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(
                                &mut self.threed.stereo_brightness,
                                0.0..=2.0,
                            ));
                            ui.label(t!("threed_stereo_neutral_hint").to_string());
                        });
                        ui.end_row();

                        ui.horizontal(|ui| {
                            theme::field_label(ui, &t!("threed_stereo_saturation"));
                            help_marker(ui, &t!("threed_stereo_saturation_help"));
                        });
                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(
                                &mut self.threed.stereo_saturation,
                                0.0..=2.0,
                            ));
                            ui.label(t!("threed_stereo_neutral_hint").to_string());
                        });
                        ui.end_row();
                    });
            },
        );

        if self.threed.vr && self.threed.stereo > 0 {
            ui.add_space(8.0);
            ui.colored_label(ui.visuals().warn_fg_color, t!("threed_both_warning"));
        }
    }
}
