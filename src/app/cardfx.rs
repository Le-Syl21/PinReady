//! Carousel effects drawn as cards in 3D, each over its reflection on a
//! glossy floor. One function per effect places a card from its offset to
//! the selected table (in tables, fractional while the row glides): where it
//! stands, how it turns, its size and opacity. The same painter draws them
//! all, so an effect is a few lines of geometry.
//!
//! - **Cover flow**: the selected card faces you, the others stand turned on
//!   both sides, packed like records in a crate.
//! - **Stack**: the next tables wait in a pile behind the selected one; the
//!   ones passed fly off to the left.
//! - **Wave**: the row rides a wave, the cards tilting with its slope.
//! - **Cascade**: a staircase going back to the right and down to the left.
//! - **Accordion**: the selected picture opens wide between narrow slices of
//!   its neighbours.
//! - **Zoom**: one large card, its neighbours small and set back.

use super::mesh3d::*;
use super::*;
use egui::{Color32, Pos2, Rect, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardFx {
    CoverFlow,
    Stack,
    Wave,
    Cascade,
    Accordion,
    Zoom,
}

/// Where a card stands. Lengths are in card heights (the card is
/// `ratio` wide), `z` comes towards you, `yaw` turns it around its vertical
/// axis (positive: its left edge comes forward).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub scale: f32,
    pub alpha: f32,
    /// Part of the picture's width shown, centred (accordion slices).
    pub width: f32,
}

impl CardPose {
    fn at(x: f32, y: f32, z: f32) -> Self {
        Self {
            x,
            y,
            z,
            yaw: 0.0,
            scale: 1.0,
            alpha: 1.0,
            width: 1.0,
        }
    }
}

/// Tables drawn on each side of the selected one.
pub(super) fn reach(fx: CardFx) -> i64 {
    match fx {
        CardFx::Accordion => 9,
        CardFx::Stack => 6,
        _ => 6,
    }
}

/// Fade out past `from` tables, gone at `from + 1`.
fn edge_fade(a: f32, from: f32) -> f32 {
    (1.0 - (a - from)).clamp(0.0, 1.0)
}

/// The pose of the card `off` tables from the selected one, `ratio` its
/// width over height. `None` when it is not drawn.
pub(super) fn pose(fx: CardFx, off: f32, ratio: f32) -> Option<CardPose> {
    let a = off.abs();
    let s = off.signum();
    let near = a.min(1.0);
    let p = match fx {
        CardFx::CoverFlow => {
            let turn = 1.05 * near;
            let x = if a < 1.0 {
                off * 0.78 * ratio
            } else {
                s * (0.78 + 0.26 * (a - 1.0)) * ratio
            };
            CardPose {
                yaw: s * turn,
                alpha: edge_fade(a, 5.0),
                ..CardPose::at(x, 0.0, 0.45 * (1.0 - near))
            }
        }
        CardFx::Stack => {
            if off < 0.0 {
                // passed: flies off to the left, turning
                CardPose {
                    yaw: -0.5 * near,
                    alpha: (1.0 + off * 0.9).clamp(0.0, 1.0),
                    ..CardPose::at(off * 1.4 * ratio, -0.1 * near, 0.25 * near)
                }
            } else {
                CardPose {
                    alpha: edge_fade(a, 4.0) * (1.0 - 0.1 * a).max(0.0),
                    ..CardPose::at(0.16 * off * ratio, 0.13 * off, -0.42 * off)
                }
            }
        }
        CardFx::Wave => {
            let ph = off * 1.05;
            CardPose {
                yaw: -0.38 * ph.cos() * s * near.max(0.2),
                scale: (1.0 - 0.1 * a).max(0.5),
                alpha: edge_fade(a, 4.5),
                ..CardPose::at(off * 0.66 * ratio, 0.22 * ph.sin(), -0.2 * a)
            }
        }
        CardFx::Cascade => CardPose {
            alpha: edge_fade(a, 4.5) * (1.0 - 0.08 * a),
            ..CardPose::at(off * 0.45 * ratio, -off * 0.17, -0.6 * a)
        },
        CardFx::Accordion => {
            let slice = 0.17;
            let width = 1.0 - (1.0 - slice) * near;
            let x = if a < 1.0 {
                off * (0.5 + slice / 2.0) * ratio
            } else {
                s * (0.5 + slice / 2.0 + slice * (a - 1.0)) * ratio
            };
            CardPose {
                width,
                alpha: edge_fade(a, 8.0),
                ..CardPose::at(x, 0.0, 0.0)
            }
        }
        CardFx::Zoom => {
            let x = if a < 1.0 {
                off * 0.92 * ratio
            } else {
                s * (0.92 + 0.58 * (a - 1.0)) * ratio
            };
            CardPose {
                scale: 1.0 - 0.35 * near,
                alpha: edge_fade(a, 3.5) * (1.0 - 0.1 * near),
                ..CardPose::at(x, -0.22 * near, -0.5 * near)
            }
        }
    };
    (p.alpha > 0.01).then_some(p)
}

