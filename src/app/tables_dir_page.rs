use super::*;

const ASTERISK_RED: egui::Color32 = egui::Color32::from_rgb(255, 80, 80);
const NOTICE_AMBER: egui::Color32 = egui::Color32::from_rgb(255, 200, 80);

impl App {
    pub(super) fn render_tables_dir_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tables_heading"));
        ui.add_space(4.0);
        ui.label(t!("tables_desc"));
        ui.add_space(12.0);

        // ---- The question that decides how many paths we need ----------
        //
        // Asked before anything else: a modern collection needs one path
        // (it is both source and destination), a bulk one needs two.
        ui.label(egui::RichText::new(t!("merge_layout_question")).strong());
        let mut layout_changed = false;
        if ui
            .radio_value(
                &mut self.merge_layout_modern,
                true,
                t!("merge_layout_modern"),
            )
            .on_hover_text(t!("merge_layout_modern_hint"))
            .changed()
        {
            layout_changed = true;
        }
        if ui
            .radio_value(
                &mut self.merge_layout_modern,
                false,
                t!("merge_layout_legacy"),
            )
            .on_hover_text(t!("merge_layout_legacy_hint"))
            .changed()
        {
            layout_changed = true;
        }
        if layout_changed {
            let _ = self.db.set_config(
                "merge_layout_modern",
                if self.merge_layout_modern { "1" } else { "0" },
            );
        }
        ui.add_space(12.0);

        // ---- Paths, in the order the answer implies ---------------------
        if !self.merge_layout_modern {
            let mut source = std::mem::take(&mut self.merge_src_root);
            if pick_dir_row(
                ui,
                "🔎",
                &t!("merge_src_root"),
                &t!("merge_src_root_hint"),
                &t!("merge_src_root_picker"),
                &mut source,
                false,
            ) {
                let _ = self.db.set_merge_source("root", &source);
            }
            self.merge_src_root = source;
            ui.add_space(10.0);
        }

        // The tables directory is the wizard's only mandatory field here
        // (the Next button is gated on it in mod.rs), hence the asterisk.
        let dest_label = if self.merge_layout_modern {
            t!("tables_path")
        } else {
            t!("tables_path_output")
        };
        let mut dest = std::mem::take(&mut self.tables_dir);
        pick_dir_row(
            ui,
            "📂",
            &dest_label,
            &t!("tables_dir_path_hint"),
            &t!("tables_folder_picker"),
            &mut dest,
            true,
        );
        self.tables_dir = dest;

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

