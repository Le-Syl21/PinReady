//! The arcade layout: the tables stand in a row of pinball cabinets, the
//! camera walks along the row. Each cabinet shows its backglass in the
//! backbox and its wheel as a topper; the selected one steps forward, lit,
//! with its playfield clip on the playfield, and the others fade into the
//! hall. Moving the selection glides the camera to the next cabinet.
//!
//! Everything is drawn with egui meshes: each face of a cabinet is a quad
//! projected with a pinhole camera, cut into a small grid so the textures
//! keep their perspective, and the faces are painted far to near.

use super::*;
use egui::{Color32, Pos2, Rect, Vec2, epaint::Vertex};

/// Distance between two cabinets (metres, like everything here).
const SPACING: f32 = 1.3;
/// Cabinets drawn on each side of the camera.
const SIDE: i64 = 5;
/// How far the selected cabinet comes forward.
const STEP_OUT: f32 = 0.38;

#[derive(Clone, Copy, Debug, PartialEq)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn scale(self, k: f32) -> V3 {
        v3(self.x * k, self.y * k, self.z * k)
    }
    fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn norm(self) -> V3 {
        self.scale(1.0 / self.dot(self).sqrt().max(1e-6))
    }
    fn lerp(self, o: V3, t: f32) -> V3 {
        self.add(o.sub(self).scale(t))
    }
}

/// A pinhole camera looking from `eye` at `target`, `focal` in pixels.
struct Camera {
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
    focal: f32,
    center: Pos2,
}

impl Camera {
    fn new(eye: V3, target: V3, focal: f32, center: Pos2) -> Self {
        let fwd = target.sub(eye).norm();
        let right = fwd.cross(v3(0.0, 1.0, 0.0)).norm();
        let up = right.cross(fwd);
        Self {
            eye,
            right,
            up,
            fwd,
            focal,
            center,
        }
    }

    /// Screen position and depth of a point; `None` behind the camera.
    fn project(&self, p: V3) -> Option<(Pos2, f32)> {
        let d = p.sub(self.eye);
        let z = d.dot(self.fwd);
        if z < 0.05 {
            return None;
        }
        let x = d.dot(self.right) / z * self.focal;
        let y = d.dot(self.up) / z * self.focal;
        Some((Pos2::new(self.center.x + x, self.center.y - y), z))
    }
}

/// One face to paint: four corners (top-left, top-right, bottom-right,
/// bottom-left as the texture sees them), what covers it, and its depth.
struct Face {
    corners: [V3; 4],
    fill: Fill,
    tint: Color32,
    depth: f32,
}

enum Fill {
    Flat,
    /// A texture, with the part of it to show.
    Tex(egui::TextureId, Rect),
    /// Two colours, top to bottom.
    Gradient(Color32),
}

/// Grid cells per side when a textured face is cut up for perspective.
const SUBDIV: usize = 6;

