use super::*;

impl App {
    pub(super) fn render_system_page(&mut self, ui: &mut egui::Ui) {
        theme::page_header(ui, &t!("system_heading"), Some(&t!("system_desc")));

        theme::section(ui, &t!("system_section_start"), None, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.autostart, t!("autostart_label"));
                help_marker(ui, &t!("autostart_hint"));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut self.desktop_integration,
                    t!("desktop_integration_label"),
                );
                help_marker(ui, &t!("desktop_integration_hint"));
            });

            // Two conveniences the menu entry doesn't cover. Separate toggles:
            // a cabinet wants the desktop icon, a workstation usually wants
            // neither. Like every box on this page they only record the choice:
            // Finish applies it (`system_apply`), off the UI thread — creating a
            // Windows shortcut from the click handler froze the window.
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.desktop_shortcut, t!("desktop_shortcut_label"));
                help_marker(ui, &t!("desktop_shortcut_hint"));
            });

            // Pinning is shell-specific and we only speak GNOME's dialect, so the
            // option simply isn't there on a session we can't serve.
            if self.dock_pinning_available {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.dock_pinned, t!("dock_pin_label"));
                    help_marker(ui, &t!("dock_pin_hint"));
                });
            }
        });

        // ---- Self-hosted mirror (VBS catalog + VPin media DB). Empty
        // = direct GitHub fetch (the default). When set, all index URLs
        // and per-asset URLs route through the mirror; the server is
        // responsible for rewriting URLs inside the manifests so they
        // point back at itself (cf. db.rs::mirror_base_url docs).
        theme::section(
            ui,
            &t!("system_mirror_label"),
            Some(&t!("system_mirror_intro")),
            |ui| {
                help_marker(ui, &t!("system_mirror_hint"));
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.mirror_base_url)
                            .hint_text("https://pinready.syl21.org")
                            .desired_width(420.0),
                    );
                    if resp.changed()
                        && let Err(e) = self.db.set_mirror_base_url(&self.mirror_base_url)
                    {
                        log::error!("Failed to persist mirror_base_url: {e}");
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
            },
        );

        // ---- Credits — last wizard page is the natural spot for "thanks
        // to" since it's the screen the user sees just before Finish.
        // Names sorted alphabetically (case-insensitive).
        theme::section(
            ui,
            &t!("system_credits_title"),
            Some(&t!("system_credits_intro")),
            |ui| {
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
            },
        );
    }
}