/// The four corners of a card (top-left, top-right, bottom-right,
/// bottom-left), from its pose; `u0..u1` / `v0..v1` select a part of it in
/// card coordinates (0..1), and `mirror` flips it under the floor at `floor`.
fn card_corners(p: &CardPose, ratio: f32, part: Rect, mirror: Option<f32>) -> [V3; 4] {
    let w = ratio * p.scale * p.width;
    let h = p.scale;
    let (c, sn) = (p.yaw.cos(), p.yaw.sin());
    let at = |u: f32, v: f32| {
        let lx = (u - 0.5) * w;
        let ly = (0.5 - v) * h;
        let x = p.x + lx * c;
        let z = p.z - lx * sn;
        let y = p.y + ly;
        match mirror {
            Some(f) => v3(x, 2.0 * f - y, z),
            None => v3(x, y, z),
        }
    };
    match mirror {
        // Under the floor the face starts at the edge that touches it (the
        // card's bottom), so the texture's top row and the fade's opaque
        // end are against the card; with a flipped `uv` this shows the
        // picture as a mirror image.
        Some(_) => [
            at(part.min.x, part.max.y),
            at(part.max.x, part.max.y),
            at(part.max.x, part.min.y),
            at(part.min.x, part.min.y),
        ],
        None => [
            at(part.min.x, part.min.y),
            at(part.max.x, part.min.y),
            at(part.max.x, part.max.y),
            at(part.min.x, part.max.y),
        ],
    }
}

/// Where the picture goes on a card and which part of the texture it shows,
/// for the tile fit setting: bands keep the whole picture inside the card,
/// crop fills it, stretch fills it distorted. `width` narrows the card to its
/// centre (accordion).
fn picture_part(fit: super::tile_fit::TileFit, tex: Vec2, card: Vec2, width: f32) -> (Rect, Rect) {
    use super::tile_fit::{TileFit, fill_uv, fit_rect};
    let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    let shown = Vec2::new(card.x * width, card.y);
    match fit {
        TileFit::Stretch => {
            let u0 = (1.0 - width) / 2.0;
            (
                full,
                Rect::from_min_max(Pos2::new(u0, 0.0), Pos2::new(1.0 - u0, 1.0)),
            )
        }
        TileFit::Crop => (full, fill_uv(tex, shown)),
        TileFit::Bands => {
            // the picture fitted in the full card, then cut to the shown part
            let r = fit_rect(tex, Rect::from_min_size(Pos2::ZERO, card));
            let (x0, x1) = (card.x * (1.0 - width) / 2.0, card.x * (1.0 + width) / 2.0);
            let (l, rr) = (r.left().max(x0), r.right().min(x1));
            if rr <= l {
                return (Rect::NOTHING, full);
            }
            let part = Rect::from_min_max(
                Pos2::new((l - x0) / (x1 - x0), r.top() / card.y),
                Pos2::new((rr - x0) / (x1 - x0), r.bottom() / card.y),
            );
            let uv = Rect::from_min_max(
                Pos2::new((l - r.left()) / r.width(), 0.0),
                Pos2::new((rr - r.left()) / r.width(), 1.0),
            );
            (part, uv)
        }
    }
}

