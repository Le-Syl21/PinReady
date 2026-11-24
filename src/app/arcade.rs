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
    wheel: Option<egui::TextureHandle>,
    /// The DMD clip (selected machine) or picture; without one the DMD shows
    /// the top of the backglass in orange.
    dmd: Option<egui::TextureHandle>,
}

/// A pinball cabinet's measures (metres, a standard widebody machine).
mod size {
    /// Half the cabinet's width.
    pub const HALF_W: f32 = 0.37;
    /// Cabinet length, lockbar to back.
    pub const LENGTH: f32 = 1.40;
    /// Height of the cabinet's bottom (the legs).
    pub const BOTTOM: f32 = 0.50;
    /// Height of the top edge at the lockbar and at the back. Steeper than
    /// a real machine (16° rather than 6.5°), so the playfield faces the
    /// camera a little.
    pub const TOP_FRONT: f32 = 0.92;
    pub const TOP_BACK: f32 = TOP_FRONT + LENGTH * 0.2867; // tan 16°
    /// Side rails and lockbar.
    pub const RAIL: f32 = 0.035;
    pub const LOCKBAR: f32 = 0.08;
    /// Backbox: height, depth, how far it overhangs the cabinet's back.
    pub const BB_H: f32 = 0.82;
    pub const BB_D: f32 = 0.30;
    pub const BB_OVER: f32 = 0.03;
    /// The backglass's room at the top of the backbox; under it the speaker
    /// panel with the DMD window.
    pub const GLASS_H: f32 = 0.56;
    /// Where the glass ends, under the backbox's front (0 at the lockbar,
    /// 1 at the cabinet's back): the backbox stands on the slope there.
    pub const T_END: f32 = 1.0 - (BB_D - BB_OVER) / LENGTH;
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

/// The corners of an upright rectangle facing the camera (+z), from
/// (`x0`, `y0`) to (`x1`, `y1`) at depth `z`.
fn panel(x0: f32, y0: f32, x1: f32, y1: f32, z: f32) -> [V3; 4] {
    [v3(x0, y1, z), v3(x1, y1, z), v3(x1, y0, z), v3(x0, y0, z)]
}

/// A disc of radius `r` around `c` in the plane of `u`, `v` (`v` up), cut
/// into `segs` wedges. It shades from `inner` at a highlight up and to the
/// left of its centre to `outer` at the rim: a dome lit from above.
#[allow(clippy::too_many_arguments)]
fn disc(
    c: V3,
    u: V3,
    v: V3,
    r: f32,
    segs: usize,
    inner: Color32,
    outer: Color32,
    cam: &Camera,
    out: &mut Vec<Face>,
) {
    let hl = c.add(u.scale(-0.25 * r)).add(v.scale(0.3 * r));
    let rim = |i: usize| {
        let a = i as f32 / segs as f32 * std::f32::consts::TAU;
        c.add(u.scale(a.cos() * r)).add(v.scale(a.sin() * r))
    };
    for i in 0..segs {
        // a wedge is a quad with two corners on the highlight: its top row
        // takes `inner`, its rim `outer`
        let mut f = quad(
            [hl, hl, rim(i + 1), rim(i)],
            Fill::Gradient(outer),
            inner,
            cam,
        );
        f.depth = c.sub(cam.eye).dot(cam.fwd);
        out.push(f);
    }
}

/// A coin door as on the front of a real machine: a chrome bezel, the black
/// door, two coin entries with their lit price inserts, the lock, and the
/// coin return at the bottom.
fn coin_door(
    x: f32,
    front_z: f32,
    light: f32,
    cam: &Camera,
    out: &mut Vec<Face>,
    lit: &dyn Fn(Color32, f32) -> Color32,
) {
    use size::BOTTOM;
    let chrome = Color32::from_rgb(186, 176, 166);
    let black = Color32::from_rgb(12, 11, 14);
    let (hw, y0, y1) = (0.15f32, BOTTOM + 0.05, BOTTOM + 0.40);
    let z = front_z + 0.003;
    // the door stands proud of the cabinet: its shadow, down and right
    out.push(quad(
        panel(
            x - hw + 0.008,
            y0 - 0.012,
            x + hw + 0.01,
            y1 - 0.006,
            z - 0.001,
        ),
        Fill::Flat,
        Color32::from_black_alpha(130),
        cam,
    ));
    // bezel, then the door inset in it
    out.push(quad(
        panel(x - hw, y0, x + hw, y1, z),
        Fill::Gradient(lit(chrome, 0.22)),
        lit(chrome, 0.8),
        cam,
    ));
    out.push(quad(
        panel(
            x - hw + 0.012,
            y0 + 0.012,
            x + hw - 0.012,
            y1 - 0.012,
            z + 0.001,
        ),
        Fill::Gradient(lit(black, 0.7)),
        lit(Color32::from_rgb(34, 32, 38), 1.0),
        cam,
    ));
    let z = z + 0.002;
    let glow = 0.35 + 0.65 * light;
    for cx in [-0.068f32, 0.068] {
        let cx = x + cx;
        // the price insert: red plastic lit from behind, a hot centre
        let (py0, py1) = (y1 - 0.105, y1 - 0.05);
        out.push(quad(
            panel(cx - 0.034, py0, cx + 0.034, py1, z),
            Fill::Flat,
            lit(chrome, 0.8),
            cam,
        ));
        out.push(quad(
            panel(cx - 0.028, py0 + 0.006, cx + 0.028, py1 - 0.006, z + 0.001),
            Fill::Gradient(lit(Color32::from_rgb(150, 20, 12), glow)),
            lit(Color32::from_rgb(220, 70, 40), glow),
            cam,
        ));
        // the bulb behind it shows as a hot band across the plastic
        let mid = (py0 + py1) / 2.0;
        out.push(quad(
            panel(cx - 0.028, mid - 0.006, cx + 0.028, mid + 0.01, z + 0.002),
            Fill::Gradient(lit(Color32::from_rgb(230, 80, 40), glow)),
            lit(Color32::from_rgb(255, 190, 130), glow),
            cam,
        ));
        // the coin entry plate with its slot
        let (cy0, cy1) = (y1 - 0.2, y1 - 0.12);
        out.push(quad(
            panel(cx - 0.026, cy0, cx + 0.026, cy1, z),
            Fill::Gradient(lit(chrome, 0.35)),
            lit(chrome, 1.15),
            cam,
        ));
        out.push(quad(
            panel(cx - 0.003, cy0 + 0.018, cx + 0.003, cy1 - 0.012, z + 0.001),
            Fill::Flat,
            black,
            cam,
        ));
    }
    // the lock, between the coin entries
    let (u, v) = (v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0));
    disc(
        v3(x, y1 - 0.16, z),
        u,
        v,
        0.011,
        8,
        lit(chrome, 1.2),
        lit(chrome, 0.35),
        cam,
        out,
    );
    // the coin return: a chrome lip over a dark mouth
    out.push(quad(
        panel(x - 0.05, y0 + 0.04, x + 0.05, y0 + 0.09, z),
        Fill::Gradient(lit(chrome, 0.25)),
        lit(chrome, 1.0),
        cam,
    ));
    out.push(quad(
        panel(x - 0.04, y0 + 0.048, x + 0.04, y0 + 0.075, z + 0.001),
        Fill::Flat,
        black,
        cam,
    ));
}

