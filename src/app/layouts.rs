//! The launcher's layouts, and the wheel bar.
//!
//! - **Grid**: the tiles (`launcher_ui.rs`).
//! - **Carousel**: the turntable (`carousel.rs`).
//! - **Wheels**: the selected table's playfield clip fills the screen (its
//!   backglass while there is no clip), with a row of wheels along the bottom,
//!   the selected one in the middle and larger, the table's name under it.
//!   The row fades out after a few idle seconds and comes back on any input,
//!   so the clip plays alone like on an attract screen.
//! - **Arcade**: a row of cabinets the camera walks along (`arcade.rs`).

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherLayout {
    Grid,
    Carousel,
    Wheels,
    Arcade,
}

impl LauncherLayout {
    pub const ALL: [Self; 4] = [Self::Grid, Self::Carousel, Self::Wheels, Self::Arcade];

    /// The value stored in `launcher_layout`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Carousel => "carousel",
            Self::Wheels => "wheels",
            Self::Arcade => "arcade",
        }
    }

    pub fn from_key(k: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|l| Some(l.key()) == k)
            .unwrap_or(Self::Grid)
    }

    /// The next one, for the header button.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&l| l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> String {
        match self {
            Self::Grid => t!("launcher_view_grid"),
            Self::Carousel => t!("launcher_view_carousel"),
            Self::Wheels => t!("launcher_view_wheels"),
            Self::Arcade => t!("launcher_view_arcade"),
        }
        .to_string()
    }

    /// Whether the selected table's clip should be decoded at screen size
    /// (it fills the screen) rather than tile size.
    pub fn full_screen_clip(self) -> bool {
        matches!(self, Self::Wheels | Self::Arcade)
    }
}

/// Wheels drawn on each side of the selected one.
const WHEEL_SIDE: i64 = 4;
/// Seconds without input before the wheel row fades out.
const WHEEL_IDLE_S: f32 = 6.0;
/// Fade duration (seconds).
const WHEEL_FADE_S: f32 = 0.6;

/// Size (relative to the selected wheel) and horizontal place (in wheel
/// widths from the centre) of the wheel `off` tables away from the selected
/// one.
pub(super) fn wheel_slot(off: f32) -> (f32, f32) {
    let a = off.abs();
    let scale = if a < 1.0 {
        1.0 - 0.3 * a
    } else {
        0.7 - 0.07 * (a - 1.0)
    }
    .max(0.3);
    // the selected wheel needs more room than its neighbours
    let x = off.signum()
        * if a < 1.0 {
            0.88 * a
        } else {
            0.88 + 0.72 * (a - 1.0)
        };
    (scale, x)
}

