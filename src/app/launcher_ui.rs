use super::*;

impl App {
    #[allow(deprecated)]
    pub(super) fn render_launcher(&mut self, ui: &mut egui::Ui) {
        // Install image loaders once
        egui_extras::install_image_loaders(ui.ctx());

        self.process_bg_extraction(ui.ctx());
        self.preload_images(ui.ctx());
        self.handle_launcher_joystick(ui);
        self.process_vpx_status();
        self.process_update_check();
        // Only repaint when needed: bg extraction in progress, VPX running, joystick connected, or update in progress
        if self.bg_rx.is_some()
            || self.vpx_running.load(Ordering::Relaxed)
            || self.joystick_rx.is_some()
            || self.update_downloading
            || self.update_check_rx.is_some()
        {
            ui.ctx().request_repaint();
        }

        // Keyboard navigation in launcher
        if !self.tables.is_empty() && !self.vpx_running.load(Ordering::Relaxed) {
            let len = self.tables.len();
            let cols = self.launcher_cols.max(1);
            ui.input(|i| {
                for event in &i.events {
                    if let egui::Event::Key {
                        key, pressed: true, ..
                    } = event
                    {
                        match key {
                            egui::Key::ArrowLeft => {
                                self.selected_table = if self.selected_table > 0 {
                                    self.selected_table - 1
                                } else {
                                    len - 1
                                };
                                self.scroll_to_selected = true;
                            }
                            egui::Key::ArrowRight => {
                                self.selected_table = (self.selected_table + 1) % len;
                                self.scroll_to_selected = true;
                            }
                            egui::Key::ArrowUp => {
                                self.selected_table = if self.selected_table >= cols {
                                    self.selected_table - cols
                                } else {
                                    self.selected_table
                                };
                                self.scroll_to_selected = true;
                            }
                            egui::Key::ArrowDown => {
                                self.selected_table = if self.selected_table + cols < len {
                                    self.selected_table + cols
                                } else {
                                    self.selected_table
                                };
                                self.scroll_to_selected = true;
                            }
                            egui::Key::Enter => {
                                let path = self.tables[self.selected_table].path.clone();
                                self.launch_table(&path);
                            }
                            egui::Key::Escape => {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            _ => {}
                        }
                    }
                }
            });
        }

        // Multi-screen: position main window (table selector) on the right display
        // 2 screens: selector on Playfield, BG viewport on Backglass
        // 3+ screens: selector on DMD, BG viewport on Backglass, cover on Playfield (+Topper)
        let has_dmd = self.displays.iter().any(|d| d.role == DisplayRole::Dmd);
        let has_bg = self
            .displays
            .iter()
            .any(|d| d.role == DisplayRole::Backglass);
        if has_dmd {
            // 3+ screens: main window on DMD
            if let Some(dmd) = self.displays.iter().find(|d| d.role == DisplayRole::Dmd) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        dmd.x as f32,
                        dmd.y as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        dmd.width as f32,
                        dmd.height as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            }
        } else if has_bg {
            // 2 screens (PF + BG): main window on Playfield
            if let Some(pf) = self
                .displays
                .iter()
                .find(|d| d.role == DisplayRole::Playfield)
            {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        pf.x as f32,
                        pf.y as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        pf.width as f32,
                        pf.height as f32,
                    )));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            }
        }

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("launcher_title"))
                    .size(24.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("launcher_quit")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button(t!("launcher_config")).clicked() {
                    self.mode = AppMode::Wizard;
                }
                // Update button — visible when a new release is available
                if self.update_downloading {
                    let (current, total) = self.update_progress;
                    if total > 0 {
                        let pct = (current as f32 / total as f32 * 100.0) as u32;
                        let mb = current / (1024 * 1024);
                        let total_mb = total / (1024 * 1024);
                        ui.add(
                            egui::ProgressBar::new(current as f32 / total as f32).text(t!(
                                "update_progress",
                                mb = mb,
                                total = total_mb,
                                pct = pct
                            )),
                        );
                    } else {
                        ui.spinner();
                        ui.label(t!("update_extracting"));
                    }
                } else if let Some(release) = self.vpx_latest_release.clone() {
                    let btn = ui.button(
                        egui::RichText::new(t!("update_button", tag = release.tag.as_str()))
                            .color(egui::Color32::from_rgb(100, 200, 100)),
                    );
                    if btn.clicked() {
                        self.start_vpx_download(&release);
                    }
                }
                if let Some(ref err) = self.update_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 100, 100),
                        t!("update_error", msg = err.as_str()),
                    );
                }
                let rescan_label = if let Some(start) = self.rescan_press_start {
                    let held = start.elapsed().as_secs_f32();
                    let pct = ((held / 3.0) * 100.0).min(100.0) as u32;
                    t!("launcher_reset_pct", pct = pct).to_string()
                } else {
                    t!("launcher_rescan").to_string()
                };
                let rescan_btn = ui.button(&rescan_label);
                if rescan_btn.is_pointer_button_down_on() {
                    if self.rescan_press_start.is_none() {
                        self.rescan_press_start = Some(std::time::Instant::now());
                    }
                    if let Some(start) = self.rescan_press_start {
                        if start.elapsed().as_secs_f32() >= 3.0 {
                            // Long press: delete all caches and full rescan
                            log::info!("Long press: full backglass regeneration");
                            self.rescan_press_start = None;
                            let dir = std::path::Path::new(&self.tables_dir);
                            if dir.is_dir() {
                                for entry in walkdir::WalkDir::new(dir)
                                    .max_depth(2)
                                    .into_iter()
                                    .flatten()
                                {
                                    let p = entry.path();
                                    if p.file_name()
                                        .and_then(|f| f.to_str())
                                        .is_some_and(|f| f.starts_with(".pinready_bg_v"))
                                    {
                                        let _ = std::fs::remove_file(p);
                                    }
                                }
                            }
                            self.scan_tables();
                        } else {
                            ui.ctx().request_repaint();
                        }
                    }
                } else if self.rescan_press_start.is_some() {
                    // Released before 3s: incremental rescan
                    self.rescan_press_start = None;
                    self.scan_tables();
                }
            });
        });
        ui.add_space(8.0);

        // VPX loading overlay — show spinner but don't return, viewports need to render below
        let vpx_loading =
            self.vpx_running.load(Ordering::Relaxed) && !self.vpx_loading_msg.is_empty();
        if vpx_loading {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.spinner();
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(&self.vpx_loading_msg)
                        .size(18.0)
                        .strong(),
                );
            });
        }

        // VPX error popup
        if self.vpx_error_log.is_some() {
            let mut close = false;
            egui::Window::new(t!("launcher_error_title").to_string())
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_size([600.0, 400.0])
                .show(ui.ctx(), |ui| {
                    ui.label(
                        egui::RichText::new(t!("launcher_vpx_crashed").to_string())
                            .size(16.0)
                            .strong()
                            .color(egui::Color32::RED),
                    );
                    ui.add_space(8.0);
                    if let Some(ref log) = self.vpx_error_log {
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                ui.monospace(log);
                            });
                    }
                    ui.add_space(8.0);
                    if ui.button(t!("launcher_close").to_string()).clicked() {
                        close = true;
                    }
                });
            if close {
                self.vpx_error_log = None;
            }
        }

        // Search filter
        ui.horizontal(|ui| {
            ui.label(t!("launcher_search").to_string());
            if ui.text_edit_singleline(&mut self.table_filter).changed() {
                self.table_filter_lower = self.table_filter.to_lowercase();
            }
            ui.label(t!("launcher_table_count", count = self.tables.len()).to_string());
        });
        ui.add_space(8.0);

        if self.tables.is_empty() {
            ui.label(t!("launcher_no_tables").to_string());
            return;
        }

        // Table grid with backglass images
        let filter = &self.table_filter_lower;
        let mut launch_idx: Option<usize> = None;
        let card_width = 400.0;
        let card_height = 520.0;
        let img_height = 400.0;
        let card_spacing = 8.0;
        let available_width = ui.available_width();
        let cols = ((available_width / (card_width + card_spacing)) as usize).max(1);
        self.launcher_cols = cols;
        let row_height = card_height + card_spacing;

        let mut scroll_area = egui::ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);

        // Auto-scroll to selected table when navigating with joystick
        if self.scroll_to_selected {
            self.scroll_to_selected = false;
            // Compute the row of the selected table and scroll to it
            let selected_row = self.selected_table / cols;
            let target_y = selected_row as f32 * row_height;
            scroll_area = scroll_area.vertical_scroll_offset(target_y);
        }

        scroll_area.show(ui, |ui| {
            let filtered: Vec<usize> = (0..self.tables.len())
                .filter(|&i| {
                    filter.is_empty()
                        || self.tables[i].name.to_lowercase().contains(filter.as_str())
                })
                .collect();

            for row_start in (0..filtered.len()).step_by(cols) {
                // Center the row
                let row_count = (filtered.len() - row_start).min(cols);
                let row_width = row_count as f32 * (card_width + 8.0) - 8.0;
                let left_pad = ((available_width - row_width) / 2.0).max(0.0);

                ui.horizontal(|ui| {
                    ui.add_space(left_pad);
                    for col in 0..cols {
                        let fi = row_start + col;
                        if fi >= filtered.len() {
                            break;
                        }
                        let idx = filtered[fi];
                        let table = &self.tables[idx];

                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(card_width, card_height),
                            egui::Sense::click(),
                        );

                        if response.hovered() {
                            self.selected_table = idx;
                        }
                        if response.clicked() {
                            launch_idx = Some(idx);
                        }

                        let painter = ui.painter_at(rect);
                        let is_selected = idx == self.selected_table;

                        // Card background
                        let bg_color = if is_selected {
                            egui::Color32::from_rgb(60, 60, 90)
                        } else if response.hovered() {
                            egui::Color32::from_rgb(50, 50, 65)
                        } else {
                            egui::Color32::from_rgb(35, 35, 45)
                        };
                        painter.rect_filled(rect, 6.0, bg_color);

                        // Selection border (inside to avoid clipping by painter_at)
                        if is_selected {
                            painter.rect_stroke(
                                rect,
                                6.0,
                                egui::Stroke::new(4.0, egui::Color32::from_rgb(255, 200, 0)),
                                egui::StrokeKind::Inside,
                            );
                        }

                        // Backglass image (centered in image area)
                        let img_area = egui::Rect::from_min_size(
                            rect.min + egui::vec2(4.0, 4.0),
                            egui::vec2(card_width - 8.0, img_height - 8.0),
                        );
                        if table.bg_path.is_some() {
                            let uri = format!("bytes://bg/{idx}");
                            let img = egui::Image::new(uri)
                                .shrink_to_fit()
                                .corner_radius(egui::CornerRadius::same(4));
                            img.paint_at(ui, img_area);
                        } else {
                            // Placeholder
                            painter.rect_filled(img_area, 4.0, egui::Color32::from_rgb(25, 25, 30));
                            painter.text(
                                img_area.center(),
                                egui::Align2::CENTER_CENTER,
                                "Pas de backglass",
                                egui::FontId::proportional(18.0),
                                egui::Color32::GRAY,
                            );
                        }

                        // Table name (centered, bigger, bold)
                        let text_center = egui::pos2(
                            rect.center().x,
                            rect.min.y + img_height + (card_height - img_height) / 2.0,
                        );
                        painter.text(
                            text_center,
                            egui::Align2::CENTER_CENTER,
                            &table.name,
                            egui::FontId::new(24.0, egui::FontFamily::Proportional),
                            if is_selected {
                                egui::Color32::from_rgb(255, 200, 0)
                            } else {
                                egui::Color32::WHITE
                            },
                        );

                        // B2S badge
                        if !table.has_directb2s {
                            let badge_pos =
                                egui::pos2(rect.max.x - 12.0, rect.min.y + img_height + 6.0);
                            painter.text(
                                badge_pos,
                                egui::Align2::RIGHT_TOP,
                                "No B2S",
                                egui::FontId::proportional(14.0),
                                egui::Color32::from_rgb(255, 100, 100),
                            );
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });

        if let Some(idx) = launch_idx {
            self.selected_table = idx;
            let path = self.tables[idx].path.clone();
            self.launch_table(&path);
        }

        // 3+ screens: cover Playfield with grey metal background + VPX logo
        // (with 2 screens, Playfield hosts the table selector — no cover needed)
        let has_pf_display = self
            .displays
            .iter()
            .any(|d| d.role == DisplayRole::Playfield);
        if has_pf_display && has_dmd && !self.vpx_hide_covers {
            let pf_display = self
                .displays
                .iter()
                .find(|d| d.role == DisplayRole::Playfield);
            let (pf_x, pf_y, pf_w, pf_h) = pf_display
                .map(|d| (d.x as f32, d.y as f32, d.width as f32, d.height as f32))
                .unwrap_or((0.0, 0.0, 1920.0, 1080.0));

            let pf_viewport_id = egui::ViewportId::from_hash_of(PF_VIEWPORT);
            ui.ctx().show_viewport_deferred(
                pf_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("PinReady — Playfield")
                    .with_position(egui::pos2(pf_x, pf_y))
                    .with_inner_size(egui::vec2(pf_w, pf_h))
                    .with_decorations(false)
                    .with_override_redirect(true),
                move |ctx, _class| {
                    // Force position every frame — WM may ignore initial with_position
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        pf_x, pf_y,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(pf_w, pf_h)));
                    egui_extras::install_image_loaders(ctx);
                    ctx.include_bytes("bytes://vpx_logo", VPX_LOGO);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(80, 80, 85)))
                        .show(ctx, |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.add(
                                    egui::Image::new("bytes://vpx_logo")
                                        .max_size(egui::vec2(512.0, 512.0))
                                        .rotate(270.0_f32.to_radians(), egui::vec2(0.5, 0.5))
                                        .tint(egui::Color32::from_rgba_premultiplied(
                                            180, 180, 190, 200,
                                        )),
                                );
                            });
                        });
                },
            );
        }

        // If multi-screen, show backglass on BG display via secondary viewport
        let has_bg_display = self
            .displays
            .iter()
            .any(|d| d.role == DisplayRole::Backglass);
        if has_bg_display && !self.tables.is_empty() && !self.vpx_hide_covers {
            let selected = self.selected_table.min(self.tables.len() - 1);
            let table_name = self.tables[selected].name.clone();
            let bg_bytes = self.tables[selected].bg_bytes.clone();
            let bg_display = self
                .displays
                .iter()
                .find(|d| d.role == DisplayRole::Backglass);
            let (bg_x, bg_y, bg_w, bg_h) = bg_display
                .map(|d| (d.x as f32, d.y as f32, d.width as f32, d.height as f32))
                .unwrap_or((0.0, 0.0, 1280.0, 1024.0));

            let bg_viewport_id = egui::ViewportId::from_hash_of(BG_VIEWPORT);
            ui.ctx().request_repaint_of(bg_viewport_id);
            ui.ctx().show_viewport_deferred(
                bg_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("PinReady — Backglass")
                    .with_position(egui::pos2(bg_x, bg_y))
                    .with_inner_size(egui::vec2(bg_w, bg_h))
                    .with_decorations(false)
                    .with_override_redirect(true),
                move |ctx, _class| {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        bg_x, bg_y,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(bg_w, bg_h)));
                    egui_extras::install_image_loaders(ctx);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                        .show(ctx, |ui| {
                            if let Some(ref bytes) = bg_bytes {
                                let uri = format!("bytes://viewport_bg/{selected}");
                                ctx.include_bytes(uri.clone(), bytes.clone());
                                ui.centered_and_justified(|ui| {
                                    ui.add(egui::Image::new(uri).shrink_to_fit());
                                });
                            } else {
                                ui.centered_and_justified(|ui| {
                                    ui.colored_label(
                                        egui::Color32::WHITE,
                                        egui::RichText::new(&table_name).size(32.0),
                                    );
                                });
                            }
                        });
                },
            );
        }

        // 3+ screens: cover Topper with grey metal background + VPX logo
        let has_topper_display = self.displays.iter().any(|d| d.role == DisplayRole::Topper);
        if has_topper_display && has_dmd && !self.vpx_hide_covers {
            let topper_display = self.displays.iter().find(|d| d.role == DisplayRole::Topper);
            let (tp_x, tp_y, tp_w, tp_h) = topper_display
                .map(|d| (d.x as f32, d.y as f32, d.width as f32, d.height as f32))
                .unwrap_or((0.0, 0.0, 1920.0, 1080.0));

            let tp_viewport_id = egui::ViewportId::from_hash_of(TOPPER_VIEWPORT);
            ui.ctx().show_viewport_deferred(
                tp_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("PinReady — Topper")
                    .with_position(egui::pos2(tp_x, tp_y))
                    .with_inner_size(egui::vec2(tp_w, tp_h))
                    .with_decorations(false)
                    .with_override_redirect(true),
                move |ctx, _class| {
                    // Force position every frame — WM may ignore initial with_position
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        tp_x, tp_y,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(tp_w, tp_h)));
                    egui_extras::install_image_loaders(ctx);
                    ctx.include_bytes("bytes://vpx_logo", VPX_LOGO);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(80, 80, 85)))
                        .show(ctx, |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.add(
                                    egui::Image::new("bytes://vpx_logo")
                                        .max_size(egui::vec2(512.0, 512.0))
                                        .tint(egui::Color32::from_rgba_premultiplied(
                                            180, 180, 190, 200,
                                        )),
                                );
                            });
                        });
                },
            );
        }
    }
}
