use super::*;

/// A small ⓘ info marker placed next to an option's label.
///
/// Hovering shows `help` as a quick tooltip; clicking pins it in a popup that
/// stays open until the user clicks elsewhere (handy on a pincab where holding
/// a hover is awkward). This is the single, consistent help affordance for
/// every wizard option — the text comes from the per-option `*_help` i18n keys.
pub(crate) fn help_marker(ui: &mut egui::Ui, help: &str) {
    let response = ui
        .add(
            egui::Label::new(egui::RichText::new("ⓘ").color(ui.visuals().hyperlink_color))
                .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::Help)
        .on_hover_text(help);

    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_max_width(340.0);
            ui.label(help);
        });
}
