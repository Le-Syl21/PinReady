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
const SPACING: f32 = 1.1;
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

/// A pinball cabinet's measures (metres, a standard widebody machine).
mod size {
    /// Half the cabinet's width.
    pub const HALF_W: f32 = 0.37;
    /// Cabinet length, lockbar to back.
    pub const LENGTH: f32 = 1.40;
    /// Height of the cabinet's bottom (the legs).
    pub const BOTTOM: f32 = 0.50;
    /// Height of the top edge at the lockbar and at the back: the playfield
    /// climbs about 6.5 degrees.
    pub const TOP_FRONT: f32 = 0.92;
    pub const TOP_BACK: f32 = 1.08;
    /// Side rails and lockbar.
    pub const RAIL: f32 = 0.035;
    pub const LOCKBAR: f32 = 0.08;
    /// The apron at the front of the playfield.
    pub const APRON: f32 = 0.12;
    /// Backbox: height, depth, how far it overhangs the cabinet's back.
    pub const BB_H: f32 = 0.82;
    pub const BB_D: f32 = 0.30;
    pub const BB_OVER: f32 = 0.03;
    /// The backglass's room at the top of the backbox; under it the speaker
    /// panel with the DMD window.
    pub const GLASS_H: f32 = 0.56;
}

/// `rect` of aspect `aspect` (width / height) fitted inside `w`×`h`: its
/// width and height.
fn fit(aspect: f32, w: f32, h: f32) -> (f32, f32) {
    if aspect <= 0.0 {
        return (w, h);
    }
    if w / h > aspect {
        (h * aspect, h)
    } else {
        (w, w / aspect)
    }
}

