//! The arcade layout: the tables stand in a row of pinball cabinets, the
//! camera walks along the row. Each cabinet shows its backglass in the
//! backbox and its wheel as a topper; the selected one steps forward, lit,
//! with its playfield clip on the playfield, and the others fade into the
//! hall. Moving the selection glides the camera to the next cabinet.
//!
//! Everything is drawn with egui meshes: each face of a cabinet is a quad
//! projected with a pinhole camera, cut into a small grid so the textures
//! keep their perspective, and the faces are painted far to near.

use super::mesh3d::*;
use super::*;
use egui::{Color32, Pos2, Rect, Vec2};

/// Distance between two cabinets (metres, like everything here).
const SPACING: f32 = 1.3;
/// Cabinets drawn on each side of the camera.
const SIDE: i64 = 5;
/// How far the selected cabinet comes forward.
const STEP_OUT: f32 = 0.38;
/// Length of the dive to the playfield when a table is launched (seconds).
const PLUNGE_S: f32 = 1.4;

/// What a cabinet shows.
struct CabinetArt {
    backglass: Option<egui::TextureHandle>,
    playfield: Option<egui::TextureHandle>,
    /// The backglass blurred: what an unselected playfield shows, like a lit
    /// playfield seen through dusty glass.
    glow: Option<egui::TextureHandle>,
    wheel: Option<egui::TextureHandle>,
}