        // ---- Import: the action, now that both ends are known ----------
        self.render_merge_section(ui);

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // ---- Maintenance: rescan / patches / catalog. Kept open because
        // these settings are scan-time and users need them visible to make
        // a deliberate choice. Placed before the layout reference, which is
        // documentation rather than something to act on.
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
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut self.jsm174_patching, t!("tables_vbs_patch_toggle"))
                    .changed()
                {
                    if let Err(e) = self.db.set_jsm174_patching_enabled(self.jsm174_patching) {
                        log::error!("Failed to persist jsm174_patching_enabled: {e}");
                    }
                }
                help_marker(ui, &t!("tables_vbs_patch_desc"));
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
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
                help_marker(ui, &t!("tables_catalog_desc"));
            });
        });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // ---- Reference: what the modern layout actually looks like -----
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("❓ {}", t!("tables_help_section"))).strong(),
        )
        .default_open(true)
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
    }

    fn render_merge_section(&mut self, ui: &mut egui::Ui) {
        use crate::merge::{MergeEvent, MergeMode, MergeStrategy};

        // Drain any pending events from a running worker into self.merge_log.
        let mut received_done = false;
        if let Some(rx) = &self.merge_progress_rx {
            while let Ok(ev) = rx.try_recv() {
                match &ev {
                    MergeEvent::ScanProgress { files, dirs } => {
                        self.merge_scan_files = *files;
                        self.merge_scan_dirs = *dirs;
                        continue; // a moving counter, not a log line
                    }
                    MergeEvent::ScanDone {
                        files,
                        dirs,
                        tables,
                    } => {
                        self.merge_scan_files = *files;
                        self.merge_scan_dirs = *dirs;
                        self.merge_table_total = *tables;
                        self.merge_scan_finished = true;
                    }
                    MergeEvent::TableStarted { index, total, .. }
                    | MergeEvent::TableSkipped { index, total, .. } => {
                        // Skipped tables count towards the bar too, or it
                        // stalls short of the end on a collection full of
                        // duplicates.
                        self.merge_table_index = *index;
                        self.merge_table_total = *total;
                    }
                    MergeEvent::Done(report) => {
                        if self.merge_dry_run_report.is_none() {
                            self.merge_dry_run_report = Some(report.clone());
                        }
                        received_done = true;
                    }
                    _ => {}
                }
                self.merge_log.push(ev);
            }
        }
        if received_done {
            self.merge_running = false;
            self.merge_progress_rx = None;
            self.merge_cancel = None;
        }

        // Not a collapsible section: bundling the collection is what this
        // page is for, not an optional extra.
        ui.strong(format!("📥 {}", t!("merge_section_title")));
        ui.add_space(4.0);
        {
            ui.label(t!("merge_section_desc"));
            ui.add_space(8.0);

            // Where the files will land, spelled out — the whole point of
            // the question above is that these two cases differ.
            ui.horizontal_wrapped(|ui| {
                ui.label("📍");
                ui.colored_label(
                    NOTICE_AMBER,
                    match (self.merge_layout_modern, self.tables_dir.trim().is_empty()) {
                        (true, true) => t!("merge_in_place_notice").to_string(),
                        (true, false) => t!(
                            "merge_in_place_notice_path",
                            path = self.tables_dir.as_str()
                        )
                        .to_string(),
                        (false, true) => t!("merge_destination_notice").to_string(),
                        (false, false) => t!(
                            "merge_destination_notice_path",
                            path = self.tables_dir.as_str()
                        )
                        .to_string(),
                    },
                );
            });

            // Copy or move only means something when files come from
            // somewhere else.
            if !self.merge_layout_modern {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t!("merge_strategy_label")).strong());
                    help_marker(ui, &t!("merge_strategy_help"));
                });
                let mut strategy = self.merge_strategy;
                let mut strategy_changed = false;
                ui.horizontal(|ui| {
                    if ui
                        .radio_value(
                            &mut strategy,
                            MergeStrategy::Copy,
                            t!("merge_strategy_copy"),
                        )
                        .on_hover_text(t!("tables_dir_merge_strategy_copy_hint"))
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
                        .on_hover_text(t!("tables_dir_merge_strategy_move_hint"))
                        .changed()
                    {
                        strategy_changed = true;
                    }
                });
                if strategy_changed {
                    self.merge_strategy = strategy;
                    let _ = self.db.set_merge_strategy(strategy.as_db_str());
                }
            }

            ui.add_space(10.0);

            let tables_dir_set =
                !self.tables_dir.is_empty() && std::path::Path::new(&self.tables_dir).is_dir();
            let source_set = self.merge_layout_modern
                || std::path::Path::new(self.merge_src_root.trim()).is_dir();
            let ready = tables_dir_set && source_set;

            ui.horizontal(|ui| {
                let dry_btn = ui
                    .add_enabled(
                        ready && !self.merge_running,
                        egui::Button::new(t!("merge_dry_run")),
                    )
                    .on_hover_text(t!("tables_dir_merge_dry_run_hint"))
                    .on_disabled_hover_text(t!("merge_run_disabled_tooltip"));
                if dry_btn.clicked() {
                    self.start_merge_run(MergeMode::DryRun, ui.ctx());
                }

                let can_commit =
                    ready && !self.merge_running && self.merge_dry_run_report.is_some();
                let commit_btn = ui
                    .add_enabled(can_commit, egui::Button::new(t!("merge_confirm_apply")))
                    .on_hover_text(t!("tables_dir_merge_apply_hint"))
                    .on_disabled_hover_text(t!("merge_apply_disabled_tooltip"));
                if commit_btn.clicked() {
                    self.start_merge_run(MergeMode::Commit, ui.ctx());
                }

                if self.merge_running {
                    let cancel_btn = ui
                        .button(t!("merge_cancel"))
                        .on_hover_text(t!("tables_dir_merge_cancel_hint"));
                    if cancel_btn.clicked() {
                        if let Some(c) = &self.merge_cancel {
                            c.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                }
            });

            // ---- Two-step progress ------------------------------------
            // Step 1 has no known total (that is the point of a recursive
            // scan), so it spins; step 2 fills as tables are bundled.
            if self.merge_running || self.merge_scan_finished {
                ui.add_space(6.0);
                if !self.merge_scan_finished {
                    ui.add(
                        egui::ProgressBar::new(0.0)
                            .animate(self.merge_running)
                            .text(t!(
                                "merge_step_scan",
                                files = self.merge_scan_files,
                                dirs = self.merge_scan_dirs
                            )),
                    );
                } else {
                    let fraction = if self.merge_table_total == 0 {
                        1.0
                    } else {
                        self.merge_table_index as f32 / self.merge_table_total as f32
                    };
                    ui.add(egui::ProgressBar::new(fraction).text(t!(
                        "merge_step_import",
                        current = self.merge_table_index,
                        total = self.merge_table_total
                    )));
                    ui.label(
                        egui::RichText::new(t!(
                            "merge_step_scan_done",
                            files = self.merge_scan_files,
                            dirs = self.merge_scan_dirs
                        ))
                        .weak(),
                    );
                }
            }

            if let Some(report) = &self.merge_dry_run_report {
                ui.add_space(6.0);
                ui.label(t!(
                    "merge_progress",
                    tables = report.tables_processed,
                    found = report.assets_found,
                    applied = report.assets_applied,
                    skipped = report.assets_skipped
                ));
                if report.tables_skipped > 0 {
                    ui.colored_label(
                        NOTICE_AMBER,
                        t!("merge_tables_skipped", count = report.tables_skipped),
                    );
                }
                if report.tables_sample_skipped > 0 {
                    ui.colored_label(
                        NOTICE_AMBER,
                        t!(
                            "merge_samples_skipped",
                            count = report.tables_sample_skipped
                        ),
                    );
                }
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
        }

        if self.merge_running {
            ui.ctx().request_repaint();
        }
    }

    fn start_merge_run(&mut self, mode: crate::merge::MergeMode, _ctx: &egui::Context) {
        if self.tables_dir.is_empty() {
            return;
        }
        self.merge_log.clear();
        self.merge_scan_files = 0;
        self.merge_scan_dirs = 0;
        self.merge_scan_finished = false;
        self.merge_table_index = 0;
        self.merge_table_total = 0;
        if matches!(mode, crate::merge::MergeMode::DryRun) {
            self.merge_dry_run_report = None;
        }
        let output_root = std::path::PathBuf::from(&self.tables_dir);
        let scan_root = if self.merge_layout_modern {
            output_root.clone()
        } else {
            std::path::PathBuf::from(self.merge_src_root.trim())
        };
        let (rx, cancel, _handle) = crate::merge::spawn(crate::merge::MergeConfig {
            scan_root,
            output_root,
            // In place there is nothing to copy or move: tables stay put.
            strategy: if self.merge_layout_modern {
                crate::merge::MergeStrategy::Copy
            } else {
                self.merge_strategy
            },
            mode,
        });
        self.merge_progress_rx = Some(rx);
        self.merge_cancel = Some(cancel);
        self.merge_running = true;
    }
}

