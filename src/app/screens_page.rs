use super::*;

impl App {
    pub(super) fn render_screens_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("screens_heading"));
        ui.add_space(8.0);

        // Language selector
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Langue / Language").size(14.0));
            ui.add_space(8.0);
            let prev_lang = self.selected_language;
            let current_label = LANGUAGE_OPTIONS
                .get(self.selected_language)
                .map(|(_, l)| *l)
                .unwrap_or("English");
            egui::ComboBox::from_id_salt("lang_combo")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (idx, (_code, label)) in LANGUAGE_OPTIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_language, idx, *label);
                    }
                });
            if self.selected_language != prev_lang {
                let (code, _) = LANGUAGE_OPTIONS[self.selected_language];
                i18n::set_locale(code);
                let _ = self.db.set_config("language", code);
            }
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Visual Pinball installation
        ui.label(egui::RichText::new(t!("vpx_install_title")).strong());
        ui.add_space(4.0);

        ui.radio_value(
            &mut self.vpx_install_mode,
            VpxInstallMode::Auto,
            t!("vpx_auto_install"),
        );
        if self.vpx_install_mode == VpxInstallMode::Auto {
            ui.indent("auto_install", |ui| {
                if let Some(ref release) = self.vpx_latest_release {
                    ui.label(t!("vpx_version_available", tag = release.tag.as_str()));
                    let size_mb = release.asset_size / (1024 * 1024);
                    ui.label(t!(
                        "vpx_artifact_info",
                        name = release.asset_name.as_str(),
                        size = size_mb
                    ));
                } else if self.update_check_rx.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t!("vpx_checking"));
                    });
                    ui.ctx().request_repaint();
                } else if !self.vpx_installed_tag.is_empty() {
                    ui.label(t!(
                        "vpx_version_installed",
                        tag = self.vpx_installed_tag.as_str()
                    ));
                } else {
                    ui.label(t!("vpx_no_version"));
                }

                if self.update_downloading {
                    let (current, total) = self.update_progress;
                    if total > 0 {
                        let pct = current as f32 / total as f32;
                        let mb = current / (1024 * 1024);
                        let total_mb = total / (1024 * 1024);
                        ui.add(egui::ProgressBar::new(pct).text(format!("{mb}/{total_mb} Mo")));
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(t!("vpx_extracting"));
                        });
                    }
                    ui.ctx().request_repaint();
                } else if let Some(release) = self.vpx_latest_release.clone() {
                    if release.tag != self.vpx_installed_tag
                        && ui.button(t!("vpx_install_button")).clicked()
                    {
                        self.start_vpx_download(&release);
                    }
                }

                if let Some(ref err) = self.update_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 100, 100),
                        t!("vpx_install_error", msg = err.as_str()),
                    );
                }

                ui.add_space(4.0);
                ui.label(t!("vpx_install_dir_label"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.vpx_install_dir).desired_width(400.0),
                    );
                    if ui.button(t!("vpx_browse")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title(t!("vpx_install_dir_picker"))
                            .set_directory(&self.vpx_install_dir)
                            .pick_folder()
                        {
                            self.vpx_install_dir = path.display().to_string();
                        }
                    }
                });
                ui.label(egui::RichText::new(t!("vpx_install_dir_hint")).weak());
            });
        }

        ui.radio_value(
            &mut self.vpx_install_mode,
            VpxInstallMode::Manual,
            t!("vpx_manual_install"),
        );
        if self.vpx_install_mode == VpxInstallMode::Manual {
            ui.indent("manual_install", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.vpx_exe_path).desired_width(400.0));
                    if ui.button(t!("vpx_browse")).clicked() {
                        // On macOS, .app bundles are directories — use pick_folder
                        // as fallback so users can select them.
                        let picked = if cfg!(target_os = "macos") {
                            rfd::FileDialog::new()
                                .set_title(t!("vpx_file_picker"))
                                .pick_file()
                                .or_else(|| {
                                    rfd::FileDialog::new()
                                        .set_title(t!("vpx_file_picker"))
                                        .pick_folder()
                                })
                        } else {
                            rfd::FileDialog::new()
                                .set_title(t!("vpx_file_picker"))
                                .pick_file()
                        };
                        if let Some(path) = picked {
                            self.vpx_exe_path = path.display().to_string();
                        }
                    }
                });
                let resolved = updater::resolve_vpx_exe(std::path::Path::new(&self.vpx_exe_path));
                let vpx_exists = resolved.is_file();
                if !vpx_exists && !self.vpx_exe_path.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 100, 100),
                        format!("⚠ {}", t!("vpx_file_not_found")),
                    );
                }
            });
        }

        ui.add_space(4.0);
        ui.label(egui::RichText::new(t!("vpx_validated_on")).weak().italics());
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // Screen count selection
        ui.label(t!("screens_count_label"));
        ui.horizontal(|ui| {
            for n in 1..=4 {
                let label = match n {
                    1 => t!("screens_1"),
                    2 => t!("screens_2"),
                    3 => t!("screens_3"),
                    _ => t!("screens_4"),
                };
                if ui.radio_value(&mut self.screen_count, n, label).changed() {
                    // Re-assign roles based on new screen count
                    for (i, display) in self.displays.iter_mut().enumerate() {
                        let roles = [
                            DisplayRole::Playfield,
                            DisplayRole::Backglass,
                            DisplayRole::Dmd,
                            DisplayRole::Topper,
                        ];
                        display.role = if i < self.screen_count {
                            roles.get(i).copied().unwrap_or(DisplayRole::Unused)
                        } else {
                            DisplayRole::Unused
                        };
                    }
                }
            }
        });

        ui.add_space(8.0);

        // View mode
        ui.label(t!("screens_display_mode"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.view_mode, 0, t!("screens_mode_desktop"));
            ui.radio_value(&mut self.view_mode, 1, t!("screens_mode_cabinet"));
            ui.radio_value(&mut self.view_mode, 2, t!("screens_mode_fss"));
        });

        ui.add_space(8.0);

        ui.checkbox(&mut self.disable_touch, t!("screens_disable_touch"));

        // External DMD checkbox — only relevant when no screen has DMD role
        let has_dmd_screen = self.displays.iter().any(|d| d.role == DisplayRole::Dmd);
        if !has_dmd_screen {
            ui.checkbox(&mut self.external_dmd, t!("screens_external_dmd"));
            ui.label(egui::RichText::new(t!("screens_external_dmd_hint")).weak());
        } else {
            self.external_dmd = false;
        }

        ui.add_space(12.0);

        // Display table
        if self.displays.is_empty() {
            ui.label(t!("screens_no_displays"));
            return;
        }

        egui::Grid::new("displays_grid")
            .striped(true)
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.strong(t!("screens_col_screen"));
                ui.strong(t!("screens_col_resolution"));
                ui.strong(t!("screens_col_hz"));
                ui.strong(t!("screens_col_size"));
                ui.strong(t!("screens_col_role"));
                ui.end_row();

                let available_roles: Vec<DisplayRole> = DisplayRole::all()
                    .iter()
                    .copied()
                    .filter(|r| {
                        *r == DisplayRole::Unused
                            || match r {
                                DisplayRole::Playfield => self.screen_count >= 1,
                                DisplayRole::Backglass => self.screen_count >= 2,
                                DisplayRole::Dmd => self.screen_count >= 3,
                                DisplayRole::Topper => self.screen_count >= 4,
                                DisplayRole::Unused => true,
                            }
                    })
                    .collect();

                for display in &mut self.displays {
                    // Name + inches
                    let label = if let Some(inches) = display.size_inches {
                        format!("{} ({}\")", display.name, inches)
                    } else {
                        display.name.clone()
                    };
                    ui.label(&label);
                    ui.label(format!("{}x{}", display.width, display.height));
                    ui.label(format!("{:.0} Hz", display.refresh_rate));
                    // Physical size mm (editable)
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut display.width_mm)
                                .speed(1)
                                .suffix(" mm"),
                        );
                        ui.label("x");
                        ui.add(
                            egui::DragValue::new(&mut display.height_mm)
                                .speed(1)
                                .suffix(" mm"),
                        );
                    });

                    egui::ComboBox::from_id_salt(format!("role_{}", display.name))
                        .selected_text(display.role.label())
                        .show_ui(ui, |ui| {
                            for role in &available_roles {
                                ui.selectable_value(&mut display.role, *role, role.label());
                            }
                        });
                    ui.end_row();
                }
            });

        if self.view_mode == 1 {
            self.render_cabinet_diagram(ui);
        }
    }

    /// Render the interactive side-view cabinet diagram with drag handles.
    fn render_cabinet_diagram(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);
        ui.strong(t!("cabinet_dimensions"));
        ui.add_space(4.0);
        ui.label(t!("cabinet_drag_hint"));
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            self.render_cabinet_schema(ui);
            self.render_cabinet_values(ui);
        });
    }

    /// Draw the interactive side-view schema (cabinet, player, drag handles).
    fn render_cabinet_schema(&mut self, ui: &mut egui::Ui) {
        let schema_size = egui::vec2(450.0, 500.0);
        let (rect, response) = ui.allocate_exact_size(schema_size, egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        let scale = 2.0_f32;
        let ground_y = rect.bottom() - 25.0;
        let cab_x = rect.left() + 220.0;

        let col_cab = egui::Color32::from_rgb(120, 80, 50);
        let col_screen = egui::Color32::from_rgb(60, 120, 200);
        let col_player = egui::Color32::from_rgb(80, 180, 80);
        let col_dim = egui::Color32::from_rgb(200, 60, 60);
        let col_ground = egui::Color32::GRAY;
        let col_handle = egui::Color32::from_rgb(255, 200, 0);

        // Ground
        painter.line_segment(
            [
                egui::pos2(rect.left() + 10.0, ground_y),
                egui::pos2(rect.right() - 10.0, ground_y),
            ],
            egui::Stroke::new(2.0, col_ground),
        );
        painter.text(
            egui::pos2(rect.right() - 30.0, ground_y + 8.0),
            egui::Align2::CENTER_CENTER,
            &t!("cabinet_ground"),
            egui::FontId::proportional(10.0),
            col_ground,
        );

        let lockbar_y = ground_y - self.lockbar_height * scale;
        let font_label = egui::FontId::proportional(12.0);
        let font_dim = egui::FontId::proportional(11.0);

        // Screen
        let screen_len_px = 150.0;
        let incl_rad = self.screen_inclination.to_radians();
        let screen_end_x = cab_x + screen_len_px * incl_rad.cos();
        let screen_end_y = lockbar_y - screen_len_px * incl_rad.sin();

        // Cabinet legs
        painter.line_segment(
            [egui::pos2(cab_x, ground_y), egui::pos2(cab_x, lockbar_y)],
            egui::Stroke::new(3.0, col_cab),
        );
        painter.line_segment(
            [
                egui::pos2(screen_end_x, ground_y),
                egui::pos2(screen_end_x, screen_end_y),
            ],
            egui::Stroke::new(3.0, col_cab),
        );

        // Lockbar
        painter.line_segment(
            [
                egui::pos2(cab_x - 15.0, lockbar_y),
                egui::pos2(cab_x + 15.0, lockbar_y),
            ],
            egui::Stroke::new(5.0, col_cab),
        );
        painter.text(
            egui::pos2(cab_x, lockbar_y + 12.0),
            egui::Align2::CENTER_TOP,
            "Lockbar",
            font_label.clone(),
            col_cab,
        );

        // Playfield screen
        painter.line_segment(
            [
                egui::pos2(cab_x, lockbar_y),
                egui::pos2(screen_end_x, screen_end_y),
            ],
            egui::Stroke::new(6.0, col_screen),
        );
        painter.text(
            egui::pos2(
                (cab_x + screen_end_x) / 2.0,
                (lockbar_y + screen_end_y) / 2.0 - 14.0,
            ),
            egui::Align2::CENTER_CENTER,
            "Playfield",
            font_label.clone(),
            col_screen,
        );

        // Backglass
        let bg_height = 80.0;
        painter.line_segment(
            [
                egui::pos2(screen_end_x, screen_end_y),
                egui::pos2(screen_end_x, screen_end_y - bg_height),
            ],
            egui::Stroke::new(4.0, col_screen.linear_multiply(0.6)),
        );
        painter.text(
            egui::pos2(screen_end_x + 8.0, screen_end_y - bg_height / 2.0),
            egui::Align2::LEFT_CENTER,
            "BG",
            font_label,
            col_screen,
        );

        // Player stick figure
        let player_base_x = cab_x - (-self.player_y) * scale;
        let player_feet_y = ground_y;
        let player_head_y = ground_y - self.player_height * scale;
        let player_hip_y = ground_y - self.player_height * scale * 0.45;
        let player_shoulder_y = ground_y - self.player_height * scale * 0.72;
        let player_neck_y = ground_y - self.player_height * scale * 0.82;
        let head_radius = 10.0;
        let head_center_y = player_head_y + head_radius;
        let leg_spread = 14.0;
        let stroke = egui::Stroke::new(3.0, col_player);

        // Legs
        painter.line_segment(
            [
                egui::pos2(player_base_x + leg_spread, player_feet_y),
                egui::pos2(player_base_x + 2.0, player_hip_y),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(player_base_x - leg_spread, player_feet_y),
                egui::pos2(player_base_x - 2.0, player_hip_y),
            ],
            stroke,
        );
        // Torso
        painter.line_segment(
            [
                egui::pos2(player_base_x, player_hip_y),
                egui::pos2(player_base_x + 3.0, player_neck_y),
            ],
            stroke,
        );
        // Head + eye
        painter.circle_filled(
            egui::pos2(player_base_x + 3.0, head_center_y),
            head_radius,
            col_player,
        );
        painter.circle_filled(
            egui::pos2(player_base_x + 7.0, head_center_y - 2.0),
            2.0,
            egui::Color32::WHITE,
        );
        // Arms
        painter.line_segment(
            [
                egui::pos2(player_base_x + 3.0, player_shoulder_y),
                egui::pos2(player_base_x + 20.0, player_shoulder_y + 15.0),
            ],
            stroke,
        );

        // Dimension arrows
        let arrow_x = cab_x + 50.0;
        painter.line_segment(
            [
                egui::pos2(arrow_x, ground_y),
                egui::pos2(arrow_x, lockbar_y),
            ],
            egui::Stroke::new(1.5, col_dim),
        );
        painter.text(
            egui::pos2(arrow_x + 5.0, (ground_y + lockbar_y) / 2.0),
            egui::Align2::LEFT_CENTER,
            format!("{:.0} cm", self.lockbar_height),
            font_dim.clone(),
            col_dim,
        );

        let parrow_x = player_base_x - 25.0;
        painter.line_segment(
            [
                egui::pos2(parrow_x, player_feet_y),
                egui::pos2(parrow_x, player_head_y),
            ],
            egui::Stroke::new(1.5, col_player),
        );
        painter.text(
            egui::pos2(parrow_x - 5.0, (player_feet_y + player_head_y) / 2.0),
            egui::Align2::RIGHT_CENTER,
            format!("{:.0} cm", self.player_height),
            font_dim.clone(),
            col_player,
        );

        painter.line_segment(
            [
                egui::pos2(player_base_x, lockbar_y + 12.0),
                egui::pos2(cab_x, lockbar_y + 12.0),
            ],
            egui::Stroke::new(1.0, col_dim),
        );
        painter.text(
            egui::pos2((player_base_x + cab_x) / 2.0, lockbar_y + 24.0),
            egui::Align2::CENTER_CENTER,
            format!("Y={:.0} cm", self.player_y),
            font_dim.clone(),
            col_dim,
        );

        if self.screen_inclination.abs() > 0.5 {
            painter.text(
                egui::pos2(cab_x + 30.0, lockbar_y - 12.0),
                egui::Align2::LEFT_CENTER,
                format!("{:.0} deg", self.screen_inclination),
                font_dim,
                col_screen,
            );
        }

        // Drag handles
        let handle_radius = 6.0;
        let h_lockbar = egui::pos2(cab_x, lockbar_y);
        let h_head = egui::pos2(player_base_x, player_head_y);
        let h_waist = egui::pos2(player_base_x, player_hip_y);
        let h_screen = egui::pos2(screen_end_x, screen_end_y);
        painter.circle_filled(h_lockbar, handle_radius, col_handle);
        painter.circle_filled(h_head, handle_radius, col_handle);
        painter.circle_filled(h_waist, handle_radius, col_handle);
        painter.circle_filled(h_screen, handle_radius, col_handle);

        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let dist = |p: egui::Pos2| ((pos.x - p.x).powi(2) + (pos.y - p.y).powi(2)).sqrt();
                if dist(h_lockbar) < 30.0 {
                    self.lockbar_height = ((ground_y - pos.y) / scale).clamp(0.0, 250.0);
                } else if dist(h_head) < 30.0 {
                    self.player_height = ((ground_y - pos.y) / scale).clamp(75.0, 250.0);
                } else if dist(h_waist) < 30.0 {
                    self.player_y = (-(cab_x - pos.x) / scale).clamp(-70.0, 30.0);
                } else if dist(h_screen) < 30.0 {
                    let dx = pos.x - cab_x;
                    let dy = lockbar_y - pos.y;
                    self.screen_inclination = dy.atan2(dx).to_degrees().clamp(-30.0, 30.0);
                }
            }
        }

        self.player_z = (self.player_height - 12.0 - self.lockbar_height).max(0.0);
    }

    /// Render the cabinet dimension values panel (right side of the diagram).
    fn render_cabinet_values(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.strong(t!("cabinet_values"));
            ui.add_space(8.0);

            ui.label(t!("cabinet_lockbar_width"));
            ui.add(
                egui::DragValue::new(&mut self.lockbar_width)
                    .range(10.0..=150.0)
                    .speed(1.0)
                    .suffix(" cm"),
            );
            ui.add_space(4.0);

            ui.label(t!("cabinet_lockbar_height"));
            ui.add(
                egui::DragValue::new(&mut self.lockbar_height)
                    .range(0.0..=250.0)
                    .speed(1.0)
                    .suffix(" cm"),
            );
            ui.add_space(4.0);

            ui.label(t!("cabinet_screen_inclination"));
            ui.add(
                egui::DragValue::new(&mut self.screen_inclination)
                    .range(-30.0..=30.0)
                    .speed(0.5)
                    .suffix(" deg"),
            );
            ui.add_space(4.0);

            ui.label(t!("cabinet_player_height"));
            ui.add(
                egui::DragValue::new(&mut self.player_height)
                    .range(75.0..=250.0)
                    .speed(1.0)
                    .suffix(" cm"),
            );
            ui.add_space(4.0);

            ui.label(t!("cabinet_player_distance"));
            ui.add(
                egui::DragValue::new(&mut self.player_y)
                    .range(-70.0..=30.0)
                    .speed(1.0)
                    .suffix(" cm"),
            );
            ui.add_space(4.0);

            ui.label(t!("cabinet_player_offset"));
            ui.add(
                egui::DragValue::new(&mut self.player_x)
                    .range(-30.0..=30.0)
                    .speed(1.0)
                    .suffix(" cm"),
            );
            ui.add_space(12.0);

            ui.separator();
            ui.label(t!(
                "cabinet_eye_height",
                value = format!("{:.0}", self.player_z)
            ));
            ui.label(t!("cabinet_eye_formula"));
        });
    }
}
