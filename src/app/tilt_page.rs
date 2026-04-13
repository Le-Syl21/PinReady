use super::*;

impl App {
    pub(super) fn render_tilt_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("tilt_heading"));
        ui.add_space(4.0);
        ui.label(t!("tilt_desc"));
        ui.add_space(12.0);

        // Request repaint for live accelerometer data
        ui.ctx().request_repaint();

        // --- Nudge section ---
        ui.separator();
        ui.strong(t!("tilt_nudge"));
        ui.add_space(4.0);

        ui.checkbox(&mut self.tilt.nudge_filter, t!("tilt_noise_filter"));
        ui.add_space(4.0);

        ui.label(t!("tilt_sensitivity"));
        ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Slider::new(&mut self.tilt.nudge_scale, 0.1..=2.0)
                .custom_formatter(|v, _| format!("{:.1}x", v)),
        );
        ui.add_space(12.0);

        // --- Tilt section ---
        ui.separator();
        ui.strong(t!("tilt_section"));
        ui.add_space(4.0);

        ui.label(t!("tilt_threshold"));
        ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Slider::new(&mut self.tilt.plumb_threshold_angle, 5.0..=60.0)
                .suffix("°")
                .custom_formatter(|v, _| format!("{:.0}°", v)),
        );
        ui.add_space(8.0);

        // Single visualization circle: live accel dot + tilt threshold ring
        ui.label(t!("tilt_visualization"));
        ui.add_space(4.0);
        let viz_size = egui::vec2(240.0, 240.0);
        let (rect, _response) = ui.allocate_exact_size(viz_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let center = rect.center();
        let radius = 110.0;

        // Outer circle (max range)
        painter.circle_stroke(center, radius, egui::Stroke::new(2.0, egui::Color32::GRAY));
        // Cross hairs
        painter.line_segment(
            [
                center - egui::vec2(radius, 0.0),
                center + egui::vec2(radius, 0.0),
            ],
            egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
        );
        painter.line_segment(
            [
                center - egui::vec2(0.0, radius),
                center + egui::vec2(0.0, radius),
            ],
            egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
        );

        // TILT threshold ring (red)
        let threshold_radius = radius * (self.tilt.plumb_threshold_angle / 60.0);
        painter.circle_stroke(
            center,
            threshold_radius,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 80, 80)),
        );
        painter.text(
            center + egui::vec2(threshold_radius + 4.0, -10.0),
            egui::Align2::LEFT_CENTER,
            "TILT",
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(255, 80, 80),
        );

        // Live accelerometer dot — apply nudge_scale so slider changes are visible live
        let scale = self.tilt.nudge_scale * 8.0; // 8x base amplification for visibility
        let dot_x = center.x + (self.accel_x * scale).clamp(-1.0, 1.0) * radius;
        let dot_y = center.y + (self.accel_y * scale).clamp(-1.0, 1.0) * radius;
        let dot_pos = egui::pos2(dot_x, dot_y);
        let dist = ((dot_x - center.x).powi(2) + (dot_y - center.y).powi(2)).sqrt();
        let dot_color = if dist > threshold_radius {
            egui::Color32::from_rgb(255, 50, 50) // in TILT zone
        } else {
            egui::Color32::from_rgb(100, 220, 100) // safe
        };
        painter.circle_filled(dot_pos, 7.0, dot_color);
    }
}
