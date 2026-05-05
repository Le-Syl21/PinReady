use super::*;

const ASTERISK_RED: egui::Color32 = egui::Color32::from_rgb(255, 80, 80);
const NOTICE_AMBER: egui::Color32 = egui::Color32::from_rgb(255, 200, 80);

impl App {
    pub(super) fn render_tables_dir_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tables_heading"));
        ui.add_space(4.0);
        ui.label(t!("tables_desc"));
        ui.add_space(12.0);

        // ---- Mandatory tables_dir picker ----
        // Marked with a red asterisk so users see at a glance that this is
        // the only required field on the page; the wizard's Next button is
        // also gated on this in mod.rs.
        ui.horizontal(|ui| {
            ui.label("📂");
            ui.label(egui::RichText::new(t!("tables_path")).strong());
            ui.colored_label(ASTERISK_RED, "*");
        });
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.tables_dir);
            if ui.button(t!("tables_browse")).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(t!("tables_folder_picker"))
                    .pick_folder()
                {
                    self.tables_dir = path.to_string_lossy().into_owned();
                }
            }
        });

        ui.add_space(4.0);
        if self.tables_dir.is_empty() {
            ui.colored_label(ASTERISK_RED, format!("⚠ {}", t!("tables_path_required")));
        } else {
            let path = std::path::Path::new(&self.tables_dir);
            if path.is_dir() {
                let count = std::fs::read_dir(path)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_dir())
                            .count()
                    })
                    .unwrap_or(0);
                ui.colored_label(
                    egui::Color32::from_rgb(120, 200, 120),
                    format!("✓ {}", t!("tables_valid", count = count)),
                );
            } else {
                ui.colored_label(ASTERISK_RED, format!("⚠ {}", t!("tables_invalid")));
            }
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // ---- Import (merge) — placed right after the picker because it
        // uses tables_dir as its destination. The notice makes the dependency
        // explicit so the user can't be confused about where files land.
        self.render_merge_section(ui);

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // ---- Help section (folded by default) ----
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("❓ {}", t!("tables_help_section"))).strong(),
        )
        .default_open(false)
        .show(ui, |ui| {
            ui.label(t!("tables_structure"));
            ui.add_space(4.0);
            ui.code(t!("tables_structure_tree").to_string());

            ui.add_space(6.0);
            ui.colored_label(
                egui::Color32::from_rgb(200, 180, 100),
                t!("tables_formats_supported"),
            );
            ui.colored_label(
                egui::Color32::from_rgb(255, 80, 80),
                t!("tables_formats_unsupported"),
            );

            ui.add_space(8.0);
            ui.label(t!("tables_modifiable"));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("📖");
                ui.label(t!("tables_tips_patch_desc"));
                ui.hyperlink_to(
                    t!("tables_tips_info_here"),
                    t!("tables_tips_patch_url").to_string(),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("\u{1F527}\u{FE0E}");
                ui.label(t!("tables_tips_webp_desc"));
                ui.hyperlink_to(
                    t!("tables_tips_info_here"),
                    t!("tables_tips_webp_url").to_string(),
                );
            });
        });

        ui.add_space(8.0);

        // ---- Maintenance: Rebuild + VBS toggle + Catalog toggle, kept open
        // because these settings are scan-time and users actually need them
        // visible to make a deliberate choice.
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("🛠 {}", t!("tables_maintenance_section"))).strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(t!("launcher_rebuild"))
                        .strong()
                        .color(egui::Color32::from_rgb(255, 80, 80)),
                );
                ui.label(t!("tables_rebuild_desc"));
            });

            ui.add_space(10.0);
            if ui
                .checkbox(&mut self.jsm174_patching, t!("tables_vbs_patch_toggle"))
                .changed()
            {
                if let Err(e) = self.db.set_jsm174_patching_enabled(self.jsm174_patching) {
                    log::error!("Failed to persist jsm174_patching_enabled: {e}");
                }
            }
            ui.label(egui::RichText::new(t!("tables_vbs_patch_desc")).weak());

            ui.add_space(10.0);
            if ui
                .checkbox(&mut self.catalog_enrichment, t!("tables_catalog_toggle"))
                .changed()
            {
                if let Err(e) = self
                    .db
                    .set_catalog_enrichment_enabled(self.catalog_enrichment)
                {
                    log::error!("Failed to persist catalog_enrichment_enabled: {e}");
                }
            }
            ui.label(egui::RichText::new(t!("tables_catalog_desc")).weak());
        });
    }

    fn render_merge_section(&mut self, ui: &mut egui::Ui) {
        use crate::merge::{MergeEvent, MergeMode, MergeSources, MergeStrategy};

        // Drain any pending events from a running worker into self.merge_log.
        let mut received_done = false;
        if let Some(rx) = &self.merge_progress_rx {
            while let Ok(ev) = rx.try_recv() {
                if let MergeEvent::Done(report) = &ev {
                    if self.merge_dry_run_report.is_none() {
                        self.merge_dry_run_report = Some(report.clone());
                    }
                    received_done = true;
                }
                self.merge_log.push(ev);
            }
        }
        if received_done {
            self.merge_running = false;
            self.merge_progress_rx = None;
            self.merge_cancel = None;
        }

        // Header carries a small badge with how many sources are configured
        // — lets the user see at a glance whether the section is in use
        // without expanding it.
        let configured_sources = [
            &self.merge_src_vpinmame,
            &self.merge_src_pupvideos,
            &self.merge_src_music,
        ]
        .iter()
        .filter(|s| !s.trim().is_empty())
        .count();
        let header_text = if configured_sources == 0 {
            format!(
                "📥 {}  —  {}",
                t!("merge_section_title"),
                t!("merge_sources_none")
            )
        } else {
            format!(
                "📥 {}  —  {}",
                t!("merge_section_title"),
                t!("merge_sources_configured", count = configured_sources)
            )
        };

        let header = egui::CollapsingHeader::new(egui::RichText::new(header_text).strong())
            .default_open(self.merge_section_open);

        header.show(ui, |ui| {
            // Destination notice — the import always writes into tables_dir,
            // so make that explicit. Show the resolved path when available
            // to make the effect concrete.
            let notice = if self.tables_dir.trim().is_empty() {
                t!("merge_destination_notice").to_string()
            } else {
                t!(
                    "merge_destination_notice_path",
                    path = self.tables_dir.as_str()
                )
                .to_string()
            };
            ui.horizontal(|ui| {
                ui.label("📍");
                ui.colored_label(NOTICE_AMBER, notice);
            });
            ui.add_space(6.0);

            ui.label(t!("merge_section_desc"));
            ui.label(
                egui::RichText::new(t!("merge_section_optional"))
                    .weak()
                    .italics(),
            );
            ui.add_space(8.0);

            let mut pick = |label: &str, value: &mut String, browse_label: &str| -> bool {
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label(label);
                    if ui.text_edit_singleline(value).changed() {
                        changed = true;
                    }
                    if ui.button(browse_label).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            *value = p.to_string_lossy().into_owned();
                            changed = true;
                        }
                    }
                });
                changed
            };

            if pick(
                &t!("merge_src_vpinmame"),
                &mut self.merge_src_vpinmame,
                &t!("tables_browse"),
            ) {
                let _ = self
                    .db
                    .set_merge_source("vpinmame", &self.merge_src_vpinmame);
            }
            if pick(
                &t!("merge_src_pupvideos"),
                &mut self.merge_src_pupvideos,
                &t!("tables_browse"),
            ) {
                let _ = self
                    .db
                    .set_merge_source("pupvideos", &self.merge_src_pupvideos);
            }
            if pick(
                &t!("merge_src_music"),
                &mut self.merge_src_music,
                &t!("tables_browse"),
            ) {
                let _ = self.db.set_merge_source("music", &self.merge_src_music);
            }

            ui.add_space(8.0);
            ui.label(egui::RichText::new(t!("merge_strategy_label")).strong());
            let mut strategy = self.merge_strategy;
            let mut strategy_changed = false;
            ui.horizontal(|ui| {
                if ui
                    .radio_value(
                        &mut strategy,
                        MergeStrategy::Copy,
                        t!("merge_strategy_copy"),
                    )
                    .changed()
                {
                    strategy_changed = true;
                }
                if ui
                    .radio_value(
                        &mut strategy,
                        MergeStrategy::Move,
                        t!("merge_strategy_move"),
                    )
                    .changed()
                {
                    strategy_changed = true;
                }
                if ui
                    .radio_value(
                        &mut strategy,
                        MergeStrategy::Symlink,
                        t!("merge_strategy_symlink"),
                    )
                    .changed()
                {
                    strategy_changed = true;
                }
            });
            if strategy_changed {
                self.merge_strategy = strategy;
                let _ = self.db.set_merge_strategy(strategy.as_db_str());
            }
            if matches!(self.merge_strategy, MergeStrategy::Symlink) && cfg!(target_os = "windows")
            {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 180, 80),
                    t!("merge_symlink_windows_warning"),
                );
            }

            ui.add_space(8.0);

            let tables_dir_set =
                !self.tables_dir.is_empty() && std::path::Path::new(&self.tables_dir).is_dir();

            ui.horizontal(|ui| {
                let dry_btn = ui
                    .add_enabled(
                        tables_dir_set && !self.merge_running,
                        egui::Button::new(t!("merge_dry_run")),
                    )
                    .on_disabled_hover_text(t!("merge_run_disabled_tooltip"));
                if dry_btn.clicked() {
                    self.start_merge_run(MergeMode::DryRun, ui.ctx());
                }

                let can_commit =
                    tables_dir_set && !self.merge_running && self.merge_dry_run_report.is_some();
                let commit_btn = ui
                    .add_enabled(can_commit, egui::Button::new(t!("merge_confirm_apply")))
                    .on_disabled_hover_text(t!("merge_apply_disabled_tooltip"));
                if commit_btn.clicked() {
                    self.start_merge_run(MergeMode::Commit, ui.ctx());
                }

                if self.merge_running && ui.button(t!("merge_cancel")).clicked() {
                    if let Some(c) = &self.merge_cancel {
                        c.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });

            if let Some(report) = &self.merge_dry_run_report {
                ui.add_space(6.0);
                ui.label(t!(
                    "merge_progress",
                    tables = report.tables_processed,
                    found = report.assets_found,
                    applied = report.assets_applied,
                    skipped = report.assets_skipped
                ));
            }

            if !self.merge_log.is_empty() {
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let start = self.merge_log.len().saturating_sub(400);
                        for ev in &self.merge_log[start..] {
                            render_merge_event(ui, ev);
                        }
                    });
            }
        });

        if self.merge_running {
            ui.ctx().request_repaint();
        }

        let _ = MergeSources {
            vpinmame: None,
            pupvideos: None,
            music: None,
        };
    }

    fn start_merge_run(&mut self, mode: crate::merge::MergeMode, _ctx: &egui::Context) {
        if self.tables_dir.is_empty() {
            return;
        }
        self.merge_log.clear();
        if matches!(mode, crate::merge::MergeMode::DryRun) {
            self.merge_dry_run_report = None;
        }
        let sources = crate::merge::MergeSources {
            vpinmame: opt_path(&self.merge_src_vpinmame),
            pupvideos: opt_path(&self.merge_src_pupvideos),
            music: opt_path(&self.merge_src_music),
        };
        let (rx, cancel, _handle) = crate::merge::spawn(
            std::path::PathBuf::from(&self.tables_dir),
            sources,
            self.merge_strategy,
            mode,
        );
        self.merge_progress_rx = Some(rx);
        self.merge_cancel = Some(cancel);
        self.merge_running = true;
    }
}

