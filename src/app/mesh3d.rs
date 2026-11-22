//! A tiny 3D toolkit for the launcher layouts drawn with egui meshes: a
//! pinhole camera and textured quads cut into a grid so the textures keep
//! their perspective (egui maps textures affinely per triangle).

use egui::{Color32, Pos2, Rect, epaint::Vertex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub(super) const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub(super) fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    pub(super) fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    pub(super) fn scale(self, k: f32) -> V3 {
        v3(self.x * k, self.y * k, self.z * k)
    }
    pub(super) fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub(super) fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub(super) fn norm(self) -> V3 {
        self.scale(1.0 / self.dot(self).sqrt().max(1e-6))
    }
    pub(super) fn lerp(self, o: V3, t: f32) -> V3 {
        self.add(o.sub(self).scale(t))
    }
}

/// A pinhole camera looking from `eye` at `target`, `focal` in pixels.
pub(super) struct Camera {
    pub eye: V3,
    pub right: V3,
    pub up: V3,
    pub fwd: V3,
    pub focal: f32,
    pub center: Pos2,
}

impl Camera {
    pub(super) fn new(eye: V3, target: V3, focal: f32, center: Pos2) -> Self {
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
    pub(super) fn project(&self, p: V3) -> Option<(Pos2, f32)> {
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
pub(super) struct Face {
    pub corners: [V3; 4],
    pub fill: Fill,
    pub tint: Color32,
    pub depth: f32,
}

pub(super) enum Fill {
    Flat,
    /// A texture, with the part of it to show.
    Tex(egui::TextureId, Rect),
    /// Two colours, top to bottom.
    Gradient(Color32),
    /// A texture fading from the tint (top) to a second colour (bottom):
    /// reflections.
    TexFade(egui::TextureId, Rect, Color32),
}

/// Grid cells per side when a textured face is cut up for perspective.
pub(super) const SUBDIV: usize = 6;

pub(super) fn add_face(
    mesh_flat: &mut egui::Mesh,
    meshes: &mut Vec<egui::Mesh>,
    cam: &Camera,
    f: &Face,
) {
    let [tl, tr, br, bl] = f.corners;
    let (cells, tex, uv) = match &f.fill {
        Fill::Tex(id, uv) | Fill::TexFade(id, uv, _) => (SUBDIV, Some(*id), *uv),
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
            Fill::Gradient(bottom) | Fill::TexFade(_, _, bottom) => lerp_color(f.tint, *bottom, v),
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

pub(super) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_premultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

pub(super) fn shade(c: Color32, k: f32) -> Color32 {
    let m = |x: u8| (x as f32 * k).min(255.0) as u8;
    Color32::from_rgba_premultiplied(m(c.r()), m(c.g()), m(c.b()), c.a())
}

/// A box face list helper: an axis-aligned quad.
pub(super) fn quad(corners: [V3; 4], fill: Fill, tint: Color32, cam: &Camera) -> Face {
    let c = corners[0].lerp(corners[2], 0.5);
    let depth = c.sub(cam.eye).dot(cam.fwd);
    Face {
        corners,
        fill,
        tint,
        depth,
    }
}

/// Paint faces far to near, each flushed in turn so textured and flat faces
/// keep their order.
pub(super) fn paint_faces(painter: &egui::Painter, cam: &Camera, faces: &mut [Face]) {
    faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    for f in faces.iter() {
        let mut flat = egui::Mesh::default();
        let mut textured = Vec::new();
        add_face(&mut flat, &mut textured, cam, f);
        if !flat.vertices.is_empty() {
            painter.add(egui::Shape::mesh(flat));
        }
        for m in textured {
            painter.add(egui::Shape::mesh(m));
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