/// Stack decals on the panel they sit on: `faces[0]` is the panel, the
/// others are painted after it in the order they were pushed, whatever
/// their centres' depths (a decal low on a panel seen from above has its
/// centre further away than the panel's).
fn layer(faces: &mut [Face]) {
    let Some(base) = faces.first().map(|f| f.depth) else {
        return;
    };
    for (i, f) in faces.iter_mut().enumerate().skip(1) {
        f.depth = base - 1e-4 * i as f32;
    }
}

/// The faces of one cabinet whose lockbar stands at `x`, `z`.
/// `light` 0..1 (1 = the selected one).
fn cabinet_faces(
    x: f32,
    z: f32,
    light: f32,
    art: &CabinetArt,
    hall: &HallTextures,
    cam: &Camera,
    out: &mut Vec<Face>,
) {
    use size::*;
    let w = HALF_W;
    let body = Color32::from_rgb(28, 26, 40);
    // chrome warmed by the bar's light
    let chrome = Color32::from_rgb(186, 176, 166);
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

    // legs at the four corners: chrome, tapering to a black leveler foot
    for (lx, lz) in [
        (-w + 0.04, front_z - 0.05),
        (w - 0.04, front_z - 0.05),
        (-w + 0.04, back_z + 0.06),
        (w - 0.04, back_z + 0.06),
    ] {
        let lx = x + lx;
        out.push(quad(
            [
                v3(lx - 0.034, BOTTOM, lz),
                v3(lx + 0.034, BOTTOM, lz),
                v3(lx + 0.022, 0.035, lz),
                v3(lx - 0.022, 0.035, lz),
            ],
            Fill::Gradient(lit(chrome, 0.3)),
            lit(chrome, 0.85),
            cam,
        ));
        // the leg's bright edge: chrome catches the light on its corner
        out.push(quad(
            [
                v3(lx - 0.004, BOTTOM, lz + 0.001),
                v3(lx + 0.006, BOTTOM, lz + 0.001),
                v3(lx + 0.004, 0.035, lz + 0.001),
                v3(lx - 0.002, 0.035, lz + 0.001),
            ],
            Fill::Gradient(lit(chrome, 0.5)),
            lit(chrome, 1.25),
            cam,
        ));
        out.push(quad(
            panel(lx - 0.02, 0.0, lx + 0.02, 0.035, lz + 0.002),
            Fill::Flat,
            lit(Color32::from_rgb(14, 13, 16), 1.0),
            cam,
        ));
    }
    // body sides, taller at the back, dressed with the backglass's art
    // (a band of it, as cabinet side art is), darker towards the floor
    let side_art = art.backglass.as_ref();
    for s in [-1.0f32, 1.0] {
        // a side turned away from the camera is not drawn: seen from inside
        // it would paint over the playfield
        if (cam.eye.x - (x + s * w)) * s <= 0.0 {
            continue;
        }
        let sx = x + s * w;
        // the back corner at the backbox's foot, so the side stays under it
        let corners = [
            v3(sx, TOP_FRONT + (TOP_BACK - TOP_FRONT) * T_END, back_z),
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
        // up to the backbox's front
        let end = top(sx, T_END, 0.0);
        out.push(quad(
            [
                v3(sx, end.y + 0.015, end.z),
                v3(sx, TOP_FRONT + 0.015, front_z),
                v3(sx, TOP_FRONT - 0.045, front_z),
                v3(sx, end.y - 0.045, end.z),
            ],
            Fill::Gradient(lit(chrome, 0.5)),
            lit(chrome, 0.95),
            cam,
        ));
    }
    // flipper buttons, one on each side near the lockbar
    for s in [-1.0f32, 1.0] {
        if (cam.eye.x - (x + s * w)) * s <= 0.0 {
            continue;
        }
        let c = v3(x + s * (w + 0.006), TOP_FRONT - 0.075, front_z - 0.11);
        let (u, v) = (v3(0.0, 0.0, 1.0), v3(0.0, 1.0, 0.0));
        disc(
            c,
            u,
            v,
            0.022,
            10,
            lit(chrome, 1.1),
            lit(chrome, 0.4),
            cam,
            out,
        );
        let c = c.add(v3(s * 0.004, 0.0, 0.0));
        disc(
            c,
            u,
            v,
            0.016,
            10,
            lit(Color32::from_rgb(255, 250, 240), 1.0),
            lit(Color32::from_rgb(170, 160, 150), 1.0),
            cam,
            out,
        );
    }
    // front panel: the cabinet's art carries round the front, darker
    // towards the floor
    let front_first = out.len();
    let front = [
        v3(x - w, TOP_FRONT, front_z),
        v3(x + w, TOP_FRONT, front_z),
        v3(x + w, BOTTOM, front_z),
        v3(x - w, BOTTOM, front_z),
    ];
    out.push(match side_art {
        Some(t) => {
            let uv =
                super::tile_fit::fill_uv(t.size_vec2(), Vec2::new(2.0 * w, TOP_FRONT - BOTTOM));
            quad(
                front,
                Fill::TexFade(t.id(), uv, lit(Color32::from_gray(35), 1.0)),
                lit(Color32::from_gray(125), 1.0),
                cam,
            )
        }
        None => quad(front, Fill::Gradient(lit(body, 0.45)), lit(body, 0.9), cam),
    });
    coin_door(x, front_z, light, cam, out, &lit);
    // the start button, top left of the front: a lit dome in a chrome ring
    let sb = v3(x - w + 0.085, TOP_FRONT - 0.075, front_z + 0.004);
    let (u, v) = (v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0));
    disc(
        sb,
        u,
        v,
        0.03,
        12,
        lit(chrome, 1.2),
        lit(chrome, 0.35),
        cam,
        out,
    );
    let glow = 0.45 + 0.55 * light;
    disc(
        sb.add(v3(0.0, 0.0, 0.006)),
        u,
        v,
        0.022,
        12,
        lit(Color32::from_rgb(255, 236, 190), glow),
        lit(Color32::from_rgb(220, 110, 30), glow),
        cam,
        out,
    );

    // the plunger, right of the coin door, under the lockbar
    // a chrome escutcheon on the front, the rod with its housing sleeve,
    // and a black domed knob
    let (py, px) = (TOP_FRONT - 0.11, x + w - 0.085);
    let (u, v) = (v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0));
    disc(
        v3(px, py, front_z + 0.004),
        u,
        v,
        0.034,
        14,
        lit(chrome, 1.25),
        lit(chrome, 0.3),
        cam,
        out,
    );
    layer(&mut out[front_first..]);

    // the playfield fills the glass, between the rails, from the lockbar to
    // the back (the picture slightly stretched to the real playfield's shape)
    let iw = w - RAIL;
    let t0 = LOCKBAR / LENGTH;
    let t1 = T_END - 0.02 / LENGTH;
    let pf = [
        top(x - iw, t1, 0.0),
        top(x + iw, t1, 0.0),
        top(x + iw, t0, 0.0),
        top(x - iw, t0, 0.0),
    ];
    out.push(match &art.playfield {
        Some(t) => quad(pf, Fill::Tex(t.id(), full), lit(Color32::WHITE, 1.0), cam),
        // no picture yet: dark glass
        None => quad(
            pf,
            Fill::Gradient(lit(Color32::from_rgb(8, 8, 14), 1.0)),
            lit(Color32::from_rgb(24, 22, 34), 1.0),
            cam,
        ),
    });
    // the sleeve then the rod: their top (lit) and their front-facing
    // underside (in shade), so they read round
    for (r, z0, z1, k) in [
        (0.011f32, 0.004f32, 0.04f32, 0.9f32),
        (0.0065, 0.04, 0.105, 1.15),
    ] {
        let (a, b) = (front_z + z0, front_z + z1);
        out.push(quad(
            [
                v3(px - r, py + r * 0.7, b),
                v3(px + r, py + r * 0.7, b),
                v3(px + r, py + r * 0.7, a),
                v3(px - r, py + r * 0.7, a),
            ],
            Fill::Gradient(lit(chrome, 0.5 * k)),
            lit(chrome, 1.2 * k),
            cam,
        ));
        out.push(quad(
            [
                v3(px - r, py + r * 0.7, b),
                v3(px + r, py + r * 0.7, b),
                v3(px + r, py - r, b),
                v3(px - r, py - r, b),
            ],
            Fill::Gradient(lit(chrome, 0.25 * k)),
            lit(chrome, 0.7 * k),
            cam,
        ));
    }
    // the knob is a ball: a disc facing the camera reads as a sphere, with
    // a small hot reflection on its shoulder
    let knob = v3(px, py, front_z + 0.11);
    disc(
        knob,
        cam.right,
        cam.up,
        0.022,
        16,
        lit(Color32::from_rgb(96, 90, 98), 1.0),
        lit(Color32::from_rgb(8, 7, 10), 1.0),
        cam,
        out,
    );
    let mut spec = Vec::new();
    disc(
        knob.add(cam.right.scale(-0.007))
            .add(cam.up.scale(0.008))
            .sub(cam.fwd.scale(0.002)),
        cam.right,
        cam.up,
        0.0045,
        8,
        lit(Color32::from_rgb(255, 246, 236), 1.0),
        lit(Color32::from_rgb(120, 112, 116), 1.0),
        cam,
        &mut spec,
    );
    for f in &mut spec {
        f.depth -= 0.003;
    }
    out.extend(spec);
    // side rails and lockbar, chrome, standing above the glass: polished
    // metal mirrors the dark room, so a dark body with one bright streak
    for s in [-1.0f32, 1.0] {
        let first = out.len();
        let (a, b) = (x + s * w, x + s * iw);
        let rail = |p0: f32, p1: f32, lift: f32| {
            [
                top(p0.min(p1), T_END, lift),
                top(p0.max(p1), T_END, lift),
                top(p0.max(p1), 0.0, lift),
                top(p0.min(p1), 0.0, lift),
            ]
        };
        out.push(quad(
            rail(a, b, 0.015),
            Fill::Gradient(lit(chrome, 0.55)),
            lit(chrome, 0.28),
            cam,
        ));
        let m = x + s * (iw + RAIL * 0.55);
        out.push(quad(
            rail(m - 0.003, m + 0.003, 0.0152),
            Fill::Gradient(lit(chrome, 1.35)),
            lit(chrome, 0.75),
            cam,
        ));
        layer(&mut out[first..]);
    }
    let first = out.len();
    out.push(quad(
        [
            top(x - w, t0, 0.02),
            top(x + w, t0, 0.02),
            top(x + w, 0.0, 0.02),
            top(x - w, 0.0, 0.02),
        ],
        Fill::Gradient(lit(chrome, 0.4)),
        lit(chrome, 0.3),
        cam,
    ));
    let tm = t0 * 0.45;
    out.push(quad(
        [
            top(x - w, tm + 0.004, 0.0202),
            top(x + w, tm + 0.004, 0.0202),
            top(x + w, tm - 0.004, 0.0202),
            top(x - w, tm - 0.004, 0.0202),
        ],
        Fill::Gradient(lit(chrome, 1.0)),
        lit(chrome, 1.35),
        cam,
    ));
    layer(&mut out[first..]);
    out.push(quad(
        [
            v3(x - w, TOP_FRONT + 0.02, front_z + 0.002),
            v3(x + w, TOP_FRONT + 0.02, front_z + 0.002),
            v3(x + w, TOP_FRONT - 0.008, front_z + 0.002),
            v3(x - w, TOP_FRONT - 0.008, front_z + 0.002),
        ],
        Fill::Gradient(lit(chrome, 0.18)),
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
    let bb_bottom = TOP_FRONT + (TOP_BACK - TOP_FRONT) * T_END;
    let bb_top = bb_bottom + BB_H;
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
    let bb_first = out.len();
    out.push(quad(
        panel(x - w, bb_bottom, x + w, bb_top, bb_front),
        Fill::Gradient(lit(Color32::from_rgb(8, 7, 10), 1.0)),
        lit(Color32::from_rgb(20, 18, 24), 1.0),
        cam,
    ));
    let gz = bb_front + 0.003;
    let glass_top = bb_top - 0.03;
    let glass_bottom = glass_top - GLASS_H;
    if let Some(t) = &art.backglass {
        let sz = t.size_vec2();
        let (gw, gh) = fit(sz.x / sz.y.max(1.0), 2.0 * w - 0.05, GLASS_H);
        let cy = glass_top - GLASS_H / 2.0;
        out.push(quad(
            panel(x - gw / 2.0, cy - gh / 2.0, x + gw / 2.0, cy + gh / 2.0, gz),
            Fill::Tex(t.id(), full),
            lit(Color32::WHITE, 1.0),
            cam,
        ));
    }
    // the trim between the glass and the speaker panel
    out.push(quad(
        panel(x - w, glass_bottom - 0.022, x + w, glass_bottom - 0.008, gz),
        Fill::Gradient(lit(chrome, 0.25)),
        lit(chrome, 0.8),
        cam,
    ));
    // speaker panel: two perforated grilles, the DMD between them, its dots
    // glowing orange
    let sp_top = glass_bottom - 0.035;
    let sp_mid = (sp_top + bb_bottom) / 2.0;
    let sp_h = (sp_top - bb_bottom - 0.03).max(0.05);
    let pitch = 0.035; // one grille tile: four holes
    for cx in [-w + 0.085, w - 0.085] {
        let (gx0, gx1) = (x + cx - 0.055, x + cx + 0.055);
        let (gy0, gy1) = (sp_mid - sp_h / 2.0, sp_mid + sp_h / 2.0);
        out.push(quad(
            panel(gx0, gy0, gx1, gy1, gz),
            Fill::Tex(
                hall.grille.id(),
                Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new((gx1 - gx0) / pitch, (gy1 - gy0) / pitch),
                ),
            ),
            lit(Color32::from_gray(150), 1.0),
            cam,
        ));
    }
    // the DMD between the grilles, its top level with theirs: a clip or
    // picture fills that room keeping its shape, the orange title is 4:1
    let room_w = 2.0 * w - 2.0 * 0.15;
    let (dw, dh) = match &art.dmd {
        Some(t) => {
            let sz = t.size_vec2();
            fit(sz.x / sz.y.max(1.0), room_w, sp_h)
        }
        None => (room_w, (room_w / 4.0).min(sp_h)),
    };
    let d_top = sp_mid + sp_h / 2.0;
    let d_mid = d_top - dh / 2.0;
    out.push(quad(
        panel(
            x - dw / 2.0 - 0.01,
            d_mid - dh / 2.0 - 0.01,
            x + dw / 2.0 + 0.01,
            d_mid + dh / 2.0 + 0.01,
            gz,
        ),
        Fill::Flat,
        lit(Color32::from_rgb(4, 4, 5), 1.0),
        cam,
    ));
    // the DMD shows the table's DMD clip or picture, whole; without one the
    // game's title (the top of its backglass) in orange plasma. Either way
    // seen through a mask of round dots
    let on = 0.4 + 0.6 * light;
    let dmd = panel(
        x - dw / 2.0,
        d_mid - dh / 2.0,
        x + dw / 2.0,
        d_mid + dh / 2.0,
        gz,
    );
    let plasma = Color32::from_rgb(255, 130, 40);
    out.push(match (&art.dmd, &art.backglass) {
        (Some(t), _) => quad(dmd, Fill::Tex(t.id(), full), shade(Color32::WHITE, on), cam),
        (None, Some(t)) => {
            let sz = t.size_vec2();
            let h = (sz.x / 4.0 / sz.y.max(1.0)).min(1.0);
            let uv = Rect::from_min_size(Pos2::new(0.0, 0.02), Vec2::new(1.0, h));
            quad(dmd, Fill::Tex(t.id(), uv), shade(plasma, on), cam)
        }
        (None, None) => quad(dmd, Fill::Flat, shade(plasma, 0.25 * on), cam),
    });
    out.push(quad(
        dmd,
        Fill::Tex(
            hall.dots.id(),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(128.0, 32.0)),
        ),
        Color32::WHITE,
        cam,
    ));
    layer(&mut out[bb_first..]);

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
    /// A speaker grille's perforated metal: four holes per tile.
    grille: egui::TextureHandle,
    /// A DMD's dot mask: black around one clear round dot, tiled 128 x 32.
    dots: egui::TextureHandle,
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
    // a small square texture from a function of (u, v) in 0..1
    let small = |n: usize, f: &dyn Fn(f32, f32) -> Color32| {
        let mut px = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                px.push(f((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32));
            }
        }
        egui::ColorImage::new([n, n], px)
    };
    let grille = small(32, &|u, v| {
        // staggered holes: the centres of the four quarters, offset rows
        let hole = [
            (0.25f32, 0.25f32),
            (0.75, 0.25),
            (0.0, 0.75),
            (0.5, 0.75),
            (1.0, 0.75),
        ]
        .iter()
        .map(|(hx, hy)| ((u - hx).powi(2) + (v - hy).powi(2)).sqrt())
        .fold(f32::MAX, f32::min);
        let a = ((hole - 0.12) / 0.05).clamp(0.0, 1.0);
        let metal = 44.0 + 14.0 * (1.0 - v);
        let g = (6.0 + (metal - 6.0) * a) as u8;
        Color32::from_rgb(g, g, (g as f32 * 1.08).min(255.0) as u8)
    });
    let dots = small(16, &|u, v| {
        let r = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
        // black between the dots, clear inside them
        let a = ((r - 0.3) / 0.1).clamp(0.0, 1.0);
        Color32::from_black_alpha((a * 245.0) as u8)
    });
    let opts = egui::TextureOptions::LINEAR_REPEAT;
    let mip = opts.with_mipmap_mode(Some(egui::TextureFilter::Linear));
    let t = HallTextures {
        grille: ctx.load_texture("arcade/grille", grille, mip),
        dots: ctx.load_texture("arcade/dots", dots, mip),
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
        let info_h = super::launcher_ui::caption_height(rect);
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
            // a mahogany picture rail high on the carpeted wall
            quad(
                [
                    v3(-9.0, 2.62, -1.28),
                    v3(9.0, 2.62, -1.28),
                    v3(9.0, 2.58, -1.28),
                    v3(-9.0, 2.58, -1.28),
                ],
                Fill::Gradient(Color32::from_rgb(20, 9, 6)),
                Color32::from_rgb(120, 64, 36),
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
        let dmd_clip = self.video_dmd.as_ref().and_then(|v| v.texture().cloned());
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
            let art = CabinetArt {
                backglass,
                // the selected one plays its clip, the others show the
                // playfield picture
                playfield: if idx == sel { clip.clone() } else { None }.or_else(|| {
                    self.playfields
                        .get(idx, &path, download.as_deref())
                        .cloned()
                }),
                wheel: self.wheels.get(idx, &path, download.as_deref()).cloned(),
                // the selected one plays its DMD clip, the others show the
                // DMD picture
                dmd: if idx == sel { dmd_clip.clone() } else { None }
                    .or_else(|| self.dmds.get(idx, &path, download.as_deref()).cloned()),
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
                    tint(0.17 + 0.25 * light),
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
            cabinet_faces(x, z, light, &art, &tex, &cam, &mut faces);
            faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
            let mut bbox = Rect::NOTHING;
            for f in &faces {
                for c in f.corners {
                    if let Some((p, _)) = cam.project(c) {
                        bbox.extend_with(p);
                    }
                }
                // flat faces gather in one mesh; it is flushed before each
                // textured face so the depth order holds
                if matches!(f.fill, Fill::Tex(..) | Fill::TexFade(..)) {
                    painter.add(egui::Shape::mesh(std::mem::take(&mut flat)));
                    add_face(&mut flat, &mut textured, &cam, f);
                    for m in textured.drain(..) {
                        painter.add(egui::Shape::mesh(m));
                    }
                } else {
                    add_face(&mut flat, &mut textured, &cam, f);
                }
            }
            painter.add(egui::Shape::mesh(std::mem::take(&mut flat)));
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
            super::launcher_ui::paint_caption(
                &info_painter,
                rect,
                t,
                target as usize + 1,
                n,
                &super::launcher_ui::CaptionColors {
                    name: p.text,
                    stars: super::launcher_ui::STAR_GOLD,
                    place: p.text_faint,
                },
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