/// One labelled directory row: icon, label, optional mandatory marker,
/// help bubble, text field and a browse button. The picker opens where
/// the current value points — left to itself the OS dialog lands in the
/// process's working directory, which is never what the user meant.
fn pick_dir_row(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    hint: &str,
    picker_title: &str,
    value: &mut String,
    mandatory: bool,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(icon);
        ui.label(egui::RichText::new(label).strong());
        if mandatory {
            ui.colored_label(ASTERISK_RED, "*");
        }
        help_marker(ui, hint);
    });
    let mut changed = false;
    ui.horizontal(|ui| {
        if ui.text_edit_singleline(value).changed() {
            changed = true;
        }
        if ui
            .button(t!("tables_browse"))
            .on_hover_text(t!("tables_dir_browse_hint"))
            .clicked()
        {
            let mut dialog = rfd::FileDialog::new().set_title(picker_title);
            let current = std::path::Path::new(value.trim());
            if current.is_dir() {
                dialog = dialog.set_directory(current);
            }
            if let Some(path) = dialog.pick_folder() {
                *value = path.to_string_lossy().into_owned();
                changed = true;
            }
        }
    });
    changed
}

fn render_merge_event(ui: &mut egui::Ui, ev: &crate::merge::MergeEvent) {
    use crate::merge::MergeEvent::*;
    let green = egui::Color32::from_rgb(120, 200, 120);
    let red = egui::Color32::from_rgb(220, 110, 110);
    let yellow = egui::Color32::from_rgb(220, 200, 120);
    let weak = egui::Color32::from_gray(170);
    match ev {
        ScanProgress { .. } => {}
        ScanDone {
            files,
            dirs,
            tables,
        } => {
            ui.colored_label(
                weak,
                t!(
                    "merge_log_indexed",
                    files = files,
                    dirs = dirs,
                    tables = tables
                ),
            );
        }
        TableStarted { name, index, total } => {
            ui.colored_label(weak, format!("▸ [{index}/{total}] {name}"));
        }
        TableSkipped {
            name,
            index,
            total,
            src,
            kept,
        } => {
            ui.colored_label(
                yellow,
                format!(
                    "▸ [{index}/{total}] {name} — {}",
                    t!(
                        "merge_table_duplicate",
                        src = src.display().to_string(),
                        kept = kept.display().to_string()
                    )
                ),
            );
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
                t!(
                    "merge_log_done",
                    tables = report.tables_processed,
                    found = report.assets_found,
                    applied = report.assets_applied
                ),
            );
            if report.tables_skipped > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 80),
                    t!("merge_tables_skipped", count = report.tables_skipped),
                );
            }
        }
    }
}