/// The faces of one cabinet whose front centre stands at `x`, `z`.
/// `light` 0..1 (1 = the selected one).
fn cabinet_faces(x: f32, z: f32, light: f32, art: &CabinetArt, cam: &Camera, out: &mut Vec<Face>) {
    let w = 0.36; // half width
    let body = Color32::from_rgb(28, 26, 40);
    let trim = Color32::from_rgb(170, 160, 190);
    let k = 0.35 + 0.65 * light;
    let lit = |c: Color32, face: f32| shade(c, k * face);
    let (front_z, back_z) = (z + 0.62, z - 0.62);
    let (front_y, back_y) = (0.90, 1.04); // the playfield climbs to the back
    let bottom = 0.70;

    // legs
    for (lx, lz) in [
        (-w + 0.03, front_z - 0.04),
        (w - 0.03, front_z - 0.04),
        (-w + 0.03, back_z + 0.04),
        (w - 0.03, back_z + 0.04),
    ] {
        out.push(quad(
            [
                v3(x + lx - 0.025, bottom, lz),
                v3(x + lx + 0.025, bottom, lz),
                v3(x + lx + 0.025, 0.0, lz),
                v3(x + lx - 0.025, 0.0, lz),
            ],
            Fill::Flat,
            lit(trim, 0.7),
            cam,
        ));
    }
    // body sides (the one facing the camera is the one drawn last)
    for s in [-1.0f32, 1.0] {
        let sx = x + s * w;
        out.push(quad(
            [
                v3(sx, back_y, back_z),
                v3(sx, front_y, front_z),
                v3(sx, bottom, front_z),
                v3(sx, bottom, back_z),
            ],
            Fill::Gradient(lit(body, 0.4)),
            lit(body, 0.75),
            cam,
        ));
    }
    // front panel, with the wheel as a decal where the coin door would be
    out.push(quad(
        [
            v3(x - w, front_y, front_z),
            v3(x + w, front_y, front_z),
            v3(x + w, bottom, front_z),
            v3(x - w, bottom, front_z),
        ],
        Fill::Flat,
        lit(body, 0.9),
        cam,
    ));
    // playfield glass: the clip of the selected table, a dark glass otherwise
    let pf = [
        v3(x - w + 0.02, back_y, back_z + 0.02),
        v3(x + w - 0.02, back_y, back_z + 0.02),
        v3(x + w - 0.02, front_y, front_z - 0.02),
        v3(x - w + 0.02, front_y, front_z - 0.02),
    ];
    match (&art.playfield, &art.glow) {
        (Some(t), _) | (None, Some(t)) => {
            let uv = super::tile_fit::fill_uv(t.size_vec2(), Vec2::new(0.68, 1.24));
            out.push(quad(
                pf,
                Fill::Tex(t.id(), uv),
                lit(Color32::WHITE, 1.0),
                cam,
            ));
        }
        (None, None) => out.push(quad(
            pf,
            Fill::Gradient(lit(Color32::from_rgb(10, 14, 30), 1.0)),
            lit(Color32::from_rgb(40, 50, 90), 1.0),
            cam,
        )),
    }
    // backbox: sides, top, front with the backglass
    let (bb_front, bb_back) = (back_z + 0.02, back_z - 0.24);
    let (bb_bottom, bb_top) = (back_y, back_y + 0.80);
    for s in [-1.0f32, 1.0] {
        let sx = x + s * w;
        out.push(quad(
            [
                v3(sx, bb_top, bb_back),
                v3(sx, bb_top, bb_front),
                v3(sx, bb_bottom, bb_front),
                v3(sx, bb_bottom, bb_back),
            ],
            Fill::Flat,
            lit(body, 0.6),
            cam,
        ));
    }
    out.push(quad(
        [
            v3(x - w, bb_top, bb_back),
            v3(x + w, bb_top, bb_back),
            v3(x + w, bb_top, bb_front),
            v3(x - w, bb_top, bb_front),
        ],
        Fill::Flat,
        lit(body, 1.1),
        cam,
    ));
    out.push(quad(
        [
            v3(x - w, bb_top, bb_front),
            v3(x + w, bb_top, bb_front),
            v3(x + w, bb_bottom, bb_front),
            v3(x - w, bb_bottom, bb_front),
        ],
        Fill::Flat,
        lit(trim, 0.5),
        cam,
    ));
    let glass = [
        v3(x - w + 0.03, bb_top - 0.04, bb_front + 0.002),
        v3(x + w - 0.03, bb_top - 0.04, bb_front + 0.002),
        v3(x + w - 0.03, bb_bottom + 0.06, bb_front + 0.002),
        v3(x - w + 0.03, bb_bottom + 0.06, bb_front + 0.002),
    ];
    match &art.backglass {
        Some(t) => {
            let uv = super::tile_fit::fill_uv(t.size_vec2(), Vec2::new(0.66, 0.70));
            out.push(quad(
                glass,
                Fill::Tex(t.id(), uv),
                lit(Color32::WHITE, 1.0),
                cam,
            ));
        }
        None => out.push(quad(
            glass,
            Fill::Flat,
            lit(Color32::from_rgb(30, 30, 50), 1.0),
            cam,
        )),
    }
    // the wheel as a topper above the backbox, and small on the front panel
    if let Some(t) = &art.wheel {
        let s = t.size_vec2();
        let aspect = if s.y > 0.0 { s.x / s.y } else { 1.0 };
        let th = 0.30f32;
        let tw = (th * aspect).min(2.0 * w);
        let th = tw / aspect;
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        out.push(quad(
            [
                v3(x - tw / 2.0, bb_top + 0.02 + th, bb_front - 0.06),
                v3(x + tw / 2.0, bb_top + 0.02 + th, bb_front - 0.06),
                v3(x + tw / 2.0, bb_top + 0.02, bb_front - 0.06),
                v3(x - tw / 2.0, bb_top + 0.02, bb_front - 0.06),
            ],
            Fill::Tex(t.id(), full),
            lit(Color32::WHITE, 1.0),
            cam,
        ));
        let dh = 0.16f32;
        let dw = (dh * aspect).min(2.0 * w - 0.1);
        let dh = dw / aspect;
        let cy = (front_y + bottom) / 2.0;
        out.push(quad(
            [
                v3(x - dw / 2.0, cy + dh / 2.0, front_z + 0.003),
                v3(x + dw / 2.0, cy + dh / 2.0, front_z + 0.003),
                v3(x + dw / 2.0, cy - dh / 2.0, front_z + 0.003),
                v3(x - dw / 2.0, cy - dh / 2.0, front_z + 0.003),
            ],
            Fill::Tex(t.id(), full),
            lit(Color32::WHITE, 0.9),
            cam,
        ));
    }
}