/// The faces of one cabinet whose lockbar stands at `x`, `z`.
/// `light` 0..1 (1 = the selected one).
fn cabinet_faces(x: f32, z: f32, light: f32, art: &CabinetArt, cam: &Camera, out: &mut Vec<Face>) {
    use size::*;
    let w = HALF_W;
    let body = Color32::from_rgb(28, 26, 40);
    let chrome = Color32::from_rgb(178, 176, 188);
    let k = 0.35 + 0.65 * light;
    let lit = |c: Color32, face: f32| shade(c, k * face);
    let (front_z, back_z) = (z, z - LENGTH);
    // a point on the top plane, `t` = 0 at the lockbar … 1 at the back,
    // `lift` above it
    let top = |px: f32, t: f32, lift: f32| {
        v3(
            px,
            TOP_FRONT + (TOP_BACK - TOP_FRONT) * t + lift,
            front_z - LENGTH * t,
        )
    };
    let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

    // legs, chrome, at the four corners
    for (lx, lz) in [
        (-w + 0.04, front_z - 0.06),
        (w - 0.04, front_z - 0.06),
        (-w + 0.04, back_z + 0.06),
        (w - 0.04, back_z + 0.06),
    ] {
        out.push(quad(
            [
                v3(x + lx - 0.03, BOTTOM, lz),
                v3(x + lx + 0.03, BOTTOM, lz),
                v3(x + lx + 0.03, 0.0, lz),
                v3(x + lx - 0.03, 0.0, lz),
            ],
            Fill::Gradient(lit(chrome, 0.35)),
            lit(chrome, 0.8),
            cam,
        ));
    }
    // body sides, taller at the back, dressed with the backglass's art
    // (a band of it, as cabinet side art is), darker towards the floor
    let side_art = art.backglass.as_ref().or(art.glow.as_ref());
    for s in [-1.0f32, 1.0] {
        // a side turned away from the camera is not drawn: seen from inside
        // it would paint over the playfield
        if (cam.eye.x - (x + s * w)) * s <= 0.0 {
            continue;
        }
        let sx = x + s * w;
        let corners = [
            v3(sx, TOP_BACK, back_z),
            v3(sx, TOP_FRONT, front_z),
            v3(sx, BOTTOM, front_z),
            v3(sx, BOTTOM, back_z),
        ];
        out.push(match side_art {
            Some(t) => {
                let uv = super::tile_fit::fill_uv(t.size_vec2(), Vec2::new(LENGTH, 0.5));
                quad(
                    corners,
                    Fill::TexFade(t.id(), uv, lit(Color32::from_gray(70), 1.0)),
                    lit(Color32::from_gray(225), 1.0),
                    cam,
                )
            }
            None => quad(
                corners,
                Fill::Gradient(lit(body, 0.4)),
                lit(body, 0.75),
                cam,
            ),
        });
    }
    // side armour: the chrome strip along the top of each side
    for s in [-1.0f32, 1.0] {
        // a side turned away from the camera is not drawn: seen from inside
        // it would paint over the playfield
        if (cam.eye.x - (x + s * w)) * s <= 0.0 {
            continue;
        }
        let sx = x + s * (w + 0.004);
        out.push(quad(
            [
                v3(sx, TOP_BACK + 0.015, back_z),
                v3(sx, TOP_FRONT + 0.015, front_z),
                v3(sx, TOP_FRONT - 0.045, front_z),
                v3(sx, TOP_BACK - 0.045, back_z),
            ],
            Fill::Gradient(lit(chrome, 0.5)),
            lit(chrome, 0.95),
            cam,
        ));
    }
    // front panel with the coin door and the start button
    out.push(quad(
        [
            v3(x - w, TOP_FRONT, front_z),
            v3(x + w, TOP_FRONT, front_z),
            v3(x + w, BOTTOM, front_z),
            v3(x - w, BOTTOM, front_z),
        ],
        Fill::Flat,
        lit(body, 0.9),
        cam,
    ));
    let door_mid = (TOP_FRONT + BOTTOM) / 2.0 - 0.01;
    let fz = front_z + 0.004;
    out.push(quad(
        [
            v3(x - 0.14, door_mid + 0.15, fz),
            v3(x + 0.14, door_mid + 0.15, fz),
            v3(x + 0.14, door_mid - 0.15, fz),
            v3(x - 0.14, door_mid - 0.15, fz),
        ],
        Fill::Gradient(lit(chrome, 0.25)),
        lit(chrome, 0.55),
        cam,
    ));
    for cx in [-0.06f32, 0.06] {
        out.push(quad(
            [
                v3(x + cx - 0.035, door_mid + 0.09, fz + 0.002),
                v3(x + cx + 0.035, door_mid + 0.09, fz + 0.002),
                v3(x + cx + 0.035, door_mid + 0.03, fz + 0.002),
                v3(x + cx - 0.035, door_mid + 0.03, fz + 0.002),
            ],
            Fill::Flat,
            lit(Color32::from_rgb(200, 40, 30), 0.5 + 0.5 * light),
            cam,
        ));
    }
    out.push(quad(
        [
            v3(x - w + 0.06, TOP_FRONT - 0.05, fz),
            v3(x - w + 0.11, TOP_FRONT - 0.05, fz),
            v3(x - w + 0.11, TOP_FRONT - 0.10, fz),
            v3(x - w + 0.06, TOP_FRONT - 0.10, fz),
        ],
        Fill::Flat,
        lit(Color32::from_rgb(255, 150, 30), 0.4 + 0.6 * light),
        cam,
    ));

    // the playfield, between the rails, from the apron to the back
    let iw = w - RAIL;
    let t0 = LOCKBAR / LENGTH;
    let t_apron = (LOCKBAR + APRON) / LENGTH;
    let t1 = 1.0 - 0.02 / LENGTH;
    // the wood around the picture (always behind it), and the apron
    let mut wood = quad(
        [
            top(x - iw, t1, -0.004),
            top(x + iw, t1, -0.004),
            top(x + iw, t0, -0.004),
            top(x - iw, t0, -0.004),
        ],
        Fill::Flat,
        lit(Color32::from_rgb(14, 12, 20), 1.0),
        cam,
    );
    wood.depth += 0.5;
    out.push(wood);
    out.push(quad(
        [
            top(x - iw, t_apron, 0.002),
            top(x + iw, t_apron, 0.002),
            top(x + iw, t0, 0.002),
            top(x - iw, t0, 0.002),
        ],
        Fill::Gradient(lit(body, 0.5)),
        lit(chrome, 0.35),
        cam,
    ));
    let pf_len = LENGTH * (t1 - t_apron);
    match (&art.playfield, &art.glow) {
        (Some(t), _) => {
            // the whole clip, kept in shape, its top at the back
            let sz = t.size_vec2();
            let (pw, pl) = fit(sz.x / sz.y.max(1.0), 2.0 * iw, pf_len);
            let tb = t1;
            let tf = t1 - pl / LENGTH;
            out.push(quad(
                [
                    top(x - pw / 2.0, tb, 0.0),
                    top(x + pw / 2.0, tb, 0.0),
                    top(x + pw / 2.0, tf, 0.0),
                    top(x - pw / 2.0, tf, 0.0),
                ],
                Fill::Tex(t.id(), full),
                lit(Color32::WHITE, 1.0),
                cam,
            ));
        }
        (None, Some(t)) => {
            // a lit playfield seen through dusty glass
            let uv = super::tile_fit::fill_uv(t.size_vec2(), Vec2::new(2.0 * iw, pf_len));
            out.push(quad(
                [
                    top(x - iw, t1, 0.0),
                    top(x + iw, t1, 0.0),
                    top(x + iw, t_apron, 0.0),
                    top(x - iw, t_apron, 0.0),
                ],
                Fill::Tex(t.id(), uv),
                lit(Color32::from_gray(200), 1.0),
                cam,
            ));
        }
        (None, None) => {}
    }
    // side rails and lockbar, chrome, standing above the glass
    for s in [-1.0f32, 1.0] {
        let (a, b) = (x + s * w, x + s * iw);
        out.push(quad(
            [
                top(a.min(b), 1.0, 0.015),
                top(a.max(b), 1.0, 0.015),
                top(a.max(b), 0.0, 0.015),
                top(a.min(b), 0.0, 0.015),
            ],
            Fill::Gradient(lit(chrome, 1.15)),
            lit(chrome, 0.55),
            cam,
        ));
    }
    out.push(quad(
        [
            top(x - w, t0, 0.02),
            top(x + w, t0, 0.02),
            top(x + w, 0.0, 0.02),
            top(x - w, 0.0, 0.02),
        ],
        Fill::Gradient(lit(chrome, 0.6)),
        lit(chrome, 1.0),
        cam,
    ));
    out.push(quad(
        [
            v3(x - w, TOP_FRONT + 0.02, front_z + 0.002),
            v3(x + w, TOP_FRONT + 0.02, front_z + 0.002),
            v3(x + w, TOP_FRONT - 0.02, front_z + 0.002),
            v3(x - w, TOP_FRONT - 0.02, front_z + 0.002),
        ],
        Fill::Flat,
        lit(chrome, 0.7),
        cam,
    ));

    // the glass over a lit playfield: one soft diagonal reflection
    if art.playfield.is_some() {
        let sheen = Color32::from_white_alpha((26.0 * light) as u8);
        out.push(quad(
            [
                top(x - iw, 0.78, 0.01),
                top(x - iw, 0.66, 0.01),
                top(x + iw, 0.38, 0.01),
                top(x + iw, 0.50, 0.01),
            ],
            Fill::Flat,
            sheen,
            cam,
        ));
    }
    // backbox: standing on the cabinet's back
    let bb_front = back_z + BB_D - BB_OVER;
    let bb_back = back_z - BB_OVER;
    let (bb_bottom, bb_top) = (TOP_BACK, TOP_BACK + BB_H);
    for s in [-1.0f32, 1.0] {
        // a side turned away from the camera is not drawn: seen from inside
        // it would paint over the playfield
        if (cam.eye.x - (x + s * w)) * s <= 0.0 {
            continue;
        }
        let sx = x + s * w;
        out.push(quad(
            [
                v3(sx, bb_top, bb_back),
                v3(sx, bb_top, bb_front),
                v3(sx, bb_bottom, bb_front),
                v3(sx, bb_bottom, bb_back),
            ],
            Fill::Gradient(lit(body, 0.45)),
            lit(body, 0.7),
            cam,
        ));
        if let Some(t) = side_art {
            let uv = super::tile_fit::fill_uv(t.size_vec2(), Vec2::new(BB_D, BB_H));
            out.push(quad(
                [
                    v3(sx, bb_top - 0.03, bb_back + 0.02),
                    v3(sx, bb_top - 0.03, bb_front - 0.02),
                    v3(sx, bb_bottom + 0.03, bb_front - 0.02),
                    v3(sx, bb_bottom + 0.03, bb_back + 0.02),
                ],
                Fill::TexFade(t.id(), uv, lit(Color32::from_gray(80), 1.0)),
                lit(Color32::from_gray(210), 1.0),
                cam,
            ));
        }
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
    // the front: a dark frame, the backglass whole at the top, the speaker
    // panel with the DMD under it
    out.push(quad(
        [
            v3(x - w, bb_top, bb_front),
            v3(x + w, bb_top, bb_front),
            v3(x + w, bb_bottom, bb_front),
            v3(x - w, bb_bottom, bb_front),
        ],
        Fill::Flat,
        lit(Color32::from_rgb(12, 11, 18), 1.0),
        cam,
    ));
    let gz = bb_front + 0.003;
    let glass_top = bb_top - 0.03;
    let glass_bottom = glass_top - GLASS_H;
    if let Some(t) = &art.backglass {
        let sz = t.size_vec2();
        let (gw, gh) = fit(sz.x / sz.y.max(1.0), 2.0 * w - 0.06, GLASS_H);
        let cy = glass_top - GLASS_H / 2.0;
        out.push(quad(
            [
                v3(x - gw / 2.0, cy + gh / 2.0, gz),
                v3(x + gw / 2.0, cy + gh / 2.0, gz),
                v3(x + gw / 2.0, cy - gh / 2.0, gz),
                v3(x - gw / 2.0, cy - gh / 2.0, gz),
            ],
            Fill::Tex(t.id(), full),
            lit(Color32::WHITE, 1.0),
            cam,
        ));
    }
    // speaker panel: two grilles and the DMD window between them
    let sp_mid = (glass_bottom + bb_bottom) / 2.0;
    let sp_h = (glass_bottom - bb_bottom) - 0.05;
    for cx in [-w + 0.09, w - 0.09] {
        out.push(quad(
            [
                v3(x + cx - 0.05, sp_mid + sp_h / 2.0, gz),
                v3(x + cx + 0.05, sp_mid + sp_h / 2.0, gz),
                v3(x + cx + 0.05, sp_mid - sp_h / 2.0, gz),
                v3(x + cx - 0.05, sp_mid - sp_h / 2.0, gz),
            ],
            Fill::Flat,
            lit(Color32::from_rgb(34, 32, 44), 1.0),
            cam,
        ));
    }
    let (dw, dh) = (0.40f32, 0.10f32.min(sp_h));
    out.push(quad(
        [
            v3(x - dw / 2.0, sp_mid + dh / 2.0, gz),
            v3(x + dw / 2.0, sp_mid + dh / 2.0, gz),
            v3(x + dw / 2.0, sp_mid - dh / 2.0, gz),
            v3(x - dw / 2.0, sp_mid - dh / 2.0, gz),
        ],
        Fill::Gradient(lit(Color32::from_rgb(40, 14, 4), 1.0)),
        lit(Color32::from_rgb(90, 34, 8), 0.3 + 0.7 * light),
        cam,
    ));

    // the wheel as a topper above the backbox
    if let Some(t) = &art.wheel {
        let s = t.size_vec2();
        let aspect = if s.y > 0.0 { s.x / s.y } else { 1.0 };
        let (tw, th) = fit(aspect, 2.0 * w, 0.28);
        let tz = (bb_front + bb_back) / 2.0 + 0.05;
        out.push(quad(
            [
                v3(x - tw / 2.0, bb_top + 0.02 + th, tz),
                v3(x + tw / 2.0, bb_top + 0.02 + th, tz),
                v3(x + tw / 2.0, bb_top + 0.02, tz),
                v3(x - tw / 2.0, bb_top + 0.02, tz),
            ],
            Fill::Tex(t.id(), full),
            lit(Color32::WHITE, 1.0),
            cam,
        ));
    }
}

/// The hall's materials, made once and kept in egui's memory: a carpeted
/// wall that swallows the sound (a billiard hall's, warm wine red), a
/// mahogany wainscot under it, a patterned bar carpet on the floor.
#[derive(Clone)]
struct HallTextures {
    wall: egui::TextureHandle,
    wood: egui::TextureHandle,
    floor: egui::TextureHandle,
    /// A soft white disc fading to nothing: a light pool, or a shadow.
    pool: egui::TextureHandle,
    /// Clear in the middle, dark at the edges of the view.
    vignette: egui::TextureHandle,
}

/// A cheap repeatable hash noise, 0..1.
fn noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x
        .wrapping_mul(374_761_393)
        .wrapping_add(y.wrapping_mul(668_265_263))
        .wrapping_add(seed.wrapping_mul(2_147_483_647));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn hall_textures(ctx: &egui::Context) -> HallTextures {
    let id = egui::Id::unique("arcade-hall-textures");
    if let Some(t) = ctx.data(|d| d.get_temp::<HallTextures>(id)) {
        return t;
    }
    const N: usize = 128;
    let make = |f: &dyn Fn(u32, u32) -> [u8; 3]| {
        let mut px = Vec::with_capacity(N * N);
        for y in 0..N as u32 {
            for x in 0..N as u32 {
                let [r, g, b] = f(x, y);
                px.push(Color32::from_rgb(r, g, b));
            }
        }
        egui::ColorImage::new([N, N], px)
    };
    let c = |v: f32| v.clamp(0.0, 255.0) as u8;
    // carpet wall: fine fibres, a faint vertical nap
    let wall = make(&|x, y| {
        let n = noise(x, y, 1) * 0.6 + noise(x / 2, y / 2, 2) * 0.4;
        let nap = noise(x, 0, 3) * 0.25;
        let k = 0.78 + 0.32 * n + nap * 0.3;
        [c(96.0 * k), c(24.0 * k), c(30.0 * k)]
    });
    // wainscot: vertical mahogany boards, grain, a dark joint every board
    let wood = make(&|x, y| {
        let board = x % 32;
        let grain = ((y as f32 * 0.05 + noise(x / 3, 0, 4) * 6.0).sin() * 0.5 + 0.5) * 0.18;
        let k = 0.82 + grain + noise(x, y, 5) * 0.08;
        let k = if board < 2 { k * 0.45 } else { k };
        [c(112.0 * k), c(54.0 * k), c(30.0 * k)]
    });
    // bar carpet: dark wine with a small gold diamond pattern
    let floor = make(&|x, y| {
        let (u, v) = ((x % 32) as i32 - 16, (y % 32) as i32 - 16);
        let d = u.abs() + v.abs();
        let n = 0.85 + noise(x, y, 6) * 0.3;
        if (12..=13).contains(&d) {
            [c(120.0 * n), c(86.0 * n), c(40.0 * n)]
        } else if d < 4 {
            [c(70.0 * n), c(40.0 * n), c(22.0 * n)]
        } else {
            [c(52.0 * n), c(14.0 * n), c(20.0 * n)]
        }
    });
    let pool = {
        const P: usize = 64;
        let mut px = Vec::with_capacity(P * P);
        for y in 0..P {
            for x in 0..P {
                let (u, v) = (
                    (x as f32 + 0.5) / P as f32 * 2.0 - 1.0,
                    (y as f32 + 0.5) / P as f32 * 2.0 - 1.0,
                );
                let r = (u * u + v * v).sqrt().min(1.0);
                let a = (1.0 - r * r).powi(2);
                px.push(Color32::from_white_alpha((a * 255.0) as u8));
            }
        }
        egui::ColorImage::new([P, P], px)
    };
    let vignette = {
        const V: usize = 64;
        let mut px = Vec::with_capacity(V * V);
        for y in 0..V {
            for x in 0..V {
                let (u, v) = (
                    (x as f32 + 0.5) / V as f32 * 2.0 - 1.0,
                    (y as f32 + 0.5) / V as f32 * 2.0 - 1.0,
                );
                let r = (u * u * 0.8 + v * v).sqrt();
                let a = ((r - 0.55) / 0.75).clamp(0.0, 1.0);
                px.push(Color32::from_black_alpha((a * a * 210.0) as u8));
            }
        }
        egui::ColorImage::new([V, V], px)
    };
    let opts = egui::TextureOptions::LINEAR_REPEAT;
    let t = HallTextures {
        wall: ctx.load_texture("arcade/wall", wall, opts),
        wood: ctx.load_texture("arcade/wood", wood, opts),
        floor: ctx.load_texture("arcade/floor", floor, opts),
        pool: ctx.load_texture("arcade/pool", pool, egui::TextureOptions::LINEAR),
        vignette: ctx.load_texture("arcade/vignette", vignette, egui::TextureOptions::LINEAR),
    };
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// The colour a machine's light throws around it: its backglass's colour
/// brought up to full brightness, a warm white when it has none yet.
fn glow_colour(edge: Option<Color32>) -> Color32 {
    let c = edge.unwrap_or(Color32::from_rgb(255, 196, 140));
    let max = c.r().max(c.g()).max(c.b()).max(1) as f32;
    let k = 255.0 / max;
    // half way to warm white so a dark or grey backglass still glows
    lerp_color(
        Color32::from_rgb(
            (c.r() as f32 * k) as u8,
            (c.g() as f32 * k) as u8,
            (c.b() as f32 * k) as u8,
        ),
        Color32::from_rgb(255, 210, 160),
        0.4,
    )
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
            v3(cam_x, 2.0, 3.3).lerp(v3(cam_x, 1.5, STEP_OUT + 1.25), e),
            v3(cam_x, 1.1, -0.3).lerp(v3(cam_x, 0.95, STEP_OUT - 0.3), e),
            focal,
            egui::pos2(view.center().x, view.center().y + view.height() * 0.05),
        );

        // the hall: a bar carpet on the floor, a mahogany wainscot and a
        // carpeted wall above it. No lamps: like in a real pinball bar at
        // night, the machines are the light (see the glows below)
        let tex = hall_textures(ui.ctx());
        let mut flat = egui::Mesh::default();
        let mut textured: Vec<egui::Mesh> = Vec::new();
        // the walk slides the patterns with the row
        let slide = -self.carousel_pos * SPACING;
        let uv = |w: f32, h: f32, tile: f32| {
            Rect::from_min_size(Pos2::new(slide / tile, 0.0), Vec2::new(w / tile, h / tile))
        };
        let warm = Color32::from_rgb(255, 226, 190);
        let hall = [
            quad(
                [
                    v3(-9.0, 0.0, -1.3),
                    v3(9.0, 0.0, -1.3),
                    v3(9.0, 0.0, 3.0),
                    v3(-9.0, 0.0, 3.0),
                ],
                Fill::TexFade(tex.floor.id(), uv(18.0, 4.3, 0.6), shade(warm, 0.42)),
                shade(warm, 0.22),
                &cam,
            ),
            quad(
                [
                    v3(-9.0, 1.15, -1.3),
                    v3(9.0, 1.15, -1.3),
                    v3(9.0, 0.0, -1.3),
                    v3(-9.0, 0.0, -1.3),
                ],
                Fill::TexFade(tex.wood.id(), uv(18.0, 1.15, 0.9), shade(warm, 0.35)),
                shade(warm, 0.6),
                &cam,
            ),
            quad(
                [
                    v3(-9.0, 3.4, -1.3),
                    v3(9.0, 3.4, -1.3),
                    v3(9.0, 1.2, -1.3),
                    v3(-9.0, 1.2, -1.3),
                ],
                Fill::TexFade(tex.wall.id(), uv(18.0, 2.2, 0.5), shade(warm, 0.6)),
                shade(warm, 0.18),
                &cam,
            ),
            // the rail capping the wainscot
            quad(
                [
                    v3(-9.0, 1.2, -1.28),
                    v3(9.0, 1.2, -1.28),
                    v3(9.0, 1.15, -1.28),
                    v3(-9.0, 1.15, -1.28),
                ],
                Fill::Flat,
                Color32::from_rgb(58, 28, 16),
                &cam,
            ),
        ];
        painter.rect_filled(rect, 0.0, Color32::from_rgb(10, 6, 6));
        for f in &hall {
            add_face(&mut flat, &mut textured, &cam, f);
        }
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
            // the lockbar's place: the cabinet's middle on the row's line
            let z = STEP_OUT * light + size::LENGTH / 2.0;
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
            let glow = self
                .bg_cache
                .backdrop(idx)
                .map(|b| b.blur.clone())
                .or_else(|| backglass.clone());
            let art = CabinetArt {
                backglass,
                // the selected one plays its clip, the others show a still
                playfield: if idx == sel { clip.clone() } else { None }
                    .or_else(|| self.playfields.get(idx, &path, None).cloned()),
                glow,
                wheel: self.wheels.get(idx, &path, download.as_deref()).cloned(),
            };
            {
                // the machine is the light: its backglass throws its colour
                // on the wall behind it, its playfield on the carpet in
                // front; the selected one shines the most
                let zc = z - size::LENGTH / 2.0;
                let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                let c = glow_colour(self.bg_cache.backdrop(idx).map(|b| b.edge));
                let tint = |a: f32| {
                    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0) as u8)
                };
                let wall = quad(
                    [
                        v3(x - 0.95, 2.75, -1.29),
                        v3(x + 0.95, 2.75, -1.29),
                        v3(x + 0.95, 0.75, -1.29),
                        v3(x - 0.95, 0.75, -1.29),
                    ],
                    Fill::Tex(tex.pool.id(), full),
                    tint(0.10 + 0.30 * light),
                    &cam,
                );
                let floor = quad(
                    [
                        v3(x - 0.8, 0.002, zc - 0.6),
                        v3(x + 0.8, 0.002, zc - 0.6),
                        v3(x + 0.8, 0.002, z + 0.9),
                        v3(x - 0.8, 0.002, z + 0.9),
                    ],
                    Fill::Tex(tex.pool.id(), full),
                    tint(0.04 + 0.30 * light),
                    &cam,
                );
                // the cabinet's shadow on the carpet, where it stands
                let shadow = quad(
                    [
                        v3(x - 0.62, 0.003, zc - 0.95),
                        v3(x + 0.62, 0.003, zc - 0.95),
                        v3(x + 0.62, 0.003, z + 0.25),
                        v3(x - 0.62, 0.003, z + 0.25),
                    ],
                    Fill::Tex(tex.pool.id(), full),
                    Color32::from_black_alpha(170),
                    &cam,
                );
                add_face(&mut flat, &mut textured, &cam, &wall);
                add_face(&mut flat, &mut textured, &cam, &floor);
                add_face(&mut flat, &mut textured, &cam, &shadow);
                painter.add(egui::Shape::mesh(std::mem::take(&mut flat)));
                for m in textured.drain(..) {
                    painter.add(egui::Shape::mesh(m));
                }
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
        // a vignette: the eye goes to the lit machine in the middle
        painter.image(
            tex.vignette.id(),
            view,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

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
