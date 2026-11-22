//! The launcher's carousel view: the tables stand on a turntable, the
//! selected one at the front and large, its neighbours turning away on both
//! sides, smaller and darker with depth, each over its reflection. Moving the
//! selection rotates the turntable with a damped glide (the shortest way
//! round, wrapping at the ends of the list).
//!
//! Navigation is the grid's: previous / next table, the magna buttons jump by
//! `CAROUSEL_JUMP`, Start launches. A click on a side table turns it to the
//! front, a click on the front table plays it.

use super::launcher_ui::{STAR_GOLD, paint_rating_star, paint_tile_picture};
use super::*;

/// Tables the magna buttons skip in the carousel.
pub(super) const CAROUSEL_JUMP: usize = 5;
/// Angle between two neighbours on the turntable (radians).
const STEP: f32 = 0.42;
/// Tables drawn on each side of the front one.
const SIDE: i64 = 5;

/// Where a table sits for an offset `off` (in tables) from the front: its
/// horizontal position (-1 left … 1 right of the radius), depth (1 front …
/// 0 side) and scale.
pub(super) fn turntable(off: f32) -> Option<(f32, f32, f32)> {
    let theta = off * STEP;
    if theta.abs() > 1.45 {
        return None;
    }
    let depth = theta.cos();
    let scale = 0.32 + 0.68 * depth.powi(3);
    Some((theta.sin(), depth, scale))
}

/// `from` moved towards `to` around a ring of `n` places, the shortest way.
pub(super) fn ring_step(from: f32, to: f32, n: usize, t: f32) -> f32 {
    let nf = n as f32;
    let d = (to - from + nf / 2.0).rem_euclid(nf) - nf / 2.0;
    (from + d * t).rem_euclid(nf)
}

impl App {
    /// Paint the carousel over the rest of the launcher area. Sets
    /// `launch_idx` when the front table is clicked, and fills `drawn` with
    /// the positions (in `filtered`) of the tables painted.
    pub(super) fn paint_carousel(
        &mut self,
        ui: &mut egui::Ui,
        filtered: &[usize],
        launch_idx: &mut Option<usize>,
        drawn: &mut Vec<usize>,
    ) {
        let n = filtered.len();
        if n == 0 {
            return;
        }
        let p = theme::pal(ui.ctx());
        let target = filtered
            .iter()
            .position(|&i| i == self.selected_table)
            .unwrap_or(0) as f32;

        // Damped glide towards the selection
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let pos = ring_step(self.carousel_pos, target, n, 1.0 - (-dt * 11.0).exp());
        let left = ((target - pos + n as f32 / 2.0).rem_euclid(n as f32) - n as f32 / 2.0).abs();
        self.carousel_pos = if left < 0.002 { target } else { pos };
        if left >= 0.002 {
            ui.ctx().request_repaint();
        }

        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let info_h = 90.0;
        let card_h = ((rect.height() - info_h) * 0.66).max(80.0);
        let ratio = self.tile_shape.ratio();
        let card_w = (card_h * ratio).min(rect.width() * 0.46);
        let card_h = card_w / ratio;
        let floor_y = rect.top() + (rect.height() - info_h) * 0.5 + card_h * 0.5;
        let radius = rect.width() * 0.42;

        // Tables around the front one, back to front. With few tables each
        // is drawn once.
        let side = SIDE.min(((n as i64) - 1) / 2);
        let base = self.carousel_pos.round();
        let mut items: Vec<(f32, usize, egui::Rect, f32)> = Vec::new();
        for k in -side..=side {
            let place = base + k as f32;
            let off = place - self.carousel_pos;
            let Some((x, depth, scale)) = turntable(off) else {
                continue;
            };
            let fpos = (place as i64).rem_euclid(n as i64) as usize;
            let size = egui::vec2(card_w * scale, card_h * scale);
            let center = egui::pos2(rect.center().x + x * radius, floor_y - size.y / 2.0);
            items.push((depth, fpos, egui::Rect::from_center_size(center, size), off));
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Clicks go to the front-most table under the pointer
        let mut clicked: Option<(usize, f32)> = None;
        for &(_, fpos, r, off) in items.iter().rev() {
            let resp = ui.interact(
                r,
                ui.scope_id().with(("carousel", fpos)),
                egui::Sense::click(),
            );
            if resp.clicked() {
                clicked = Some((fpos, off));
                break;
            }
        }

        for &(depth, fpos, r, off) in &items {
            let idx = filtered[fpos];
            drawn.push(fpos);
            let front = off.abs() < 0.5;
            let picture = if idx == self.selected_table {
                // the front table plays its clip, like the selected tile
                self.video_tile
                    .as_ref()
                    .and_then(|v| v.texture().cloned())
                    .or_else(|| self.bg_cache.best(idx))
            } else {
                self.bg_cache.thumb(idx).cloned()
            };
            // reflection on the floor, then the card
            if let Some(tex) = &picture {
                let refl =
                    egui::Rect::from_min_size(egui::pos2(r.left(), r.bottom() + 4.0), r.size());
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
                let shade = (40.0 * depth) as u8;
                painter.image(
                    tex.id(),
                    refl,
                    uv,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, shade),
                );
            }
            painter.rect_filled(r.expand(4.0), 8.0, p.surface);
            match &picture {
                Some(tex) => paint_tile_picture(ui, tex, r, self.tile_fill),
                None => {
                    painter.rect_filled(r, 4.0, p.bg);
                    painter.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        &self.tables[idx].name,
                        egui::FontId::proportional((r.height() * 0.08).max(10.0)),
                        p.text_muted,
                    );
                }
            }
            // depth: the side tables sink into the ground colour
            let dim = ((1.0 - depth) * 200.0).clamp(0.0, 200.0) as u8;
            if dim > 0 {
                painter.rect_filled(r.expand(4.0), 8.0, p.bg.gamma_multiply(dim as f32 / 255.0));
            }
            let stroke = if front {
                egui::Stroke::new(4.0, p.accent)
            } else {
                egui::Stroke::new(1.0, p.border)
            };
            painter.rect_stroke(r.expand(4.0), 8.0, stroke, egui::StrokeKind::Outside);
        }

