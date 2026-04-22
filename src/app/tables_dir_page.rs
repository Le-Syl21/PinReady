use super::*;

impl App {
    pub(super) fn render_tables_dir_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tables_heading"));
        ui.add_space(4.0);
        ui.label(t!("tables_desc"));
        ui.add_space(8.0);

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
            // Text variation selector (U+FE0E) forces monochrome rendering of the wrench.
            ui.label("\u{1F527}\u{FE0E}");
            ui.label(t!("tables_tips_webp_desc"));
            ui.hyperlink_to(
                t!("tables_tips_info_here"),
                t!("tables_tips_webp_url").to_string(),
            );
        });
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            ui.label(t!("tables_path"));
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

        if !self.tables_dir.is_empty() {
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
                ui.add_space(8.0);
                ui.label(t!("tables_valid", count = count));
            } else {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::RED, t!("tables_invalid"));
            }
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // Rebuild button explanation
        ui.label(egui::RichText::new(t!("tables_rebuild_title")).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(t!("launcher_rebuild"))
                    .strong()
                    .color(egui::Color32::from_rgb(255, 80, 80)),
            );
            ui.label(t!("tables_rebuild_desc"));
        });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // VBS patch auto-apply notice — explains that we silently
        // install sidecar scripts from jsm174/vpx-standalone-scripts at
        // scan time, and that any pre-existing custom sidecar is
        // backed up as .pre_standalone.vbs before the patch lands.
        ui.label(egui::RichText::new(t!("tables_vbs_patch_title")).strong());
        ui.add_space(4.0);
        ui.label(t!("tables_vbs_patch_desc"));
        ui.add_space(4.0);
        ui.colored_label(
            egui::Color32::from_rgb(200, 180, 100),
            t!("tables_vbs_patch_backup"),
        );

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.checkbox(&mut self.autostart, t!("autostart_label"));
        ui.label(egui::RichText::new(t!("autostart_hint")).weak());
    }
}