fn opt_path(s: &str) -> Option<std::path::PathBuf> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(trimmed))
    }
}

fn render_merge_event(ui: &mut egui::Ui, ev: &crate::merge::MergeEvent) {
    use crate::merge::MergeEvent::*;
    let green = egui::Color32::from_rgb(120, 200, 120);
    let red = egui::Color32::from_rgb(220, 110, 110);
    let yellow = egui::Color32::from_rgb(220, 200, 120);
    let weak = egui::Color32::from_gray(170);
    match ev {
        TableStarted { name } => {
            ui.colored_label(weak, format!("▸ {name}"));
        }
        AssetFound { kind, src, .. } => {
            ui.colored_label(green, format!("  + {} : {}", kind.label(), src.display()));
        }
        AssetApplied { kind, dst } => {
            ui.colored_label(green, format!("  ✓ {} → {}", kind.label(), dst.display()));
        }
        AssetSkipped { kind, reason } => {
            ui.colored_label(yellow, format!("  · {} ({})", kind.label(), reason.label()));
        }
        AssetError { kind, msg } => {
            ui.colored_label(red, format!("  ! {} : {msg}", kind.label()));
        }
        TableDone { .. } => {}
        Done(report) => {
            ui.add_space(4.0);
            ui.colored_label(
                green,
                format!(
                    "{} {} / {} / {}",
                    t!("merge_log_done"),
                    report.tables_processed,
                    report.assets_found,
                    report.assets_applied
                ),
            );
        }
    }
}
