use super::*;

impl App {
    pub(super) fn render_system_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("system_heading"));
        ui.add_space(4.0);
        ui.label(t!("system_desc"));
        ui.add_space(16.0);

        ui.separator();
        ui.add_space(8.0);
        ui.checkbox(&mut self.autostart, t!("autostart_label"));
        ui.label(egui::RichText::new(t!("autostart_hint")).weak());

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.checkbox(
            &mut self.desktop_integration,
            t!("desktop_integration_label"),
        );
        ui.label(egui::RichText::new(t!("desktop_integration_hint")).weak());
    }
}