fn add_face(mesh_flat: &mut egui::Mesh, meshes: &mut Vec<egui::Mesh>, cam: &Camera, f: &Face) {
    let [tl, tr, br, bl] = f.corners;
    let (cells, tex, uv) = match &f.fill {
        Fill::Tex(id, uv) => (SUBDIV, Some(*id), *uv),
        _ => (1, None, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
    };
    let mut pts: Vec<Option<Pos2>> = Vec::with_capacity((cells + 1) * (cells + 1));
    for j in 0..=cells {
        let v = j as f32 / cells as f32;
        let left = tl.lerp(bl, v);
        let right = tr.lerp(br, v);
        for i in 0..=cells {
            let u = i as f32 / cells as f32;
            pts.push(cam.project(left.lerp(right, u)).map(|(p, _)| p));
        }
    }
    if pts.iter().any(Option::is_none) {
        return;
    }
    let target: &mut egui::Mesh = match tex {
        Some(id) => {
            meshes.push(egui::Mesh::with_texture(id));
            meshes.last_mut().expect("just pushed")
        }
        None => mesh_flat,
    };
    let base = target.vertices.len() as u32;
    for j in 0..=cells {
        let v = j as f32 / cells as f32;
        let color = match &f.fill {
            Fill::Gradient(bottom) => lerp_color(f.tint, *bottom, v),
            _ => f.tint,
        };
        for i in 0..=cells {
            let u = i as f32 / cells as f32;
            let pos = pts[j * (cells + 1) + i].expect("checked above");
            let uv_p = match tex {
                Some(_) => Pos2::new(uv.min.x + u * uv.width(), uv.min.y + v * uv.height()),
                None => egui::epaint::WHITE_UV,
            };
            target.vertices.push(Vertex {
                pos,
                uv: uv_p,
                color,
            });
        }
    }
    let w = (cells + 1) as u32;
    for j in 0..cells as u32 {
        for i in 0..cells as u32 {
            let a = base + j * w + i;
            target
                .indices
                .extend_from_slice(&[a, a + 1, a + w, a + 1, a + w + 1, a + w]);
        }
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_premultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

fn shade(c: Color32, k: f32) -> Color32 {
    let m = |x: u8| (x as f32 * k).min(255.0) as u8;
    Color32::from_rgba_premultiplied(m(c.r()), m(c.g()), m(c.b()), c.a())
}

/// A box face list helper: an axis-aligned quad.
fn quad(corners: [V3; 4], fill: Fill, tint: Color32, cam: &Camera) -> Face {
    let c = corners[0].lerp(corners[2], 0.5);
    let depth = c.sub(cam.eye).dot(cam.fwd);
    Face {
        corners,
        fill,
        tint,
        depth,
    }
}

/// What a cabinet shows.
struct CabinetArt {
    backglass: Option<egui::TextureHandle>,
    playfield: Option<egui::TextureHandle>,
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
    match &art.playfield {
        Some(t) => {
            let uv = super::tile_fit::fill_uv(t.size_vec2(), Vec2::new(0.68, 1.24));
            out.push(quad(
                pf,
                Fill::Tex(t.id(), uv),
                lit(Color32::WHITE, 1.0),
                cam,
            ));
        }
        None => out.push(quad(
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
        let cam = Camera::new(
            v3(cam_x, 1.95, 2.9),
            v3(cam_x, 1.0, -0.2),
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
        let clip = self.video_tile.as_ref().and_then(|v| v.texture().cloned());
        let allow_download = self.db.get_config("media_videos").as_deref() != Some("0");
        let mut hit: Vec<(usize, f32, Rect)> = Vec::new();
        for &(dist, fpos, off) in &cabinets {
            let idx = filtered[fpos];
            drawn.push(fpos);
            let light = (1.0 - dist).clamp(0.0, 1.0);
            let x = cam_x + off * SPACING;
            let z = STEP_OUT * light;
            let table = &self.tables[idx];
            let dir = table.path.parent().map(|d| d.to_path_buf());
            let download = if allow_download {
                table.vps_id.clone()
            } else {
                None
            };
            let art = CabinetArt {
                backglass: if idx == sel {
                    self.bg_cache.best(idx)
                } else {
                    self.bg_cache.thumb(idx).cloned()
                },
                playfield: if idx == sel { clip.clone() } else { None },
                wheel: dir
                    .as_deref()
                    .and_then(|d| self.wheels.get(idx, d, download.as_deref()).cloned()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_projects_the_target_to_the_centre_and_keeps_left_left() {
        let c = egui::pos2(400.0, 300.0);
        let cam = Camera::new(v3(0.0, 2.0, 3.0), v3(0.0, 1.0, 0.0), 800.0, c);
        let (p, z) = cam.project(v3(0.0, 1.0, 0.0)).unwrap();
        assert!((p - c).length() < 1e-3 && z > 0.0);
        let (l, _) = cam.project(v3(-1.0, 1.0, 0.0)).unwrap();
        let (r, _) = cam.project(v3(1.0, 1.0, 0.0)).unwrap();
        assert!(l.x < c.x && r.x > c.x);
        let (hi, _) = cam.project(v3(0.0, 2.0, 0.0)).unwrap();
        assert!(hi.y < c.y, "up is up on screen");
        assert!(
            cam.project(v3(0.0, 2.0, 5.0)).is_none(),
            "behind the camera"
        );
    }

    #[test]
    fn a_textured_face_is_cut_into_a_grid() {
        let cam = Camera::new(
            v3(0.0, 2.0, 3.0),
            v3(0.0, 1.0, 0.0),
            800.0,
            egui::pos2(0.0, 0.0),
        );
        let face = quad(
            [
                v3(-0.3, 1.4, 0.0),
                v3(0.3, 1.4, 0.0),
                v3(0.3, 1.0, 0.0),
                v3(-0.3, 1.0, 0.0),
            ],
            Fill::Tex(
                egui::TextureId::Managed(1),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            ),
            Color32::WHITE,
            &cam,
        );
        let mut flat = egui::Mesh::default();
        let mut tex = Vec::new();
        add_face(&mut flat, &mut tex, &cam, &face);
        assert_eq!(tex.len(), 1);
        assert_eq!(tex[0].vertices.len(), (SUBDIV + 1) * (SUBDIV + 1));
        assert_eq!(tex[0].indices.len(), SUBDIV * SUBDIV * 6);
        assert!(flat.vertices.is_empty());
    }
}