impl App {
    /// The wheel layout. Same contract as [`App::paint_carousel`].
    pub(super) fn paint_wheel_bar(
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
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let pos =
            super::carousel::ring_step(self.carousel_pos, target, n, 1.0 - (-dt * 12.0).exp());
        let left = ((target - pos + n as f32 / 2.0).rem_euclid(n as f32) - n as f32 / 2.0).abs();
        self.carousel_pos = if left < 0.002 { target } else { pos };
        if left >= 0.002 {
            ui.ctx().request_repaint();
        }

        // idle fade: any input or a new selection brings the row back
        let now = std::time::Instant::now();
        let input = ui.input(|i| {
            i.pointer.delta() != egui::Vec2::ZERO
                || i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::PointerButton { pressed: true, .. }
                    )
                })
        });
        if input || self.wheel_last_sel != Some(self.selected_table) {
            self.wheel_last_sel = Some(self.selected_table);
            self.wheel_active_at = now;
        }
        let idle = now.duration_since(self.wheel_active_at).as_secs_f32();
        let alpha = (1.0 - (idle - WHEEL_IDLE_S) / WHEEL_FADE_S).clamp(0.0, 1.0);
        if alpha > 0.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(if idle > WHEEL_IDLE_S {
                    16
                } else {
                    250
                }));
        }

        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::BLACK);

        // the selected table's clip, else its backglass, filling the screen
        let sel = self.selected_table;
        let clip = self.video_tile.as_ref().and_then(|v| v.texture().cloned());
        let (bg, dim) = match clip {
            Some(t) => (Some(t), 0u8),
            None => (self.bg_cache.best(sel), 90u8),
        };
        if let Some(tex) = bg {
            let size = tex.size_vec2();
            if size.x > 0.0 && size.y > 0.0 {
                painter.image(
                    tex.id(),
                    rect,
                    super::tile_fit::fill_uv(size, rect.size()),
                    egui::Color32::WHITE,
                );
                if dim > 0 {
                    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(dim));
                }
            }
        }

        // the wheel row
        let big = (rect.height() * 0.17).min(rect.width() * 0.26).max(48.0);
        let row_y = rect.bottom() - big * 0.5 - 70.0;
        let band = egui::Rect::from_min_max(
            egui::pos2(rect.left(), row_y - big * 0.62),
            egui::pos2(rect.right(), rect.bottom()),
        );
        let fade = |c: egui::Color32| c.gamma_multiply(alpha);
        if alpha > 0.0 {
            painter.rect_filled(band, 0.0, fade(egui::Color32::from_black_alpha(150)));
        }
        let side = WHEEL_SIDE.min(((n as i64) - 1) / 2);
        let base = self.carousel_pos.round();
        let mut items: Vec<(f32, usize, egui::Rect)> = Vec::new();
        for k in -side..=side {
            let place = base + k as f32;
            let off = place - self.carousel_pos;
            let (scale, x) = wheel_slot(off);
            let fpos = (place as i64).rem_euclid(n as i64) as usize;
            let size = big * scale;
            let center = egui::pos2(rect.center().x + x * big, row_y + (big - size) * 0.3);
            let r = egui::Rect::from_center_size(center, egui::Vec2::splat(size));
            if r.right() < rect.left() || r.left() > rect.right() {
                continue;
            }
            items.push((off, fpos, r));
        }
        // far ones first, the selected one on top
        items.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));

        let mut clicked: Option<(usize, f32)> = None;
        for &(off, fpos, r) in items.iter().rev() {
            if alpha <= 0.0 {
                break;
            }
            let resp = ui.interact(r, ui.scope_id().with(("wheel", fpos)), egui::Sense::click());
            if resp.clicked() {
                clicked = Some((fpos, off));
                break;
            }
        }

        let allow_download = self.db.get_config("media_videos").as_deref() != Some("0");
        for &(off, fpos, r) in &items {
            let idx = filtered[fpos];
            drawn.push(fpos);
            if alpha <= 0.0 {
                continue;
            }
            let table = &self.tables[idx];
            let dir = table.path.parent().map(|d| d.to_path_buf());
            let download = if allow_download {
                table.vps_id.clone()
            } else {
                None
            };
            let shade = (255.0 * (1.0 - 0.35 * off.abs().min(1.5))) as u8;
            let tint = fade(egui::Color32::from_gray(shade));
            let wheel = dir
                .as_deref()
                .and_then(|d| self.wheels.get(idx, d, download.as_deref()).cloned());
            match wheel {
                Some(tex) => {
                    let fit = super::tile_fit::fit_rect(tex.size_vec2(), r);
                    painter.image(
                        tex.id(),
                        fit,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        tint,
                    );
                }
                None => {
                    // no wheel: a disc with the name
                    painter.circle_filled(r.center(), r.width() * 0.46, fade(p.surface));
                    painter.circle_stroke(
                        r.center(),
                        r.width() * 0.46,
                        egui::Stroke::new(2.0, fade(p.border)),
                    );
                    let galley = painter.layout(
                        self.tables[idx].name.clone(),
                        egui::FontId::proportional((r.width() * 0.11).max(9.0)),
                        tint,
                        r.width() * 0.8,
                    );
                    let at = r.center() - galley.size() / 2.0;
                    painter.galley(at, galley, tint);
                }
            }
        }

        // name and rating of the selected table
        if alpha > 0.0
            && let Some(t) = self.tables.get(sel)
        {
            let y = rect.bottom() - 46.0;
            painter.text(
                egui::pos2(rect.center().x, y),
                egui::Align2::CENTER_CENTER,
                &t.name,
                egui::FontId::proportional(26.0),
                fade(egui::Color32::WHITE),
            );
            let stars = t.user.rating_tenths as f32 / 10.0;
            for s in 0..5 {
                let c = egui::pos2(rect.center().x + (s as f32 - 2.0) * 24.0, y + 28.0);
                super::launcher_ui::paint_rating_star(
                    &painter,
                    c,
                    9.0,
                    (stars - s as f32).clamp(0.0, 1.0),
                    fade(super::launcher_ui::STAR_GOLD),
                );
            }
            painter.text(
                egui::pos2(rect.right() - 16.0, y + 28.0),
                egui::Align2::RIGHT_CENTER,
                format!("{} / {n}", target as usize + 1),
                egui::FontId::monospace(14.0),
                fade(egui::Color32::from_gray(170)),
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
    fn layout_keys_round_trip_and_cycle() {
        for l in LauncherLayout::ALL {
            assert_eq!(LauncherLayout::from_key(Some(l.key())), l);
        }
        assert_eq!(LauncherLayout::from_key(None), LauncherLayout::Grid);
        assert_eq!(
            LauncherLayout::from_key(Some("carousel")),
            LauncherLayout::Carousel
        );
        let mut l = LauncherLayout::Grid;
        for _ in 0..LauncherLayout::ALL.len() {
            l = l.next();
        }
        assert_eq!(l, LauncherLayout::Grid);
    }

    #[test]
    fn the_selected_wheel_is_largest_and_neighbours_do_not_overlap() {
        let (s0, x0) = wheel_slot(0.0);
        assert_eq!((s0, x0), (1.0, 0.0));
        let mut prev_right = 0.5; // half the selected wheel
        for k in 1..=4 {
            let (s, x) = wheel_slot(k as f32);
            let (sl, xl) = wheel_slot(-(k as f32));
            assert!(s < s0 && (s - sl).abs() < 1e-6 && (x + xl).abs() < 1e-6);
            assert!(x - s / 2.0 >= prev_right - 0.02, "wheel {k} overlaps");
            prev_right = x + s / 2.0;
        }
    }
}
