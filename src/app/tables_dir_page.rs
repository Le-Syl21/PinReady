use super::*;

impl App {
    pub(super) fn render_tables_dir_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tables_heading"));
        ui.add_space(4.0);
        ui.label(t!("tables_desc"));
        ui.add_space(8.0);

        ui.label(t!("tables_structure"));
        ui.add_space(4.0);
        ui.code(
            "Tables/
  Table_Name/
Table_Name.vpx               <- table (required)
Table_Name.directb2s         <- backglass (same name as .vpx)
Table_Name.ini               <- per-table config (optional)
pinmame/
  roms/rom_name.zip          <- PinMAME ROM
  nvram/rom_name.nv          <- save data
altcolor/rom_name/            <- Serum/VNI colorization
medias/                       <- frontend images/videos",
        );

        ui.add_space(8.0);
        ui.label(t!("tables_modifiable"));
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

        // Rescan explanation
        ui.label(egui::RichText::new(t!("tables_rescan_title")).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(t!("launcher_rescan"))
                    .strong()
                    .color(egui::Color32::from_rgb(80, 200, 80)),
            );
            ui.label(t!("tables_rescan_click"));
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(t!("launcher_reset_pct", pct = 100))
                    .strong()
                    .color(egui::Color32::from_rgb(255, 80, 80)),
            );
            ui.label(t!("tables_rescan_hold"));
        });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.checkbox(&mut self.autostart, t!("autostart_label"));
        ui.label(egui::RichText::new(t!("autostart_hint")).weak());
    }
}
