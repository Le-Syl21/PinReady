use super::*;

impl App {
    pub(super) fn render_rendering_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("rendering_heading"));
        ui.add_space(4.0);
        ui.label(t!("rendering_desc"));
        ui.add_space(12.0);

        egui::Grid::new("rendering_grid")
            .min_col_width(250.0)
            .striped(true)
            .show(ui, |ui| {
                // Sync mode
                ui.label(t!("rendering_sync"));
                egui::ComboBox::from_id_salt("sync_mode")
                    .selected_text(match self.sync_mode {
                        0 => t!("rendering_sync_none").to_string(),
                        1 => t!("rendering_sync_vsync").to_string(),
                        _ => t!("rendering_sync_vsync").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.sync_mode,
                            0,
                            t!("rendering_sync_none").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.sync_mode,
                            1,
                            t!("rendering_sync_vsync_default").to_string(),
                        );
                    });
                ui.end_row();

                // Max framerate — auto-set from playfield refresh rate
                ui.label(t!("rendering_fps_limit"));
                let pf_refresh = self
                    .displays
                    .iter()
                    .find(|d| d.role == DisplayRole::Playfield)
                    .map(|d| d.refresh_rate)
                    .unwrap_or(60.0);
                self.max_framerate = pf_refresh;
                ui.label(t!("rendering_fps_info", hz = format!("{:.2}", pf_refresh)));
                ui.end_row();

                // Supersampling
                ui.label(t!("rendering_supersampling"));
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.aa_factor, 0.5..=2.0).step_by(0.25));
                    let tip = if self.aa_factor < 0.8 {
                        t!("rendering_aa_perf")
                    } else if self.aa_factor <= 1.1 {
                        t!("rendering_aa_default")
                    } else if self.aa_factor <= 1.5 {
                        t!("rendering_aa_quality")
                    } else {
                        t!("rendering_aa_quality_heavy")
                    };
                    ui.label(tip.to_string());
                });
                ui.end_row();

                // MSAA
                ui.label(t!("rendering_msaa"));
                egui::ComboBox::from_id_salt("msaa")
                    .selected_text(match self.msaa {
                        0 => t!("rendering_msaa_off").to_string(),
                        1 => t!("rendering_msaa_4").to_string(),
                        2 => t!("rendering_msaa_6").to_string(),
                        3 => t!("rendering_msaa_8").to_string(),
                        _ => t!("rendering_msaa_off").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.msaa,
                            0,
                            t!("rendering_msaa_off_default").to_string(),
                        );
                        ui.selectable_value(&mut self.msaa, 1, t!("rendering_msaa_4").to_string());
                        ui.selectable_value(&mut self.msaa, 2, t!("rendering_msaa_6").to_string());
                        ui.selectable_value(&mut self.msaa, 3, t!("rendering_msaa_8").to_string());
                    });
                ui.end_row();

                // Post-process AA
                ui.label(t!("rendering_fxaa"));
                egui::ComboBox::from_id_salt("fxaa")
                    .selected_text(match self.fxaa {
                        0 => t!("rendering_fxaa_off").to_string(),
                        1 => t!("rendering_fxaa_fast").to_string(),
                        2 => t!("rendering_fxaa_standard").to_string(),
                        3 => t!("rendering_fxaa_quality").to_string(),
                        4 => t!("rendering_fxaa_nfaa").to_string(),
                        5 => t!("rendering_fxaa_dlaa").to_string(),
                        6 => t!("rendering_fxaa_smaa").to_string(),
                        7 => t!("rendering_fxaa_faaa").to_string(),
                        _ => t!("rendering_fxaa_off").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.fxaa,
                            0,
                            t!("rendering_fxaa_off").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            1,
                            t!("rendering_fxaa_fast").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            2,
                            t!("rendering_fxaa_standard").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            3,
                            t!("rendering_fxaa_quality").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            4,
                            t!("rendering_fxaa_nfaa").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            5,
                            t!("rendering_fxaa_dlaa").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            6,
                            t!("rendering_fxaa_smaa").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.fxaa,
                            7,
                            t!("rendering_fxaa_faaa").to_string(),
                        );
                    });
                ui.end_row();

                // Sharpening
                ui.label(t!("rendering_sharpen"));
                egui::ComboBox::from_id_salt("sharpen")
                    .selected_text(match self.sharpen {
                        0 => t!("rendering_sharpen_off").to_string(),
                        1 => t!("rendering_sharpen_cas").to_string(),
                        2 => t!("rendering_sharpen_bilateral").to_string(),
                        _ => t!("rendering_sharpen_off").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.sharpen,
                            0,
                            t!("rendering_sharpen_off").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.sharpen,
                            1,
                            t!("rendering_sharpen_cas").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.sharpen,
                            2,
                            t!("rendering_sharpen_bilateral").to_string(),
                        );
                    });
                ui.end_row();

                // Reflections
                ui.label(t!("rendering_reflections"));
                egui::ComboBox::from_id_salt("pf_reflection")
                    .selected_text(match self.pf_reflection {
                        0 => t!("rendering_reflect_off").to_string(),
                        1 => t!("rendering_reflect_balls").to_string(),
                        2 => t!("rendering_reflect_static").to_string(),
                        3 => t!("rendering_reflect_static_balls").to_string(),
                        4 => t!("rendering_reflect_static_dynamic").to_string(),
                        5 => t!("rendering_reflect_dynamic").to_string(),
                        _ => t!("rendering_reflect_dynamic").to_string(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            0,
                            t!("rendering_reflect_off_perf").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            1,
                            t!("rendering_reflect_balls").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            2,
                            t!("rendering_reflect_static_zero").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            3,
                            t!("rendering_reflect_static_balls").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            4,
                            t!("rendering_reflect_static_dynamic").to_string(),
                        );
                        ui.selectable_value(
                            &mut self.pf_reflection,
                            5,
                            t!("rendering_reflect_dynamic_default").to_string(),
                        );
                    });
                ui.end_row();

                // Max texture dimension
                ui.label(t!("rendering_tex_size"));
                egui::ComboBox::from_id_salt("max_tex")
                    .selected_text(self.max_tex_dim.to_string())
                    .show_ui(ui, |ui| {
                        for &size in &[512, 1024, 2048, 4096, 8192, 16384] {
                            let label = if size == 16384 {
                                t!("rendering_tex_default").to_string()
                            } else {
                                format!("{size}")
                            };
                            ui.selectable_value(&mut self.max_tex_dim, size, label);
                        }
                    });
                ui.end_row();
            });
    }
}
