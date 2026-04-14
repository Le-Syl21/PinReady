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
            egui::Slider::new(&mut self.tilt.nudge_scale_pct, 0.0..=100.0)
                .custom_formatter(|v, _| format!("{:.0}%", v)),
        );
        ui.add_space(4.0);

        ui.label(t!("tilt_deadzone"));
        ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Slider::new(&mut self.tilt.nudge_deadzone_pct, 0.0..=100.0)
                .custom_formatter(|v, _| format!("{:.0}%", v)),
        );
        ui.add_space(12.0);

        // --- Tilt section ---
        ui.separator();
        ui.strong(t!("tilt_section"));
        ui.add_space(4.0);

        ui.label(t!("tilt_threshold"));
        ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Slider::new(&mut self.tilt.tilt_sensitivity_pct, 0.0..=100.0)
                .custom_formatter(|v, _| format!("{:.0}%", v)),
        );
        ui.add_space(8.0);

        // Warning if deadzone >= tilt threshold
        if self.tilt.nudge_deadzone_pct >= self.tilt.tilt_sensitivity_pct {
            ui.colored_label(
                egui::Color32::from_rgb(255, 180, 50),
                t!("tilt_deadzone_warning"),
            );
        }
        ui.add_space(4.0);

        // Visualization: deadzone (green ring) + tilt (red ring) + live dot
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

        // Deadzone ring (green) — movements inside are ignored
        let deadzone_radius = radius * (self.tilt.nudge_deadzone_pct / 100.0);
        if deadzone_radius > 1.0 {
            painter.circle_filled(
                center,
                deadzone_radius,
                egui::Color32::from_rgba_unmultiplied(80, 200, 80, 30),
            );
            painter.circle_stroke(
                center,
                deadzone_radius,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 200, 80)),
            );
        }

        // TILT threshold ring (red) — beyond this = TILT
        let threshold_radius = radius * (self.tilt.tilt_sensitivity_pct / 100.0);
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

        // Live accelerometer dot
        let scale = (self.tilt.nudge_scale_pct / 100.0) * 8.0;
        let dot_x = center.x + (self.accel_x * scale).clamp(-1.0, 1.0) * radius;
        let dot_y = center.y + (self.accel_y * scale).clamp(-1.0, 1.0) * radius;
        let dot_pos = egui::pos2(dot_x, dot_y);
        let dist = ((dot_x - center.x).powi(2) + (dot_y - center.y).powi(2)).sqrt();
        let dot_color = if dist > threshold_radius {
            egui::Color32::from_rgb(255, 50, 50) // in TILT zone
        } else if dist < deadzone_radius {
            egui::Color32::from_rgb(150, 150, 150) // in deadzone (ignored)
        } else {
            egui::Color32::from_rgb(100, 220, 100) // active zone
        };
        painter.circle_filled(dot_pos, 7.0, dot_color);
    }
}
