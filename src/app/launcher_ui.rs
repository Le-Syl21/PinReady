use super::*;

impl App {
    pub(super) fn render_launcher(&mut self, ui: &mut egui::Ui) {
        // Install image loaders once
        egui_extras::install_image_loaders(ui.ctx());

        // About window: painted on top of everything when open.
        self.render_about_window(ui.ctx());

        self.process_bg_extraction(ui.ctx());
        self.process_vbs_extraction();
        self.process_preview_audio(ui.ctx());
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

        // Keyboard + (cabinet-only) mouse wheel nav. The collection
        // and string-vs-enum mapping live in `launcher_input` so this
        // call site stays a one-liner. Joystick events go through the
        // same `apply_launcher_action` dispatch from
        // `handle_launcher_joystick`.
        if !self.tables.is_empty() && !self.vpx_running.load(Ordering::Relaxed) {
            for action in launcher_input::collect_actions(ui, self.rotation) {
                self.apply_launcher_action(action, ui.ctx());
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
                // Toolbar toggles (theme + rotation + info) at the very
                // start of the launcher topbar. The version string used to
                // sit here but ate too much width on rotated viewports; it
                // now lives inside the About window opened by the ℹ icon.
                let ctx = ui.ctx().clone();
                self.toolbar_toggles(&ctx, ui, h_size);
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
                                .desired_width(h_size * 7.0),
                        )
                    })
                    .inner;
                let mut filter_changed = false;
                if search_resp.changed() {
                    self.table_filter_lower = self.table_filter.to_lowercase();
                    filter_changed = true;
                }

                // Type-anywhere-to-search: when no text field has focus
                // and a printable character is typed, append it to the
                // filter and grab focus on the search bar so the next
                // keystrokes go directly into the field. Skipped while
                // VPX is running (game owns the input) and while a text
                // edit already has focus (avoid double-insertion: the
                // TextEdit will consume the same Event::Text natively).
                if !ui.ctx().text_edit_focused() && !self.vpx_running.load(Ordering::Relaxed) {
                    let typed: String = ui.input(|i| {
                        i.events
                            .iter()
                            .filter_map(|e| match e {
                                egui::Event::Text(s) => Some(s.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("")
                    });
                    let appended: String = typed.chars().filter(|c| !c.is_control()).collect();
                    if !appended.is_empty() {
                        self.table_filter.push_str(&appended);
                        self.table_filter_lower = self.table_filter.to_lowercase();
                        filter_changed = true;
                        // Place the caret at the end of the buffer so
                        // subsequent typing appends instead of inserting
                        // before the freshly-typed characters.
                        let id = search_resp.id;
                        let mut state =
                            egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                        let end = self.table_filter.chars().count();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(end),
                            )));
                        state.store(ui.ctx(), id);
                        search_resp.request_focus();
                    }
                }