        // The front table's name, rating and place in the list
        if let Some(t) = self.tables.get(self.selected_table) {
            let y = rect.bottom() - info_h + 18.0;
            painter.text(
                egui::pos2(rect.center().x, y),
                egui::Align2::CENTER_CENTER,
                &t.name,
                egui::FontId::proportional(30.0),
                p.text,
            );
            let stars = t.user.rating_tenths as f32 / 10.0;
            for s in 0..5 {
                let c = egui::pos2(rect.center().x + (s as f32 - 2.0) * 28.0, y + 36.0);
                paint_rating_star(
                    &painter,
                    c,
                    11.0,
                    (stars - s as f32).clamp(0.0, 1.0),
                    STAR_GOLD,
                );
            }
            painter.text(
                egui::pos2(rect.right() - 16.0, y + 36.0),
                egui::Align2::RIGHT_CENTER,
                format!("{} / {n}", target as usize + 1),
                egui::FontId::monospace(16.0),
                p.text_faint,
            );
        }

        if let Some((fpos, off)) = clicked {
            if off.abs() < 0.5 {
                *launch_idx = Some(filtered[fpos]);
            } else {
                self.selected_table = filtered[fpos];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_is_largest_and_sides_shrink_symmetrically() {
        let (x0, d0, s0) = turntable(0.0).unwrap();
        assert_eq!((x0, d0, s0), (0.0, 1.0, 1.0));
        let (xl, _, sl) = turntable(-2.0).unwrap();
        let (xr, _, sr) = turntable(2.0).unwrap();
        assert!(xl < 0.0 && xr > 0.0 && (xl + xr).abs() < 1e-6);
        assert!((sl - sr).abs() < 1e-6 && sl < s0);
        assert!(turntable(10.0).is_none());
    }

    #[test]
    fn ring_step_goes_the_short_way_round() {
        // from the last table to the first: one step forward, not n-1 back
        let p = ring_step(9.0, 0.0, 10, 0.5);
        assert!((p - 9.5).abs() < 1e-6);
        let p = ring_step(0.0, 9.0, 10, 1.0);
        assert!((p - 9.0).abs() < 1e-6);
        let p = ring_step(2.0, 5.0, 10, 1.0);
        assert!((p - 5.0).abs() < 1e-6);
    }
}
