use super::*;

impl App {
    pub(super) fn render_system_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("system_heading"));
        ui.add_space(4.0);
        ui.label(t!("system_desc"));
        ui.add_space(16.0);

        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.autostart, t!("autostart_label"));
            help_marker(ui, &t!("autostart_hint"));
        });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.desktop_integration,
                t!("desktop_integration_label"),
            );
            help_marker(ui, &t!("desktop_integration_hint"));
        });

        // ---- Self-hosted mirror (VBS catalog + VPin media DB). Empty
        // = direct GitHub fetch (the default). When set, all index URLs
        // and per-asset URLs route through the mirror; the server is
        // responsible for rewriting URLs inside the manifests so they
        // point back at itself (cf. db.rs::mirror_base_url docs).
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t!("system_mirror_label")).strong());
            help_marker(ui, &t!("system_mirror_hint"));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.mirror_base_url)
                    .hint_text("https://pinready.syl21.org")
                    .desired_width(420.0),
            );
            if resp.changed() {
                if let Err(e) = self.db.set_mirror_base_url(&self.mirror_base_url) {
                    log::error!("Failed to persist mirror_base_url: {e}");
                }
            }
            if ui
                .button(t!("system_mirror_clear"))
                .on_hover_text(t!("system_mirror_clear_hint"))
                .clicked()
            {
                self.mirror_base_url.clear();
                let _ = self.db.set_mirror_base_url("");
            }
        });

        // ---- Credits — last wizard page is the natural spot for "thanks
        // to" since it's the screen the user sees just before Finish.
        // Names sorted alphabetically (case-insensitive).
        ui.add_space(24.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new(format!("💖 {}", t!("system_credits_title"))).strong());
        ui.add_space(4.0);
        ui.label(egui::RichText::new(t!("system_credits_intro")).weak());
        ui.add_space(4.0);
        for name in [
            "Caviar4456",
            "Francisdb",
            "Jsm174",
            "Major Frenchy",
            "Somatik",
            "Spielfool",
            "Superhac",
            "Toxie",
            "Vbousquet",
        ] {
            ui.label(format!("• {name}"));
        }
    }
}