                // When the filter changes, snap the selection to the
                // first match so Enter launches a table that's actually
                // visible in the filtered grid. Skip if the current
                // selection still satisfies the new filter (lets the
                // user refine without losing their place).
                if filter_changed && !self.table_filter_lower.is_empty() {
                    let f = self.table_filter_lower.as_str();
                    let current_matches = self
                        .tables
                        .get(self.selected_table)
                        .map(|t| t.name.to_lowercase().contains(f))
                        .unwrap_or(false);
                    if !current_matches {
                        if let Some(idx) = self
                            .tables
                            .iter()
                            .position(|t| t.name.to_lowercase().contains(f))
                        {
                            self.selected_table = idx;
                            self.scroll_to_selected = true;
                        }
                    }
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
                        // primary display), then close this one. Cover
                        // viewports go first to keep the Mutter destruction
                        // order deterministic — same reason as in
                        // `quit_launcher`.
                        // Stop any currently-playing table preview audio
                        // before the mode switch so the user doesn't hear
                        // it linger into the wizard. The audio thread
                        // itself stays alive — the wizard needs it for
                        // its test sequence.
                        if let Some(tx) = &self.audio_cmd_tx {
                            let _ = tx.send(AudioCommand::PreviewStop);
                            let _ = tx.send(AudioCommand::StopAll);
                        }
                        self.preview_playing = false;
                        self.preview_due_at = None;
                        crate::app::request_mode_switch(AppMode::Wizard);
                        Self::close_cover_viewports(ui.ctx());
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    // Update buttons/progress used to live here inline, but any
                    // transition (available → downloading → gone) resized the
                    // row and shifted the other buttons around. They now render
                    // as full-width banners just below the header — see
                    // `render_update_banners` after this row closes.

                    // Rebuild button: single click, flushes backglass +
                    // vbs_patches caches and re-scans from scratch. The
                    // mtime-invalidation inside `scan_tables` already
                    // handles "new tables" and "source file changed"
                    // cases transparently on every scan.
                    let label = t!("launcher_rebuild").to_string();
                    if ui
                        .button(egui::RichText::new(&label).size(h_size))
                        .clicked()
                    {
                        log::info!(
                            "Rebuild: flushing backglass + vbs_patches caches, full re-scan"
                        );
                        if let Err(e) = self.db.clear_backglass() {
                            log::error!("clear_backglass failed: {e}");
                        }
                        if let Err(e) = self.db.clear_vbs_patches() {
                            log::error!("clear_vbs_patches failed: {e}");
                        }
                        self.tables.iter_mut().for_each(|t| t.bg_bytes = None);
                        self.images_preloaded = false;
                        self.scan_tables();
                    }

                    // Update counter — surfaces tables flagged
                    // `update_available` by the catalog worker. Sits to
                    // the left of Rebuild (added after it in this
                    // right-to-left layout). Click is currently a no-op;
                    // the per-card badge is the primary affordance.
                    let updates = self.tables.iter().filter(|t| t.update_available).count();
                    if updates > 0 {
                        ui.label(
                            egui::RichText::new(format!("↑ {updates}"))
                                .size(h_size)
                                .color(egui::Color32::from_rgb(80, 170, 240)),
                        );
                    }
                });
            },
        );
        ui.add_space(8.0);

        // Full-width update banners (one per pending update). Rendering
        // below the header keeps the button row stable — the header never
        // shifts when an update appears or completes.
        self.render_update_banners(ui);

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
                        // Force the vertical scrollbar to stay visible so the
                        // user sees there's more content below — egui's
                        // default `AlwaysHidden` only shows it on hover, easy
                        // to miss on a kiosk screen.
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .show(ui, |ui| {
                                let job = highlight_error_keywords(log, ui.style());
                                ui.label(job);
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

        let filter = &self.table_filter_lower;

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

        // Compute the filtered grid once — both the scroll-target
        // calculation and the rendering closure need it. Without this,
        // `selected_row = self.selected_table / cols` would index into
        // the unfiltered grid and scroll to the wrong y-offset when a
        // search is active.
        let filtered: Vec<usize> = (0..self.tables.len())
            .filter(|&i| {
                filter.is_empty() || self.tables[i].name.to_lowercase().contains(filter.as_str())
            })
            .collect();

        // Auto-scroll to selected table when navigating with joystick.
        // Keep the selected row centered in the viewport; clamp at start/end
        // so we don't scroll past the content. Skip the scroll reset when the
        // computed target_y hasn't changed (horizontal flipper inside a row,
        // or row change that clamps to the same max_top on the last rows) —
        // repeatedly calling vertical_scroll_offset with the same value
        // causes visible reflow.
        if self.scroll_to_selected {
            self.scroll_to_selected = false;
            let selected_pos = filtered
                .iter()
                .position(|&i| i == self.selected_table)
                .unwrap_or(0);
            let selected_row = selected_pos / cols;
            let total_rows = filtered.len().div_ceil(cols);
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
                            // Generation-tagged URI matches what
                            // `process_bg_extraction` and the preload
                            // path register, so post-rescan the cell
                            // pulls the fresh JPEG instead of whatever
                            // was at this index in the previous scan.
                            let uri = format!("bytes://bg/{}/{idx}", self.scan_generation);
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

                        // Update-available badge: small "↑" disk in the
                        // top-right corner of the image area. Set by the
                        // catalog worker when the live VPSDB Game.updated_at
                        // has moved past the value recorded at link time.
                        // Click → open the VPS catalog page for this game
                        // in the user's browser; allocated AFTER the card
                        // so egui's hit-testing routes the click here
                        // instead of triggering the card's launch handler.
                        if table.update_available {
                            let badge_radius = 14.0;
                            let badge_center = egui::pos2(
                                img_area.max.x - badge_radius - 4.0,
                                img_area.min.y + badge_radius + 4.0,
                            );
                            let badge_rect = egui::Rect::from_center_size(
                                badge_center,
                                egui::vec2(badge_radius * 2.0, badge_radius * 2.0),
                            );
                            let badge_resp = ui.interact(
                                badge_rect,
                                ui.id().with(("update_badge", idx)),
                                egui::Sense::click(),
                            );
                            let bg_color = if badge_resp.hovered() {
                                egui::Color32::from_rgb(70, 170, 240)
                            } else {
                                egui::Color32::from_rgb(40, 140, 220)
                            };
                            painter.circle_filled(badge_center, badge_radius, bg_color);
                            painter.circle_stroke(
                                badge_center,
                                badge_radius,
                                egui::Stroke::new(1.5, egui::Color32::from_rgb(20, 80, 140)),
                            );
                            painter.text(
                                badge_center,
                                egui::Align2::CENTER_CENTER,
                                "↑",
                                egui::FontId::new(20.0, egui::FontFamily::Proportional),
                                egui::Color32::WHITE,
                            );
                            if badge_resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            // Tooltip on hover — replaces the previous
                            // bare circle with a friendly hint that
                            // explains what clicking does. The egui
                            // tooltip uses the response's hover state
                            // and renders a small floating panel near
                            // the cursor.
                            let badge_resp = badge_resp
                                .on_hover_text(t!("launcher_update_badge_tooltip"));
                            if badge_resp.clicked() {
                                if let Some(ref vid) = table.vps_id {
                                    let url = format!(
                                        "https://virtualpinballspreadsheet.github.io/games?game={vid}"
                                    );
                                    log::info!("Opening VPS page: {url}");
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                                }
                            }
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
                        .with_active(false),
                    move |ui, _class| {
                        let ctx = ui.ctx().clone();
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
            Self::show_logo_viewport(ui, PF_VIEWPORT, "PinReady — DMD", dmd_idx);
        }

        // Topper cover
        if let Some(tp_idx) = self
            .displays
            .iter()
            .position(|d| d.role == DisplayRole::Topper)
        {
            Self::show_logo_viewport(ui, TOPPER_VIEWPORT, "PinReady — Topper", tp_idx);
        }
    }

    /// Show a viewport with the VPX logo on a grey background, placed
    /// borderless fullscreen on the given monitor index.
    fn show_logo_viewport(ui: &mut egui::Ui, id: &'static str, title: &str, monitor_idx: usize) {
        let viewport_id = egui::ViewportId::from_hash_of(id);
        ui.ctx().show_viewport_deferred(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_decorations(false)
                .with_monitor(monitor_idx)
                .with_active(false),
            move |ui, _class| {
                let ctx = ui.ctx().clone();
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

    /// Render one full-width banner per pending update (VPX + PinReady).
    /// Each banner shows either a clickable "available" button, a live
    /// progress bar during download, or an error label. Text is centered
    /// (ProgressBar centers by default; the button spans full width via
    /// `add_sized`). Rendering here — below the header, not inside it —
    /// keeps the button row height stable across state transitions.
    fn render_update_banners(&mut self, ui: &mut egui::Ui) {
        let has_vpx = self.update_downloading
            || self.vpx_latest_release.is_some()
            || self.update_error.is_some();
        let has_pinready = self.pinready_updating
            || self.pinready_latest_release.is_some()
            || self.pinready_update_error.is_some();
        if !has_vpx && !has_pinready {
            return;
        }

        // Same 2%-of-height sizing rule as the header — banners keep
        // parity with the buttons above them across desktop/4K PF.
        let text_size = (ui.available_height() * 0.02).ceil().max(12.0);
        let bar_h = (text_size * 1.8).ceil();
        let full_w = ui.available_width();

        let render_error = |ui: &mut egui::Ui, msg: String| {
            ui.add_sized(
                [full_w, bar_h],
                egui::Label::new(
                    egui::RichText::new(msg)
                        .size(text_size)
                        .color(egui::Color32::from_rgb(255, 100, 100)),
                ),
            );
        };

        if has_vpx {
            if self.update_downloading {
                let (current, total) = self.update_progress;
                let (progress, txt) = if total > 0 {
                    let pct = (current as f32 / total as f32 * 100.0) as u32;
                    let mb = current / (1024 * 1024);
                    let total_mb = total / (1024 * 1024);
                    (
                        current as f32 / total as f32,
                        t!("update_progress", mb = mb, total = total_mb, pct = pct).to_string(),
                    )
                } else {
                    (0.0, t!("update_extracting").to_string())
                };
                ui.add(
                    egui::ProgressBar::new(progress)
                        .desired_width(full_w)
                        .desired_height(bar_h)
                        .text(egui::RichText::new(txt).size(text_size).strong()),
                );
            } else if let Some(release) = self.vpx_latest_release.clone() {
                let label = t!("update_button", tag = release.tag.as_str()).to_string();
                let btn = ui.add_sized(
                    [full_w, bar_h],
                    egui::Button::new(
                        egui::RichText::new(label)
                            .size(text_size)
                            .strong()
                            .color(egui::Color32::from_rgb(100, 200, 100)),
                    ),
                );
                if btn.clicked() {
                    self.start_vpx_download(&release);
                }
            }
            if let Some(err) = self.update_error.clone() {
                render_error(ui, t!("update_error", msg = err.as_str()).to_string());
            }
        }

        if has_vpx && has_pinready {
            ui.add_space(4.0);
        }

        if has_pinready {
            if self.pinready_updating {
                let (current, total) = self.pinready_update_progress;
                let (progress, txt) = if total > 0 {
                    let pct = (current as f32 / total as f32 * 100.0) as u32;
                    let mb = current / (1024 * 1024);
                    let total_mb = total / (1024 * 1024);
                    (
                        current as f32 / total as f32,
                        t!(
                            "pinready_update_progress",
                            mb = mb,
                            total = total_mb,
                            pct = pct
                        )
                        .to_string(),
                    )
                } else {
                    (0.0, t!("pinready_update_extracting").to_string())
                };
                ui.add(
                    egui::ProgressBar::new(progress)
                        .desired_width(full_w)
                        .desired_height(bar_h)
                        .text(egui::RichText::new(txt).size(text_size).strong()),
                );
            } else if let Some(release) = self.pinready_latest_release.clone() {
                let label = t!("pinready_update_button", tag = release.tag.as_str()).to_string();
                let btn = ui.add_sized(
                    [full_w, bar_h],
                    egui::Button::new(
                        egui::RichText::new(label)
                            .size(text_size)
                            .strong()
                            .color(egui::Color32::from_rgb(100, 180, 220)),
                    ),
                );
                if btn.clicked() {
                    self.start_pinready_download(&release);
                }
            }
            if let Some(err) = self.pinready_update_error.clone() {
                render_error(
                    ui,
                    t!("pinready_update_error", msg = err.as_str()).to_string(),
                );
            }
        }

        ui.add_space(4.0);
    }
}

/// Build a `LayoutJob` from a multi-line log, painting common
/// crash/error vocabulary in red so the user can spot the meaningful
/// lines at a glance. Case-insensitive matching, longest-keyword-wins
/// to avoid double-coloring "segmentation" then "fault" separately.
fn highlight_error_keywords(text: &str, style: &egui::Style) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    // Sorted longest-first so e.g. "segmentation fault" wins over "fault".
    const KEYWORDS: &[&str] = &[
        "segmentation fault",
        "stack overflow",
        "buffer overrun",
        "heap corruption",
        "core dumped",
        "core dump",
        "access violation",
        "killed by",
        "panicked",
        "panic",
        "exception",
        "assertion",
        "assert",
        "segfault",
        "coredump",
        "aborted",
        "abort",
        "crashed",
        "crash",
        "failed",
        "failure",
        "fatal",
        "errors",
        "errored",
        "error",
        "warning",
        "warn",
    ];
    let font = egui::FontId::monospace(style.text_styles[&egui::TextStyle::Monospace].size);
    let normal = TextFormat {
        font_id: font.clone(),
        color: style.visuals.text_color(),
        ..Default::default()
    };
    let highlight = TextFormat {
        font_id: font,
        color: egui::Color32::from_rgb(255, 110, 110),
        ..Default::default()
    };

    let mut job = LayoutJob::default();
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut i = 0;
    // Compare in byte-space: all keywords are pure ASCII so byte-level
    // `eq_ignore_ascii_case` is correct, and unlike `&str[i..j]` slicing
    // it does NOT panic when `j` falls inside a multibyte UTF-8 char
    // (the `·` separators in the system header are 2-byte chars and
    // triggered exactly that panic on non-ASCII content). We only
    // re-enter `&str` when we know both ends are valid char boundaries —
    // in `job.append(&text[start..end], …)`, where `start`/`end` are
    // either 0, `text.len()`, a position we walked to via
    // `chars().next().len_utf8()`, or `i + kw.len()` (kw is ASCII =
    // ends on a char boundary because we scan from a char boundary).
    let lower_bytes = lower.as_bytes();
    while i < text.len() {
        // Try to match a keyword at this byte offset.
        let mut matched: Option<usize> = None;
        for kw in KEYWORDS {
            let kw_bytes = kw.as_bytes();
            let end = i + kw_bytes.len();
            if end <= lower_bytes.len() && lower_bytes[i..end].eq_ignore_ascii_case(kw_bytes) {
                // Word-ish boundary: don't highlight inside another word
                // (so "panicked" doesn't paint on "panickedness", and
                // "warn" stops short of "warning" handling above since
                // we sort longest-first).
                let before_ok = i == 0 || !is_word_char(bytes[i - 1]);
                let after_ok = end == bytes.len() || !is_word_char(bytes[end]);
                if before_ok && after_ok {
                    matched = Some(kw_bytes.len());
                    break;
                }
            }
        }
        if let Some(len) = matched {
            job.append(&text[i..i + len], 0.0, highlight.clone());
            i += len;
        } else {
            // Walk forward one char (UTF-8 safe) into the normal segment,
            // batched until we hit the next keyword start.
            let start = i;
            let next_char_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            i += next_char_len;
            // Greedy fast-path: extend as long as no keyword starts here.
            'extend: while i < text.len() {
                for kw in KEYWORDS {
                    let kw_bytes = kw.as_bytes();
                    let end = i + kw_bytes.len();
                    if end <= lower_bytes.len()
                        && lower_bytes[i..end].eq_ignore_ascii_case(kw_bytes)
                    {
                        let before_ok = !is_word_char(bytes[i - 1]);
                        let after_ok = end == bytes.len() || !is_word_char(bytes[end]);
                        if before_ok && after_ok {
                            break 'extend;
                        }
                    }
                }
                let step = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                i += step;
            }
            job.append(&text[start..i], 0.0, normal.clone());
        }
    }
    job
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the system header contains middle-dot `·` (2 bytes
    /// in UTF-8). The previous implementation sliced `&str` directly
    /// and panicked when a keyword's byte window happened to end inside
    /// a `·`. The byte-space comparison fixes it; this test pins the
    /// behaviour so we don't regress.
    #[test]
    fn highlight_does_not_panic_on_multibyte_separators() {
        let style = egui::Style::default();
        let text = "$ /home/nico/visual_pinball/vpinballx_bgfx -play /home/nico/tables/avatar/avatar.vpx\n  cwd:    /home/nico\n  system: ubuntu 24.04.4 lts · x11 · gnome (mutter)\n  client: pinready v0.12.1\n\nvisual pinball exited with status: signal: 11 (sigsegv) (core dumped)\n";
        let _ = highlight_error_keywords(text, &style);
    }

    #[test]
    fn highlight_finds_keywords_in_ascii_text() {
        let style = egui::Style::default();
        let _ = highlight_error_keywords("error: panicked at foo.rs", &style);
    }

    #[test]
    fn highlight_handles_french_accents_in_path() {
        let style = egui::Style::default();
        let _ = highlight_error_keywords(
            "cwd: /home/sylvain/Téléchargements/Apollo 13/error.log\n",
            &style,
        );
    }
}