impl App {
    /// A card effect. Same contract as [`App::paint_carousel`].
    pub(super) fn paint_cards(
        &mut self,
        fx: CardFx,
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
            super::carousel::ring_step(self.carousel_pos, target, n, 1.0 - (-dt * 10.0).exp());
        let left = ((target - pos + n as f32 / 2.0).rem_euclid(n as f32) - n as f32 / 2.0).abs();
        self.carousel_pos = if left < 0.002 { target } else { pos };
        if left >= 0.002 {
            ui.ctx().request_repaint();
        }

        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let info_h = 90.0;
        let view = Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.bottom() - info_h));
        let painter = ui.painter_at(view);
        let info_painter = ui.painter_at(rect);
        let ratio = self.tile_shape.ratio();

        // the selected card is about half the view's height, never wider
        // than half its width
        let card_px = (view.height() * 0.5).min(view.width() * 0.5 / ratio);
        let dist = 3.2;
        let cam = Camera::new(
            v3(0.0, 0.25, dist),
            v3(0.0, 0.0, 0.0),
            card_px * dist,
            egui::pos2(view.center().x, view.center().y - view.height() * 0.04),
        );
        let floor = -0.5 - 0.04;

        // the room: a dark ground with a glossy floor fading away, or the
        // selected table's clip
        let behind = self.clip_behind() && self.paint_selected_behind(&painter, view, 110);
        if !behind {
            painter.rect_filled(view, 0.0, p.bg);
        }
        if !behind && let Some((fy, _)) = cam.project(v3(0.0, floor, 0.0)) {
            let fl = Rect::from_min_max(egui::pos2(view.left(), fy.y), view.max);
            let mut m = egui::Mesh::default();
            let top = p.surface.gamma_multiply(0.6);
            let bottom = p.bg;
            m.colored_vertex(fl.left_top(), top);
            m.colored_vertex(fl.right_top(), top);
            m.colored_vertex(fl.right_bottom(), bottom);
            m.colored_vertex(fl.left_bottom(), bottom);
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(m));
        }

        let side = reach(fx).min(((n as i64) - 1) / 2);
        let base = self.carousel_pos.round();
        let mut cards: Vec<(f32, usize, f32, CardPose)> = Vec::new();
        for k in -side..=side {
            let place = base + k as f32;
            let off = place - self.carousel_pos;
            let Some(pose) = pose(fx, off, ratio) else {
                continue;
            };
            let fpos = (place as i64).rem_euclid(n as i64) as usize;
            let centre = v3(pose.x, pose.y, pose.z);
            let depth = centre.sub(cam.eye).dot(cam.fwd);
            cards.push((depth, fpos, off, pose));
        }
        // far first
        cards.sort_by(|a, b| b.0.total_cmp(&a.0));

        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        let mut hit: Vec<(usize, f32, Rect)> = Vec::new();
        for &(_, fpos, off, pose) in &cards {
            let idx = filtered[fpos];
            drawn.push(fpos);
            let front = off.abs() < 0.5;
            let art = self.tile_art(idx);
            let (picture, backdrop, is_wheel) = (art.tex, art.backdrop, art.wheel);
            // a wheel sits whole on a plain card
            let fit = if is_wheel {
                super::tile_fit::TileFit::Bands
            } else {
                self.tile_fit
            };
            let a = (pose.alpha * 255.0) as u8;
            let tint = Color32::from_white_alpha(a);
            let light = 1.0 - 0.3 * off.abs().min(1.0);
            let shaded = shade(Color32::WHITE, light).gamma_multiply(pose.alpha);
            let card = Vec2::new(ratio, 1.0);

            let mut faces: Vec<Face> = Vec::new();
            let mut names: Vec<(Pos2, std::sync::Arc<egui::Galley>, Color32)> = Vec::new();
            for mirror in [Some(floor), None] {
                let fade_to = Color32::TRANSPARENT;
                let refl = mirror.is_some();
                // reflections are faint and fade with the distance to the floor
                let top_tint = if refl {
                    shaded.gamma_multiply(0.28)
                } else {
                    shaded
                };
                // the card body: the band colour behind the picture
                let band = match (self.tile_bands, &backdrop) {
                    _ if is_wheel => p.surface,
                    (super::tile_fit::BandFill::Color, Some(b)) => b.edge,
                    _ => Color32::BLACK,
                };
                let body = card_corners(&pose, ratio, full, mirror);
                let body_fill = match (fit, self.tile_bands, &backdrop) {
                    (super::tile_fit::TileFit::Bands, super::tile_fit::BandFill::Blur, Some(b)) => {
                        let uv = super::tile_fit::fill_uv(
                            b.blur.size_vec2(),
                            card * Vec2::new(pose.width, 1.0),
                        );
                        if refl {
                            Fill::TexFade(b.blur.id(), flip(uv), fade_to)
                        } else {
                            Fill::Tex(b.blur.id(), uv)
                        }
                    }
                    _ => {
                        if refl {
                            Fill::Gradient(fade_to)
                        } else {
                            Fill::Flat
                        }
                    }
                };
                let body_tint = match body_fill {
                    Fill::Flat | Fill::Gradient(_) => shade(band, light)
                        .gamma_multiply(pose.alpha * if refl { 0.28 } else { 1.0 }),
                    _ => top_tint,
                };
                let mut f = quad(body, body_fill, body_tint, &cam);
                f.depth += 0.002; // behind the picture
                faces.push(f);
                if let Some(tex) = &picture {
                    let (part, uv) = picture_part(fit, tex.size_vec2(), card, pose.width);
                    if part.is_positive() {
                        // a mirrored card shows the picture upside down
                        // (`card_corners` already places `part` under the floor)
                        let uv = if refl { flip(uv) } else { uv };
                        let corners = card_corners(&pose, ratio, part, mirror);
                        let fill = if refl {
                            Fill::TexFade(tex.id(), uv, fade_to)
                        } else {
                            Fill::Tex(tex.id(), uv)
                        };
                        faces.push(quad(corners, fill, top_tint, &cam));
                    }
                } else if !refl {
                    // no picture yet: the name on the card
                    let c = card_corners(&pose, ratio, full, None);
                    if let (Some((pa, _)), Some((pb, _))) = (cam.project(c[0]), cam.project(c[2])) {
                        let r = Rect::from_two_pos(pa, pb);
                        let name = self.tables[idx].name.clone();
                        let galley = painter.layout(
                            name,
                            egui::FontId::proportional((r.height() * 0.09).max(10.0)),
                            tint,
                            r.width() * 0.85,
                        );
                        let at = r.center() - galley.size() / 2.0;
                        names.push((at, galley, p.text_muted.gamma_multiply(pose.alpha)));
                    }
                }
            }
            paint_faces(&painter, &cam, &mut faces);
            for (at, galley, color) in names {
                painter.galley(at, galley, color);
            }
            // the outline of the selected card
            let c = card_corners(&pose, ratio, full, None);
            let pts: Vec<Pos2> = c
                .iter()
                .filter_map(|&v| cam.project(v).map(|(p, _)| p))
                .collect();
            if pts.len() == 4 {
                if front {
                    painter.add(egui::Shape::closed_line(
                        pts.clone(),
                        egui::Stroke::new(3.0, p.accent),
                    ));
                }
                let mut r = Rect::NOTHING;
                for q in &pts {
                    r.extend_with(*q);
                }
                hit.push((fpos, off, r.intersect(view)));
            }
        }

        // clicks: the nearest card under the pointer
        let mut clicked: Option<(usize, f32)> = None;
        for &(fpos, off, r) in hit.iter().rev() {
            if !r.is_positive() {
                continue;
            }
            let resp = ui.interact(r, ui.scope_id().with(("cards", fpos)), egui::Sense::click());
            if resp.clicked() {
                clicked = Some((fpos, off));
                break;
            }
        }

        if let Some(t) = self.tables.get(self.selected_table) {
            let y = rect.bottom() - info_h + 22.0;
            info_painter.text(
                egui::pos2(rect.center().x, y),
                egui::Align2::CENTER_CENTER,
                &t.name,
                egui::FontId::proportional(28.0),
                p.text,
            );
            let stars = t.user.rating_tenths as f32 / 10.0;
            for s in 0..5 {
                let c = egui::pos2(rect.center().x + (s as f32 - 2.0) * 26.0, y + 34.0);
                super::launcher_ui::paint_rating_star(
                    &info_painter,
                    c,
                    10.0,
                    (stars - s as f32).clamp(0.0, 1.0),
                    super::launcher_ui::STAR_GOLD,
                );
            }
            info_painter.text(
                egui::pos2(rect.right() - 16.0, y + 34.0),
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

/// The texture part upside down (for reflections).
fn flip(uv: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(uv.min.x, uv.max.y), Pos2::new(uv.max.x, uv.min.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [CardFx; 6] = [
        CardFx::CoverFlow,
        CardFx::Stack,
        CardFx::Wave,
        CardFx::Cascade,
        CardFx::Accordion,
        CardFx::Zoom,
    ];

    #[test]
    fn the_selected_card_is_centred_whole_and_in_front() {
        for fx in ALL {
            let p = pose(fx, 0.0, 1.25).unwrap();
            assert!(p.x.abs() < 1e-6 && p.y.abs() < 1e-6, "{fx:?}");
            assert_eq!((p.scale, p.alpha, p.width), (1.0, 1.0, 1.0), "{fx:?}");
            for k in 1..=3 {
                if let Some(q) = pose(fx, k as f32, 1.25) {
                    assert!(
                        q.z <= p.z + 1e-6,
                        "{fx:?}: card {k} in front of the selected one"
                    );
                }
            }
        }
    }

    #[test]
    fn rows_go_right_for_the_next_tables_and_glide_without_jumps() {
        for fx in ALL {
            let mut last: Option<CardPose> = None;
            let mut t = -3.0f32;
            while t <= 3.0 {
                if let Some(p) = pose(fx, t, 1.25) {
                    if let Some(q) = last {
                        // small steps of the offset move the card a little
                        assert!((p.x - q.x).abs() < 0.2, "{fx:?} jumps at {t}");
                        assert!((p.z - q.z).abs() < 0.2, "{fx:?} jumps at {t}");
                    }
                    last = Some(p);
                } else {
                    last = None;
                }
                t += 0.05;
            }
            if fx != CardFx::Stack {
                assert!(pose(fx, 1.0, 1.25).unwrap().x > 0.0, "{fx:?}");
                assert!(pose(fx, -1.0, 1.25).unwrap().x < 0.0, "{fx:?}");
            }
        }
    }

    #[test]
    fn bands_keep_the_picture_inside_the_card() {
        use super::super::tile_fit::TileFit;
        // a square picture on a 16:9 card: centred, full height, 9/16 wide
        let (part, uv) = picture_part(
            TileFit::Bands,
            Vec2::splat(400.0),
            Vec2::new(16.0 / 9.0, 1.0),
            1.0,
        );
        assert!((part.width() - 9.0 / 16.0).abs() < 1e-4 && (part.height() - 1.0).abs() < 1e-4);
        assert!(
            (part.center().x - 0.5).abs() < 1e-4
                && uv == Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
        );
        // an accordion slice of it keeps its middle
        let (part, uv) = picture_part(
            TileFit::Bands,
            Vec2::splat(400.0),
            Vec2::new(16.0 / 9.0, 1.0),
            0.2,
        );
        assert!((part.width() - 1.0).abs() < 1e-4 && uv.width() < 1.0);
    }
}