impl App {
    /// The arcade layout. Same contract as [`App::paint_carousel`].
    pub(super) fn paint_arcade(
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
        // the camera walks: a slower glide than the carousel's turn
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let pos = super::carousel::ring_step(self.carousel_pos, target, n, 1.0 - (-dt * 7.0).exp());
        let left = ((target - pos + n as f32 / 2.0).rem_euclid(n as f32) - n as f32 / 2.0).abs();
        self.carousel_pos = if left < 0.002 { target } else { pos };
        if left >= 0.002 {
            ui.ctx().request_repaint();
        }

        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let info_h = 80.0;
        let view = Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.bottom() - info_h));
        // the hall is cut at the name strip
        let painter = ui.painter_at(view);
        let info_painter = ui.painter_at(rect);
        let cam_x = 0.0; // the row moves, the camera stays: no float drift on long lists
        let focal = view.height().min(view.width() * 0.75) * 1.1;
        // launching: the camera dives to the selected playfield while VPX loads
        let launching = self.vpx_running.load(Ordering::Relaxed);
        match (launching, self.arcade_plunge) {
            (true, None) => self.arcade_plunge = Some(std::time::Instant::now()),
            (false, Some(_)) => self.arcade_plunge = None,
            _ => {}
        }
        let dive = self
            .arcade_plunge
            .map(|t0| (t0.elapsed().as_secs_f32() / PLUNGE_S).min(1.0))
            .unwrap_or(0.0);
        if dive > 0.0 && dive < 1.0 {
            ui.ctx().request_repaint();
        }
        let e = dive * dive * (3.0 - 2.0 * dive); // smoothstep
        let cam = Camera::new(
            v3(cam_x, 1.95, 2.9).lerp(v3(cam_x, 1.55, STEP_OUT + 1.05), e),
            v3(cam_x, 1.0, -0.2).lerp(v3(cam_x, 0.9, STEP_OUT - 0.35), e),
            focal,
            egui::pos2(view.center().x, view.center().y + view.height() * 0.05),
        );

        // the hall: floor and back wall
        let mut flat = egui::Mesh::default();
        let mut textured: Vec<egui::Mesh> = Vec::new();
        let hall = [
            quad(
                [
                    v3(-9.0, 0.0, -1.3),
                    v3(9.0, 0.0, -1.3),
                    v3(9.0, 0.0, 2.6),
                    v3(-9.0, 0.0, 2.6),
                ],
                Fill::Gradient(Color32::from_rgb(34, 22, 48)),
                Color32::from_rgb(8, 6, 14),
                &cam,
            ),
            quad(
                [
                    v3(-9.0, 3.2, -1.3),
                    v3(9.0, 3.2, -1.3),
                    v3(9.0, 0.0, -1.3),
                    v3(-9.0, 0.0, -1.3),
                ],
                Fill::Gradient(Color32::from_rgb(22, 14, 34)),
                Color32::from_rgb(6, 4, 10),
                &cam,
            ),
        ];
        painter.rect_filled(rect, 0.0, Color32::from_rgb(6, 4, 10));
        for f in &hall {
            add_face(&mut flat, &mut textured, &cam, f);
        }
        // a neon line along the wall, shifting with the walk
        let neon = [
            v3(-9.0, 2.36, -1.29),
            v3(9.0, 2.36, -1.29),
            v3(9.0, 2.32, -1.29),
            v3(-9.0, 2.32, -1.29),
        ];
        add_face(
            &mut flat,
            &mut textured,
            &cam,
            &quad(neon, Fill::Flat, p.accent, &cam),
        );
        painter.add(egui::Shape::mesh(std::mem::take(&mut flat)));
        for m in textured.drain(..) {
            painter.add(egui::Shape::mesh(m));
        }

        // the cabinets, far ones first
        let side = SIDE.min(((n as i64) - 1) / 2);
        let base = self.carousel_pos.round();
        let mut cabinets: Vec<(f32, usize, f32)> = Vec::new(); // (|offset|, fpos, offset)
        for k in -side..=side {
            let place = base + k as f32;
            let off = place - self.carousel_pos;
            let fpos = (place as i64).rem_euclid(n as i64) as usize;
            cabinets.push((off.abs(), fpos, off));
        }
        cabinets.sort_by(|a, b| b.0.total_cmp(&a.0));

        let sel = self.selected_table;
        // the clip's last picture stays on the playfield while VPX loads
        // (the player is stopped then)
        let clip = match self.video_tile.as_ref().and_then(|v| v.texture().cloned()) {
            Some(t) => {
                self.arcade_pf_last = Some((sel, t.clone()));
                Some(t)
            }
            None => match &self.arcade_pf_last {
                Some((i, t)) if *i == sel => Some(t.clone()),
                _ => None,
            },
        };
        let allow_download = self.db.get_config("media_videos").as_deref() != Some("0");
        let mut hit: Vec<(usize, f32, Rect)> = Vec::new();
        for &(dist, fpos, off) in &cabinets {
            let idx = filtered[fpos];
            drawn.push(fpos);
            let light = (1.0 - dist).clamp(0.0, 1.0);
            let x = cam_x + off * SPACING;
            let z = STEP_OUT * light;
            let table = &self.tables[idx];
            let path = table.path.clone();
            let download = if allow_download {
                table.vps_id.clone()
            } else {
                None
            };
            // the real backglass (bg.png or the .directb2s), the table's
            // picture only when it has none or while it loads
            let backglass = self.backglasses.get(idx, &path, None).cloned().or_else(|| {
                if idx == sel {
                    self.bg_cache.best(idx)
                } else {
                    self.bg_cache.thumb(idx).cloned()
                }
            });
            let art = CabinetArt {
                backglass,
                playfield: if idx == sel { clip.clone() } else { None },
                glow: self.bg_cache.backdrop(idx).map(|b| b.blur.clone()),
                wheel: self.wheels.get(idx, &path, download.as_deref()).cloned(),
            };
            if idx == sel
                && let (Some((a, _)), Some((b, _))) = (
                    cam.project(v3(x - 0.55, 0.0, z)),
                    cam.project(v3(x + 0.55, 0.0, z)),
                )
            {
                // a glow on the floor under the selected cabinet
                painter.add(egui::Shape::ellipse_filled(
                    a.lerp(b, 0.5),
                    Vec2::new((b.x - a.x) * 0.5, (b.x - a.x) * 0.06),
                    p.accent.gamma_multiply(0.22),
                ));
            }
            let mut faces = Vec::new();
            cabinet_faces(x, z, light, &art, &cam, &mut faces);
            faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
            let mut bbox = Rect::NOTHING;
            for f in &faces {
                for c in f.corners {
                    if let Some((p, _)) = cam.project(c) {
                        bbox.extend_with(p);
                    }
                }
                add_face(&mut flat, &mut textured, &cam, f);
                // flush in order so textured and flat faces keep their depth order
                painter.add(egui::Shape::mesh(std::mem::take(&mut flat)));
                for m in textured.drain(..) {
                    painter.add(egui::Shape::mesh(m));
                }
            }
            hit.push((fpos, off, bbox.intersect(view)));
        }

        // clicks: the nearest cabinet under the pointer
        let mut clicked: Option<(usize, f32)> = None;
        for &(fpos, off, r) in hit.iter().rev() {
            if !r.is_positive() {
                continue;
            }
            let resp = ui.interact(
                r,
                ui.scope_id().with(("arcade", fpos)),
                egui::Sense::click(),
            );
            if resp.clicked() {
                clicked = Some((fpos, off));
                break;
            }
        }

        // the selected table's name, rating and place in the list
        if let Some(t) = self.tables.get(sel) {
            let y = rect.bottom() - info_h + 24.0;
            info_painter.text(
                egui::pos2(rect.center().x, y),
                egui::Align2::CENTER_CENTER,
                &t.name,
                egui::FontId::proportional(28.0),
                p.text,
            );
            let stars = t.user.rating_tenths as f32 / 10.0;
            for s in 0..5 {
                let c = egui::pos2(rect.center().x + (s as f32 - 2.0) * 26.0, y + 32.0);
                super::launcher_ui::paint_rating_star(
                    &info_painter,
                    c,
                    10.0,
                    (stars - s as f32).clamp(0.0, 1.0),
                    super::launcher_ui::STAR_GOLD,
                );
            }
            info_painter.text(
                egui::pos2(rect.right() - 16.0, y + 32.0),
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
