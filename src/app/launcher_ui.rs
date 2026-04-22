use super::*;

impl App {
    #[allow(deprecated)]
    pub(super) fn render_launcher(&mut self, ui: &mut egui::Ui) {
        // Install image loaders once
        egui_extras::install_image_loaders(ui.ctx());

        self.process_bg_extraction(ui.ctx());
        self.preload_images_once(ui.ctx());
        self.handle_launcher_joystick(ui);
        self.process_vpx_status(ui.ctx());
        self.process_update_check();
        self.process_pinready_update_check(ui.ctx());
        // Only repaint when needed: bg extraction in progress, VPX running, joystick connected, or update in progress
        if self.bg_rx.is_some()
            || self.vpx_running.load(Ordering::Relaxed)
            || self.joystick_rx.is_some()
            || self.update_downloading
            || self.update_check_rx.is_some()
        {
            ui.ctx().request_repaint();
        }

        // Keyboard nav — same actions as joystick for a single source of
        // truth. Arrows = flipper/magna, Enter = launch, Escape = quit.
        // Mouse wheel is NOT mapped to navigation: it falls through to
        // egui's ScrollArea (native smooth scrolling + acceleration tuned
        // below). On pincab, navigation is done via joystick buttons and
        // the search box — the wheel is for viewport scrolling only.
        enum NavInput {
            Key(egui::Key),
        }
        if !self.tables.is_empty() && !self.vpx_running.load(Ordering::Relaxed) {
            let inputs: Vec<NavInput> = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Key {
                            key, pressed: true, ..
                        } => Some(NavInput::Key(*key)),
                        _ => None,
                    })
                    .collect()
            });

            for input in inputs {
                match input {
                    NavInput::Key(egui::Key::ArrowLeft) => {
                        self.apply_nav_action("LeftFlipper");
                    }
                    NavInput::Key(egui::Key::ArrowRight) => {
                        self.apply_nav_action("RightFlipper");
                    }
                    NavInput::Key(egui::Key::ArrowUp) => {
                        self.apply_nav_action("LeftMagna");
                    }
                    NavInput::Key(egui::Key::ArrowDown) => {
                        self.apply_nav_action("RightMagna");
                    }
                    NavInput::Key(egui::Key::Enter) => {
                        let path = self.tables[self.selected_table].path.clone();
                        self.launch_table(&path);
                    }
                    NavInput::Key(egui::Key::Escape) => {
                        self.quit_launcher(ui.ctx());
                    }
                    _ => {}
                }
            }
        }

        // Window placement handled via ViewportBuilder::with_monitor (main PF)
        // and render_cover_viewports (BG/DMD/Topper).

        // Header size: 2% of the window's available height, rounded up to
        // the next integer. Scales with the screen — readable on 4K cabinet
        // PFs, compact on desktop/windowed.
        let h_size = (ui.available_height() * 0.02).ceil();
        // Horizontal row with vertical Align::Center — same height-0
        // allocation as ui.horizontal (sized to content) but with center
        // cross-axis alignment so buttons + smaller search field line up.
        let hrow_w = ui.available_size_before_wrap().x;
        ui.allocate_ui_with_layout(
            egui::vec2(hrow_w, 0.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(format!("PinReady v{}", env!("CARGO_PKG_VERSION")))
                        .size(h_size)
                        .strong(),
                );
                ui.add_space(16.0);
                ui.label(egui::RichText::new("🔍").size(h_size));
                // Search bar font is h_size - 2 so the box sits slightly smaller
                // than the buttons without losing alignment on the row.
                let search_font = (h_size - 2.0).max(1.0);
                // Scoped visuals tweak — thicker + lighter bg_stroke so the
                // search frame is visible on glossy / bright pincab playfields.
                // Default inactive stroke is gray(60) which washes out; we force
                // a brighter light-gray + 3px width on all widget states.
                let search_resp = ui
                    .scope(|ui| {
                        let stroke = egui::Stroke::new(3.0, egui::Color32::from_gray(180));
                        let v = ui.visuals_mut();
                        v.widgets.inactive.bg_stroke = stroke;
                        v.widgets.hovered.bg_stroke = stroke;
                        v.widgets.active.bg_stroke = stroke;
                        v.widgets.noninteractive.bg_stroke = stroke;
                        v.widgets.open.bg_stroke = stroke;
                        ui.add(
                            egui::TextEdit::singleline(&mut self.table_filter)
                                .font(egui::FontId::proportional(search_font))
                                .hint_text(
                                    egui::RichText::new(
                                        t!("launcher_search", count = self.tables.len())
                                            .to_string(),
                                    )
                                    .size(search_font),
                                )
                                .desired_width(h_size * 9.0),
                        )
                    })
                    .inner;
                if search_resp.changed() {
                    self.table_filter_lower = self.table_filter.to_lowercase();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(egui::RichText::new(t!("launcher_quit")).size(h_size))
                        .clicked()
                    {
                        self.quit_launcher(ui.ctx());
                    }
                    if ui
                        .button(egui::RichText::new(t!("launcher_config")).size(h_size))
                        .clicked()
                    {
                        // Signal main.rs to relaunch this eframe session in
                        // Wizard mode with a fresh viewport (windowed on the
                        // primary display), then close this one.
                        crate::app::request_mode_switch(AppMode::Wizard);
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
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
                                .color(egui::Color32::from_rgb(100, 200, 100))
                                .size(h_size),
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

                    // PinReady self-update — button / progress / error
                    if self.pinready_updating {
                        let (current, total) = self.pinready_update_progress;
                        if total > 0 {
                            let pct = (current as f32 / total as f32 * 100.0) as u32;
                            let mb = current / (1024 * 1024);
                            let total_mb = total / (1024 * 1024);
                            ui.add(
                                egui::ProgressBar::new(current as f32 / total as f32).text(t!(
                                    "pinready_update_progress",
                                    mb = mb,
                                    total = total_mb,
                                    pct = pct
                                )),
                            );
                        } else {
                            ui.spinner();
                            ui.label(t!("pinready_update_extracting"));
                        }
                    } else if let Some(release) = self.pinready_latest_release.clone() {
                        let btn = ui.button(
                            egui::RichText::new(t!(
                                "pinready_update_button",
                                tag = release.tag.as_str()
                            ))
                            .color(egui::Color32::from_rgb(100, 180, 220))
                            .size(h_size),
                        );
                        if btn.clicked() {
                            self.start_pinready_download(&release);
                        }
                    }
                    if let Some(ref err) = self.pinready_update_error {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 100, 100),
                            t!("pinready_update_error", msg = err.as_str()),
                        );
                    }

                    // Rescan button with long-press detection and color feedback
                    // Track mouse button globally to avoid losing the press when cursor drifts
                    let primary_down = ui.input(|i| i.pointer.primary_down());
                    let held_secs = self
                        .rescan_press_start
                        .map(|s| s.elapsed().as_secs_f32())
                        .unwrap_or(0.0);
                    let hold_ratio = (held_secs / 3.0).min(1.0);

                    // Button color: flash feedback after action, or hold progress
                    let btn_color = if let Some((flash_time, is_full)) = self.rescan_flash {
                        let age = flash_time.elapsed().as_secs_f32();
                        let alpha = ((1.0 - age / 1.0) * 255.0).clamp(0.0, 255.0) as u8;
                        if alpha > 0 {
                            ui.ctx().request_repaint();
                            if is_full {
                                Some(egui::Color32::from_rgba_unmultiplied(255, 80, 80, alpha))
                            } else {
                                Some(egui::Color32::from_rgba_unmultiplied(80, 200, 80, alpha))
                            }
                        } else {
                            self.rescan_flash = None;
                            None
                        }
                    } else if self.rescan_press_start.is_some() {
                        // Gradient from green (quick rescan) to red (full reset) as hold progresses
                        let r = (80.0 + hold_ratio * 175.0) as u8;
                        let g = (200.0 - hold_ratio * 120.0) as u8;
                        Some(egui::Color32::from_rgb(r, g, 80))
                    } else {
                        None
                    };

                    let label = if self.rescan_press_start.is_some() {
                        let pct = (hold_ratio * 100.0) as u32;
                        t!("launcher_reset_pct", pct = pct).to_string()
                    } else {
                        t!("launcher_rescan").to_string()
                    };

                    let text = if let Some(color) = btn_color {
                        egui::RichText::new(&label).color(color).size(h_size)
                    } else {
                        egui::RichText::new(&label).size(h_size)
                    };
                    let rescan_btn = ui.button(text);

                    if rescan_btn.is_pointer_button_down_on() && self.rescan_press_start.is_none() {
                        // Mouse down on button: start tracking
                        self.rescan_press_start = Some(std::time::Instant::now());
                    }

                    if self.rescan_press_start.is_some() {
                        if primary_down {
                            // Still holding — check if 3s reached
                            if hold_ratio >= 1.0 {
                                log::info!(
                                    "Long press: full backglass regeneration (flush DB cache)"
                                );
                                self.rescan_press_start = None;
                                self.rescan_flash = Some((std::time::Instant::now(), true));
                                // Flush the SQLite `backglass` cache so the
                                // follow-up scan re-extracts every image
                                // from scratch. Legacy `.pinready_bg_v*.png`
                                // files may still live in table folders
                                // from older PinReady versions — we no
                                // longer write nor read them; users can
                                // delete them manually if desired.
                                if let Err(e) = self.db.clear_backglass() {
                                    log::error!("clear_backglass failed: {e}");
                                }
                                self.tables.iter_mut().for_each(|t| t.bg_bytes = None);
                                self.images_preloaded = false;
                                self.scan_tables();
                            } else {
                                ui.ctx().request_repaint();
                            }
                        } else {
                            // Released before 3s: incremental rescan
                            self.rescan_press_start = None;
                            self.rescan_flash = Some((std::time::Instant::now(), false));
                            self.scan_tables();
                        }
                    }
                });
            },
        );
        ui.add_space(8.0);

        // VPX loading overlay — show spinner/progress but don't return, viewports need to render below
        let vpx_loading =
            self.vpx_running.load(Ordering::Relaxed) && !self.vpx_loading_msg.is_empty();
        if vpx_loading {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                if let Some(pct) = self.vpx_loading_pct {
                    ui.add(
                        egui::ProgressBar::new(pct)
                            .text(&self.vpx_loading_msg)
                            .desired_width(400.0),
                    );
                } else {
                    ui.spinner();
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(&self.vpx_loading_msg)
                            .size(18.0)
                            .strong(),
                    );
                }
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

        // Extra keyboard navigation for long lists.
        // Home/End jump to first/last table. PageUp/PageDown jump by one
        // viewport worth of rows, keeping alignment consistent with joystick nav.
        if !self.vpx_running.load(Ordering::Relaxed) {
            let home = ui.input(|i| i.key_pressed(egui::Key::Home));
            let end = ui.input(|i| i.key_pressed(egui::Key::End));
            let page_up = ui.input(|i| i.key_pressed(egui::Key::PageUp));
            let page_down = ui.input(|i| i.key_pressed(egui::Key::PageDown));

            if home {
                self.selected_table = 0;
                self.scroll_to_selected = true;
            }
            if end {
                self.selected_table = self.tables.len().saturating_sub(1);
                self.scroll_to_selected = true;
            }

            if page_up || page_down {
                let visible_rows = (ui.available_height() / row_height).floor().max(1.0) as usize;
                let page_size = visible_rows.saturating_mul(cols).max(1);
                if page_up {
                    self.selected_table = self.selected_table.saturating_sub(page_size);
                }
                if page_down {
                    self.selected_table = self
                        .selected_table
                        .saturating_add(page_size)
                        .min(self.tables.len().saturating_sub(1));
                }
                self.scroll_to_selected = true;
            }
        }

        // Boost line-based mouse wheel input so stronger wheel flicks scroll farther.
        // Keep trackpad behavior untouched (trackpads usually report point deltas).
        let line_wheel_strength: f32 = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta,
                        ..
                    } => Some(delta.y.abs()),
                    _ => None,
                })
                .sum()
        });
        let wheel_boost = (1.0 + line_wheel_strength * 1.25).clamp(1.0, 8.0);

        let mut scroll_area = egui::ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .wheel_scroll_multiplier(egui::vec2(1.0, wheel_boost));

        // Auto-scroll to selected table when navigating with joystick.
        // Keep the selected row centered in the viewport; clamp at start/end
        // so we don't scroll past the content. Skip the scroll reset when the
        // computed target_y hasn't changed (horizontal flipper inside a row,
        // or row change that clamps to the same max_top on the last rows) —
        // repeatedly calling vertical_scroll_offset with the same value
        // causes visible reflow.
        if self.scroll_to_selected {
            self.scroll_to_selected = false;
            let selected_row = self.selected_table / cols;
            let total_rows = self.tables.len().div_ceil(cols);
            let visible_rows = (ui.available_height() / row_height).floor() as usize;
            let half = visible_rows / 2;
            let top_row = selected_row
                .saturating_sub(half)
                .min(total_rows.saturating_sub(visible_rows));
            let target_y = top_row as f32 * row_height;
            if self.last_scroll_target != Some(target_y) {
                self.last_scroll_target = Some(target_y);
                scroll_area = scroll_area.vertical_scroll_offset(target_y);
            }
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

                        // Only let hover drive selection when the cursor is actively
                        // moving. If the user presses a flipper/magna (no mouse motion),
                        // joystick/keyboard navigation wins and the stale hover doesn't
                        // snap selected_table back under the cursor.
                        let mouse_moved_recently = ui
                            .ctx()
                            .input(|i| i.pointer.time_since_last_movement() < 0.3);
                        if response.hovered() && mouse_moved_recently {
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
                        if table.bg_bytes.is_some() {
                            let uri = format!("bytes://bg/{idx}");
                            let img = egui::Image::new(uri)
                                .shrink_to_fit()
                                .corner_radius(egui::CornerRadius::same(4));
                            img.paint_at(ui, img_area);
                        } else {
                            // Localized "missing image" placeholder. Tells the user
                            // exactly which two filenames they can drop in to fix
                            // it. Rendered live rather than pre-generated so the
                            // hint re-localizes when the user switches language.
                            painter.rect_filled(img_area, 4.0, egui::Color32::from_rgb(25, 25, 30));
                            let cx = img_area.center().x;
                            let h = img_area.height();
                            painter.text(
                                egui::pos2(cx, img_area.min.y + h * 0.30),
                                egui::Align2::CENTER_CENTER,
                                t!("launcher_missing_title", table = table.name.clone()),
                                egui::FontId::proportional(18.0),
                                egui::Color32::LIGHT_GRAY,
                            );
                            painter.text(
                                egui::pos2(cx, img_area.min.y + h * 0.52),
                                egui::Align2::CENTER_CENTER,
                                t!("launcher_missing_hint_launcher"),
                                egui::FontId::proportional(14.0),
                                egui::Color32::GRAY,
                            );
                            painter.text(
                                egui::pos2(cx, img_area.min.y + h * 0.68),
                                egui::Align2::CENTER_CENTER,
                                t!("launcher_missing_hint_b2s"),
                                egui::FontId::proportional(14.0),
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

        if !self.vpx_hide_covers {
            self.render_cover_viewports(ui);
        }
    }

    /// Backglass image on BG, VPX logo cover on DMD and Topper.
    /// Uses `with_monitor(idx)` to place each viewport — same mechanism as the
    /// main PF viewport. Monitor index = position in `self.displays`.
    fn render_cover_viewports(&self, ui: &mut egui::Ui) {
        // Backglass image
        if let Some(bg_idx) = self
            .displays
            .iter()
            .position(|d| d.role == DisplayRole::Backglass)
        {
            if !self.tables.is_empty() {
                let selected = self.selected_table.min(self.tables.len() - 1);
                let table_name = self.tables[selected].name.clone();
                let bg_bytes = self.tables[selected].bg_bytes.clone();

                let bg_viewport_id = egui::ViewportId::from_hash_of(BG_VIEWPORT);
                ui.ctx().request_repaint_of(bg_viewport_id);
                ui.ctx().show_viewport_deferred(
                    bg_viewport_id,
                    egui::ViewportBuilder::default()
                        .with_title("PinReady — Backglass")
                        .with_decorations(false)
                        .with_monitor(bg_idx)
                        .with_rotation(eframe::emath::ViewportRotation::None)
                        .with_active(false),
                    move |ui, _class| {
                        let ctx = ui.ctx().clone();
                        ctx.set_viewport_rotation(eframe::emath::ViewportRotation::None);
                        egui_extras::install_image_loaders(&ctx);
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                            .show_inside(ui, |ui| {
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
        }

        // DMD cover
        if let Some(dmd_idx) = self
            .displays
            .iter()
            .position(|d| d.role == DisplayRole::Dmd)
        {
            Self::show_logo_viewport(
                ui,
                PF_VIEWPORT,
                "PinReady — DMD",
                dmd_idx,
                eframe::emath::ViewportRotation::None,
            );
        }

        // Topper cover
        if let Some(tp_idx) = self
            .displays
            .iter()
            .position(|d| d.role == DisplayRole::Topper)
        {
            Self::show_logo_viewport(
                ui,
                TOPPER_VIEWPORT,
                "PinReady — Topper",
                tp_idx,
                eframe::emath::ViewportRotation::None,
            );
        }
    }

    /// Show a viewport with the VPX logo on a grey background, placed
    /// borderless fullscreen on the given monitor index.
    fn show_logo_viewport(
        ui: &mut egui::Ui,
        id: &'static str,
        title: &str,
        monitor_idx: usize,
        rotation: eframe::emath::ViewportRotation,
    ) {
        let viewport_id = egui::ViewportId::from_hash_of(id);
        ui.ctx().show_viewport_deferred(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_decorations(false)
                .with_monitor(monitor_idx)
                .with_rotation(rotation)
                .with_active(false),
            move |ui, _class| {
                let ctx = ui.ctx().clone();
                ctx.set_viewport_rotation(rotation);
                egui_extras::install_image_loaders(&ctx);
                ctx.include_bytes("bytes://vpx_logo", VPX_LOGO);
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(80, 80, 85)))
                    .show_inside(ui, |ui| {
                        ui.centered_and_justified(|ui| {
                            let img = egui::Image::new("bytes://vpx_logo")
                                .max_size(egui::vec2(512.0, 512.0))
                                .tint(egui::Color32::from_rgba_premultiplied(180, 180, 190, 200));
                            ui.add(img);
                        });
                    });
            },
        );
    }
}
