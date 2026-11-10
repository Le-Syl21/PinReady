//! Give a `.directb2s` the DMD area VPX's B2S plugin needs.
//!
//! A backglass made for a full DMD (the "FullDMD" artwork) carries a frame
//! image, `<DMDImage>`, with a dark hole where the DMD should show. The B2S
//! plugin of VPX standalone (build 6577 and later) draws the DMD in that hole
//! only when the document says where it is: "a display presence in the
//! document means enabled". A file without a `<Displays><DMD .../></Displays>`
//! element leaves the hole black, even though the DMD itself runs (the table
//! script, PinMAME or FlexDMD, produces it). Backglasses written for the old
//! B2S server never had that element, because it found the hole by itself.
//!
//! This module finds the hole in the frame image (the largest dark rectangle
//! with the DMD's 4:1 shape), and writes the missing element just before
//! `</DirectB2SData>`, after keeping the original file. Files that already
//! have a `<Displays>` element, or no frame image, are left alone.
//!
//! Games with a video screen instead of a dot matrix (Mr. Game, Bally Vidpin,
//! Gottlieb Caveman) have a nearly square frame. When no 4:1 hole exists, a
//! nearly square one gets a display that names the CRT/LCD screen as its
//! source, which stays black on a game that has none.

use anyhow::{Context, Result, bail};
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::rom_displays::Screen;
use std::path::{Path, PathBuf};

/// The year in a table folder name written the usual way, "Name (Maker 1991)".
fn year_of(name: &str) -> Option<u32> {
    let inner = name.trim_end().strip_suffix(')')?;
    let inner = &inner[inner.rfind('(')? + 1..];
    let year = inner.rsplit(' ').next()?;
    (year.len() == 4).then(|| year.parse().ok()).flatten()
}

/// The screen the game of a table draws its scores on, from the ROM its script
/// names (see `rom_displays.rs`). Without a known ROM, a table made before 1990
/// has no dot matrix; a newer or original one has one when its script talks
/// about a DMD.
pub fn screen_for_table(vpx: &Path) -> Screen {
    let Some(script) = crate::vpinplay::read_script(vpx) else {
        return Screen::Unknown;
    };
    let rom = crate::vpinplay::rom_of(&script);
    if !rom.is_empty() {
        let known = crate::rom_displays::screen_of(&rom);
        if known != Screen::Unknown {
            return known;
        }
    }
    let name = vpx
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let old_table = !name.contains("Original") && year_of(&name).is_some_and(|y| y < 1990);
    if old_table || !script.to_ascii_lowercase().contains("dmd") {
        Screen::Segments
    } else {
        Screen::Unknown
    }
}

/// Persisted outcome of a check, in the `b2s_dmd` table.
pub mod status {
    /// The file already has a `<Displays>` element, or no frame image: nothing to add.
    pub const NOT_NEEDED: &str = "NotNeeded";
    /// The element was added.
    pub const APPLIED: &str = "Applied";
    /// The frame image has no dark hole of the DMD's shape: left alone.
    pub const NO_HOLE: &str = "NoHole";
    /// The file could not be read, decoded or written; retried at the next change.
    pub const FAILED: &str = "Failed";
}

/// Where the DMD is, in pixels of the frame image (origin top left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmdRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl DmdRect {
    /// The element VPX reads, for the ScoreView display (the third screen).
    fn element(&self) -> String {
        format!(
            "<Displays><DMD Target=\"ScoreView\" X=\"{}\" Y=\"{}\" W=\"{}\" H=\"{}\" /></Displays>",
            self.x, self.y, self.w, self.h
        )
    }
}

impl DmdRect {
    /// The element for a game whose screen is a CRT or LCD, not a dot matrix:
    /// without a source the display only looks for a dot matrix.
    fn video_element(&self) -> String {
        format!(
            "<Displays><DMD Target=\"ScoreView\" Source=\"ctrl://default/display?dmd_only=0\" X=\"{}\" Y=\"{}\" W=\"{}\" H=\"{}\" /></Displays>",
            self.x, self.y, self.w, self.h
        )
    }
}

/// What a check of a file found.
#[derive(Debug, PartialEq, Eq)]
pub enum Analysis {
    /// A `<Displays>` element exists: authored, leave it.
    HasDisplays,
    /// No `<DMDImage>`: no DMD frame to put a DMD in.
    NoDmdImage,
    /// The game has no dot matrix and no video screen (segment displays only).
    NoScreen,
    /// The frame image has no dark hole of the right shape.
    NoHole,
    /// The element is missing and this is where the DMD goes.
    Missing(DmdRect),
    /// The element is missing and the hole is for a video (CRT/LCD) screen.
    MissingVideo(DmdRect),
    /// No frame image at all, for a game that has a screen: a plain frame is
    /// added to the file, with the display in it (`true` for a video screen).
    NoFrame(DmdRect, bool),
}

/// DMD shape: 128x32.
const DMD_ASPECT: f32 = 4.0;
/// The hole may be this much narrower than the exact aspect.
const ASPECT_TOLERANCE: f32 = 0.25;
/// Shape of a video screen's frame, nearly square (Mr. Game 256x248, Vidpin 256x240).
const VIDEO_ASPECT: f32 = 1.0;
const VIDEO_ASPECT_MIN: f32 = 0.8;
const VIDEO_ASPECT_MAX: f32 = 1.5;
/// Luminance below which a pixel is part of the hole.
const DARK_THRESHOLD: u8 = 16;
/// A hole smaller than this share of the image is not a DMD frame.
const MIN_WIDTH_SHARE: f32 = 0.25;
const MIN_HEIGHT_SHARE: f32 = 0.06;

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// The base64 of the `Value="..."` attribute of the `<DMDImage>` element.
/// The XML escapes line breaks as `&#xD;&#xA;` inside the value, which are
/// dropped along with any other character reference and whitespace.
fn dmd_image_bytes(doc: &[u8]) -> Result<Option<Vec<u8>>> {
    let Some(tag) = find(doc, b"<DMDImage", 0) else {
        return Ok(None);
    };
    let Some(value) = find(doc, b"Value=\"", tag) else {
        return Ok(None);
    };
    let start = value + b"Value=\"".len();
    let end = find(doc, b"\"", start).context("unterminated DMDImage value")?;
    let mut b64 = Vec::with_capacity(end - start);
    let mut i = start;
    while i < end {
        match doc[i] {
            b'&' => {
                // character reference: skip to its `;`
                while i < end && doc[i] != b';' {
                    i += 1;
                }
            }
            c if c.is_ascii_whitespace() => {}
            c => b64.push(c),
        }
        i += 1;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .context("DMDImage is not valid base64")?;
    Ok(Some(bytes))
}

/// What a hole is looked for as, with the width over height of the screen.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// A dot matrix (4:1 for 128x32, 8:1 for 128x16, 3:1 for 192x64).
    Dmd(f32),
    /// A video screen (nearly square for Mr. Game's rotated CRT).
    Video(f32),
}

impl Shape {
    /// The width kept of a dark `w` x `h` rectangle, `None` when its shape is wrong.
    fn kept_width(self, w: usize, h: usize) -> Option<usize> {
        let ar = w as f32 / h as f32;
        match self {
            Shape::Dmd(a) => (ar >= a * (1.0 - ASPECT_TOLERANCE))
                .then(|| w.min((h as f32 * a * (1.0 + ASPECT_TOLERANCE)) as usize)),
            Shape::Video(a) => (a * VIDEO_ASPECT_MIN..=a * VIDEO_ASPECT_MAX)
                .contains(&ar)
                .then_some(w),
        }
    }

    fn aspect(self) -> f32 {
        match self {
            Shape::Dmd(a) | Shape::Video(a) => a,
        }
    }

    fn is_dmd(self) -> bool {
        matches!(self, Shape::Dmd(_))
    }
}

/// The largest all-dark rectangle of the wanted shape (histogram and stack
/// method), as `(area, x, y, w, h)`.
fn largest_dark_rect(
    dark: &[bool],
    width: usize,
    height: usize,
    shape: Shape,
) -> (usize, usize, usize, usize, usize) {
    let mut best = (0, 0, 0, 0, 0);
    let mut heights = vec![0usize; width];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for y in 0..height {
        let row = &dark[y * width..(y + 1) * width];
        for (run, &is_dark) in heights.iter_mut().zip(row) {
            *run = if is_dark { *run + 1 } else { 0 };
        }
        stack.clear();
        // one more step at the end, with a height of 0, empties the stack
        for (x, h) in heights
            .iter()
            .copied()
            .chain(std::iter::once(0))
            .enumerate()
        {
            let mut start = x;
            while let Some(&(sx, sh)) = stack.last() {
                if sh < h {
                    break;
                }
                stack.pop();
                let w = x - sx;
                if sh >= 20
                    && w >= 60
                    && let Some(fw) = shape.kept_width(w, sh)
                {
                    let area = fw * sh;
                    if area > best.0 {
                        best = (area, sx, y + 1 - sh, fw, sh);
                    }
                }
                start = sx;
            }
            stack.push((start, h));
        }
    }
    best
}

/// Luminance (0..255) of every pixel, with see-through pixels counted as black:
/// a transparent hole is as good as a dark one.
fn luminance(image: &image::DynamicImage) -> (Vec<u8>, usize, usize) {
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let luma = rgba
        .pixels()
        .map(|p| {
            if p[3] < 32 {
                0
            } else {
                ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000)
                    as u8
            }
        })
        .collect();
    (luma, width, height)
}

/// Mean luminance of a rectangle of the image.
fn mean_luma(luma: &[u8], width: usize, x: usize, y: usize, w: usize, h: usize) -> f32 {
    let mut sum = 0u64;
    for row in y..y + h {
        sum += luma[row * width + x..row * width + x + w]
            .iter()
            .map(|&v| u64::from(v))
            .sum::<u64>();
    }
    sum as f32 / (w * h).max(1) as f32
}

/// How much brighter the ring of pixels around a rectangle is than its inside.
/// A frame has a border that stands out from the hole; a patch of dark artwork
/// does not.
///
/// A hole that touches the edge of the image is rejected when `strict` (the
/// artwork may go on past the edge). A video frame often sits flush with the
/// bottom of its image: then only the sides inside the image make the ring,
/// and at least half of it has to be there.
fn edge_contrast(
    luma: &[u8],
    width: usize,
    height: usize,
    rect: (usize, usize, usize, usize),
    strict: bool,
) -> f32 {
    const RING: usize = 6;
    let (x, y, w, h) = rect;
    let touches = x < RING || y < RING || x + w + RING > width || y + h + RING > height;
    if touches && strict {
        return 0.0; // touches the edge of the image: the artwork goes on
    }
    // the ring rectangle, cut to the image
    let ox = x.saturating_sub(RING);
    let oy = y.saturating_sub(RING);
    let ow = (x + w + RING).min(width) - ox;
    let oh = (y + h + RING).min(height) - oy;
    let outer_area = (ow * oh) as f32;
    let inner_area = (w * h) as f32;
    let ring_area = outer_area - inner_area;
    let full_ring = ((w + 2 * RING) * (h + 2 * RING) - w * h) as f32;
    if ring_area < full_ring * 0.5 {
        return 0.0;
    }
    let outer = mean_luma(luma, width, ox, oy, ow, oh);
    let inner = mean_luma(luma, width, x, y, w, h);
    (outer * outer_area - inner * inner_area) / ring_area - inner
}

/// A hole that fills the image edge to edge: a frame drawn as one flat panel
/// (a grey or black rectangle with a thin outline) leaves no ring on most sides,
/// so `edge_contrast` cannot judge it. It is still a hole when it touches the
/// edge of the image, its inside (the outline left out) is one flat neutral
/// colour (artwork never is: gradients, colours, texture) and the side that has
/// a ring stands out from it.
fn flush_flat_hole(
    rgba: &image::RgbaImage,
    luma: &[u8],
    width: usize,
    height: usize,
    rect: (usize, usize, usize, usize),
    outline: usize,
) -> bool {
    const RING: usize = 6;
    /// How far each channel may stray from the panel's colour.
    const FLAT: i32 = 4;
    /// How far apart the three channels of the panel's colour may be (grey).
    const NEUTRAL: i32 = 8;
    let (x, y, w, h) = rect;
    let touches = x < RING || y < RING || x + w + RING > width || y + h + RING > height;
    // frames for a full DMD are big pictures: a small image is a logo, whose
    // black margin is not a hole
    if !touches || width < 1000 || w <= 2 * outline || h <= 2 * outline {
        return false;
    }
    let (ix, iy, iw, ih) = (x + outline, y + outline, w - 2 * outline, h - 2 * outline);
    let centre = rgba.get_pixel((ix + iw / 2) as u32, (iy + ih / 2) as u32).0;
    let (cmax, cmin) = (
        i32::from(centre[0].max(centre[1]).max(centre[2])),
        i32::from(centre[0].min(centre[1]).min(centre[2])),
    );
    if cmax - cmin > NEUTRAL || centre[3] < 255 {
        return false;
    }
    for row in iy..iy + ih {
        for col in ix..ix + iw {
            let p = rgba.get_pixel(col as u32, row as u32).0;
            if (0..3).any(|c| (i32::from(p[c]) - i32::from(centre[c])).abs() > FLAT) {
                return false;
            }
        }
    }
    // the sides of the ring that are inside the image: above and below
    let inner = f32::from(
        ((u32::from(centre[0]) * 299 + u32::from(centre[1]) * 587 + u32::from(centre[2]) * 114)
            / 1000) as u8,
    );
    let mut ring = 0f32;
    let mut count = 0usize;
    if y >= RING {
        ring += mean_luma(luma, width, x, y - RING, w, RING) * (w * RING) as f32;
        count += w * RING;
    }
    if y + h + RING <= height {
        ring += mean_luma(luma, width, x, y + h, w, RING) * (w * RING) as f32;
        count += w * RING;
    }
    if count == 0 {
        return false;
    }
    ((ring / count as f32) - inner).abs() >= 25.0
}

/// Grow a rectangle found inside a hole out to the frame around it.
///
/// The search finds the largest uniform rectangle, which can be smaller than the
/// hole when the hole is not quite uniform (noise, a gradient, a dim logo). From
/// that seed, each side moves outwards while the next line of pixels is, nearly
/// all of it, the colour of the seed: a few stray pixels do not stop it, the
/// outline of the frame does. A growth that makes the shape wrong, or that runs
/// away, is dropped.
fn grow_to_frame(
    luma: &[u8],
    width: usize,
    height: usize,
    seed: (usize, usize, usize, usize),
    aspect: f32,
) -> (usize, usize, usize, usize) {
    /// How far a pixel may be from the colour of the seed, in luminance levels.
    const TOLERANCE: f32 = 28.0;
    /// Share of a line that has to be of the seed's colour.
    const SHARE: usize = 88;
    /// The grown rectangle may cover at most this many times the seed's area.
    const MAX_GROWTH: usize = 5;
    let (x, y, w, h) = seed;
    let reference = mean_luma(luma, width, x, y, w, h);
    let similar = |v: u8| (f32::from(v) - reference).abs() <= TOLERANCE;
    let column_ok = |col: usize, y0: usize, y1: usize| {
        let n = (y0..y1)
            .filter(|&row| similar(luma[row * width + col]))
            .count();
        n * 100 >= (y1 - y0) * SHARE
    };
    let row_ok = |row: usize, x0: usize, x1: usize| {
        let n = luma[row * width + x0..row * width + x1]
            .iter()
            .filter(|&&v| similar(v))
            .count();
        n * 100 >= (x1 - x0) * SHARE
    };
    let (mut x0, mut y0, mut x1, mut y1) = (x, y, x + w, y + h);
    loop {
        let mut moved = false;
        if x0 > 0 && column_ok(x0 - 1, y0, y1) {
            x0 -= 1;
            moved = true;
        }
        if x1 < width && column_ok(x1, y0, y1) {
            x1 += 1;
            moved = true;
        }
        if y0 > 0 && row_ok(y0 - 1, x0, x1) {
            y0 -= 1;
            moved = true;
        }
        if y1 < height && row_ok(y1, x0, x1) {
            y1 += 1;
            moved = true;
        }
        if !moved || (x1 - x0) * (y1 - y0) > MAX_GROWTH * w * h {
            break;
        }
    }
    let (gw, gh) = (x1 - x0, y1 - y0);
    let ratio = gw as f32 / gh as f32;
    if (x1 - x0) * (y1 - y0) > MAX_GROWTH * w * h || ratio < aspect * 0.6 || ratio > aspect * 1.6 {
        return seed;
    }
    (x0, y0, gw, gh)
}

/// The shape `aspect` fitted in the middle of the rectangle `(x, y, w, h)`.
fn fit_aspect(x: usize, y: usize, w: usize, h: usize, aspect: f32) -> DmdRect {
    let (mut x, mut y, mut w, mut h) = (x, y, w, h);
    if w as f32 / h as f32 > aspect {
        let nw = (h as f32 * aspect) as usize;
        x += (w - nw) / 2;
        w = nw;
    } else {
        let nh = (w as f32 / aspect) as usize;
        y += (h - nh) / 2;
        h = nh;
    }
    DmdRect {
        x: x as u32,
        y: y as u32,
        w: w as u32,
        h: h as u32,
    }
}

/// Find the hole of a frame image and fit the DMD's shape in its middle.
///
/// The hole is not always pure black (a gradient, some noise), so the search is
/// repeated at growing thresholds. Of the dark rectangles found, those that
/// stand out from their surroundings (a border around them) and do not touch
/// the edge of the image are candidates, and the largest wins: a patch of dark
/// artwork has no border, and a larger threshold only grows the hole up to its
/// border.
#[cfg(test)]
fn find_hole(image: &image::DynamicImage, shape: Shape) -> Option<DmdRect> {
    find_hole_area(image, shape).map(|(_, rect)| rect)
}

/// `find_hole` with the area of the dark rectangle it found, to compare the shapes.
fn find_hole_area(image: &image::DynamicImage, shape: Shape) -> Option<(usize, DmdRect)> {
    find_hole_with(image, shape, false)
}

/// A video frame set in dark artwork has no border that stands out from the
/// artwork, so the contrast test cannot find it. As a last resort, a large
/// nearly square dark rectangle in the middle of the image is taken for it.
fn find_dark_video_area(image: &image::DynamicImage, aspect: f32) -> Option<(usize, DmdRect)> {
    find_hole_with(image, Shape::Video(aspect), true)
}

/// `relaxed` drops the contrast test for a dark rectangle that is centred
/// (within 8% of the image width) and covers at least a fifth of the image.
fn find_hole_with(
    image: &image::DynamicImage,
    shape: Shape,
    relaxed: bool,
) -> Option<(usize, DmdRect)> {
    const THRESHOLDS: [u8; 5] = [DARK_THRESHOLD, 24, 32, 44, 60];
    /// Width of the outline of a flat panel, left out of the DMD.
    const OUTLINE: usize = 10;
    /// A border at least this much brighter than the inside, in luminance levels.
    const MIN_CONTRAST: f32 = 25.0;
    let (luma, width, height) = luminance(image);
    if width < 64 || height < 24 {
        return None;
    }
    let rgba = image.to_rgba8();
    let mut best: Option<(usize, DmdRect)> = None;
    for threshold in THRESHOLDS {
        let mut dark: Vec<bool> = luma.iter().map(|&v| v < threshold).collect();
        // The largest dark rectangle may be artwork (a night sky): when it is
        // not a frame hole, blank it out and look at the next one
        for _ in 0..4 {
            let (area, x, y, w, h) = largest_dark_rect(&dark, width, height, shape);
            if area == 0 {
                break;
            }
            let big_enough = (w as f32) >= width as f32 * MIN_WIDTH_SHARE
                && (h as f32) >= height as f32 * MIN_HEIGHT_SHARE;
            let strict = shape.is_dmd();
            let accepted = if relaxed {
                let centre = (x * 2 + w) as f32 / 2.0;
                big_enough
                    && (centre - width as f32 / 2.0).abs() <= width as f32 * 0.08
                    && (w * h) as f32 >= (width * height) as f32 * 0.2
            } else {
                big_enough
                    && edge_contrast(&luma, width, height, (x, y, w, h), strict) >= MIN_CONTRAST
            };
            // a panel that fills the image edge to edge, outline included: the
            // DMD goes inside the outline
            let flush = !accepted
                && !relaxed
                && strict
                && big_enough
                && flush_flat_hole(&rgba, &luma, width, height, (x, y, w, h), OUTLINE);
            if accepted || flush {
                if best.is_none_or(|(a, _)| area > a) {
                    let (ix, iy, iw, ih) = if flush {
                        (x + OUTLINE, y + OUTLINE, w - 2 * OUTLINE, h - 2 * OUTLINE)
                    } else {
                        grow_to_frame(&luma, width, height, (x, y, w, h), shape.aspect())
                    };
                    best = Some((area, fit_aspect(ix, iy, iw, ih, shape.aspect())));
                }
                break;
            }
            for row in y..y + h {
                dark[row * width + x..row * width + x + w].fill(false);
            }
        }
    }
    best
}

/// The pixels that belong to a flat area, with sparse noise tolerated.
///
/// A pixel is flat when it differs from its right and lower neighbours by at
/// most `threshold` in every channel; the mask is then smoothed: a pixel stays
/// flat when nearly all of its 5x5 neighbourhood is, so a few dark specks in a panel
/// do not cut it in pieces (a thin outline between two flat areas, one line
/// of pixels, is still a boundary). See-through pixels count as black.
fn flat_mask(rgba: &image::RgbaImage, threshold: i32) -> Vec<bool> {
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let px = |x: usize, y: usize| -> [i32; 3] {
        let p = rgba.get_pixel(x as u32, y as u32).0;
        if p[3] < 32 {
            [0, 0, 0]
        } else {
            [i32::from(p[0]), i32::from(p[1]), i32::from(p[2])]
        }
    };
    let mut raw = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let a = px(x, y);
            let mut flat = true;
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx < width && ny < height {
                    let b = px(nx, ny);
                    if (0..3).any(|c| (a[c] - b[c]).abs() > threshold) {
                        flat = false;
                        break;
                    }
                }
            }
            raw[y * width + x] = u8::from(flat);
        }
    }
    // integral image of the raw mask, then the share of flat pixels in 5x5
    let stride = width + 1;
    let mut integral = vec![0u32; stride * (height + 1)];
    for y in 0..height {
        let mut row = 0u32;
        for x in 0..width {
            row += u32::from(raw[y * width + x]);
            integral[(y + 1) * stride + x + 1] = integral[y * stride + x + 1] + row;
        }
    }
    const R: usize = 2;
    let mut out = vec![false; width * height];
    for y in 0..height {
        let (y0, y1) = (y.saturating_sub(R), (y + R + 1).min(height));
        for x in 0..width {
            let (x0, x1) = (x.saturating_sub(R), (x + R + 1).min(width));
            let sum = integral[y1 * stride + x1] + integral[y0 * stride + x0]
                - integral[y0 * stride + x1]
                - integral[y1 * stride + x0];
            let area = ((x1 - x0) * (y1 - y0)) as u32;
            out[y * width + x] = sum * 100 >= area * 85;
        }
    }
    out
}

/// How much the colour of a rectangle differs from the ring of pixels around
/// it (mean absolute difference of the channels). A panel set in artwork
/// stands out whatever its colour; a patch of flat background does not.
fn colour_contrast(
    rgba: &image::RgbaImage,
    rect: (usize, usize, usize, usize),
    ring: usize,
) -> f32 {
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let (x, y, w, h) = rect;
    let ox = x.saturating_sub(ring);
    let oy = y.saturating_sub(ring);
    let ow = (x + w + ring).min(width) - ox;
    let oh = (y + h + ring).min(height) - oy;
    let full_ring = ((w + 2 * ring) * (h + 2 * ring) - w * h) as f64;
    let ring_area = (ow * oh - w * h) as f64;
    if ring_area < full_ring * 0.25 {
        return 0.0;
    }
    let sum = |rx: usize, ry: usize, rw: usize, rh: usize| -> [f64; 3] {
        let mut t = [0f64; 3];
        for row in ry..ry + rh {
            for col in rx..rx + rw {
                let p = rgba.get_pixel(col as u32, row as u32).0;
                for c in 0..3 {
                    t[c] += f64::from(p[c]);
                }
            }
        }
        t
    };
    let outer = sum(ox, oy, ow, oh);
    let inner = sum(x, y, w, h);
    let inner_area = (w * h) as f64;
    (0..3)
        .map(|c| ((outer[c] - inner[c]) / ring_area - inner[c] / inner_area).abs())
        .sum::<f64>() as f32
        / 3.0
}

/// Whether a panel stands out from its surroundings: a flat panel is often set
/// in a bezel that is nearly its own colour, so the ring close around it is
/// compared first, then a wider one that reaches the artwork past the bezel.
fn stands_out(rgba: &image::RgbaImage, rect: (usize, usize, usize, usize), min: f32) -> bool {
    let wide = (rgba.width() as usize / 60).max(24);
    colour_contrast(rgba, rect, 6) >= min || colour_contrast(rgba, rect, wide) >= min
}

/// A hole that is not dark: a flat panel (light grey, white, a plain colour)
/// of the screen's shape, set in artwork that stands out from it. This is the
/// last resort, after the search for dark holes found nothing.
fn find_flat_panel(
    image: &image::DynamicImage,
    shape: Shape,
    min_contrast: f32,
    lenient: bool,
) -> Option<(usize, DmdRect)> {
    const THRESHOLDS: [i32; 3] = [4, 8, 14];
    /// Inset from the border of the panel, so the DMD sits inside its frame.
    const INSET: usize = 4;
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    if width < 64 || height < 24 {
        return None;
    }
    let (luma, _, _) = luminance(image);
    let mut best: Option<(usize, DmdRect)> = None;
    for threshold in THRESHOLDS {
        let mut flat = flat_mask(&rgba, threshold);
        for _ in 0..12 {
            let (area, x, y, w, h) = largest_dark_rect(&flat, width, height, shape);
            if area == 0 {
                break;
            }
            let big_enough = (w as f32) >= width as f32 * MIN_WIDTH_SHARE
                && (h as f32) >= height as f32 * MIN_HEIGHT_SHARE
                && w > 2 * INSET + 8
                && h > 2 * INSET + 8;
            // With nothing to tell it from the artwork (a black band under a dark
            // title), the lowest large plain panel of a game that has a dot
            // matrix is the best there is
            let low_band = lenient
                && y + h >= height * 6 / 10
                && w * 100 >= width * 40
                && area * 100 >= width * height * 8;
            if big_enough && (low_band || stands_out(&rgba, (x, y, w, h), min_contrast)) {
                if best.is_none_or(|(a, _)| area > a) {
                    let (gx, gy, gw, gh) =
                        grow_to_frame(&luma, width, height, (x, y, w, h), shape.aspect());
                    best = Some((
                        area,
                        fit_aspect(
                            gx + INSET,
                            gy + INSET,
                            gw - 2 * INSET,
                            gh - 2 * INSET,
                            shape.aspect(),
                        ),
                    ));
                }
                break;
            }
            for row in y..y + h {
                flat[row * width + x..row * width + x + w].fill(false);
            }
        }
    }
    best
}

/// Look at the text of a `.directb2s` of a game with the given `screen`.
///
/// The screen decides what is looked for: nothing for a game with segment
/// displays only, a hole of the screen's own shape for a dot matrix or a video
/// screen (a rotated CRT is nearly square), either shape for an unknown ROM.
pub fn analyze(doc: &[u8], screen: Screen) -> Result<Analysis> {
    if find(doc, b"<Displays", 0).is_some() {
        return Ok(Analysis::HasDisplays);
    }
    if screen == Screen::Segments {
        return Ok(Analysis::NoScreen);
    }
    let Some(bytes) = dmd_image_bytes(doc)? else {
        // A game with a screen needs a frame to show it in: without a
        // `<DMDImage>` the ScoreView window stays empty
        return Ok(match screen {
            Screen::Dmd { .. } => {
                Analysis::NoFrame(frame_rect(screen.aspect().unwrap_or(DMD_ASPECT)), false)
            }
            Screen::Video { .. } => {
                Analysis::NoFrame(frame_rect(screen.aspect().unwrap_or(VIDEO_ASPECT)), true)
            }
            _ => Analysis::NoDmdImage,
        });
    };
    let image = image::load_from_memory(&bytes).context("DMDImage cannot be decoded")?;
    let dmd_shape = match screen {
        Screen::Dmd { .. } => Some(Shape::Dmd(screen.aspect().unwrap_or(DMD_ASPECT))),
        Screen::Unknown => Some(Shape::Dmd(DMD_ASPECT)),
        _ => None,
    };
    let video_shape = match screen {
        Screen::Video { .. } => Some(Shape::Video(screen.aspect().unwrap_or(VIDEO_ASPECT))),
        _ => None,
    };
    let (width, height) = (image.width() as usize, image.height() as usize);
    // A game known to have a screen has it in the frame, so a weak outline will
    // do; for a game the list does not know, the panel has to stand out
    let min_contrast = if matches!(screen, Screen::Unknown) {
        20.0
    } else {
        6.0
    };
    let lenient = matches!(screen, Screen::Dmd { .. });
    // A dot matrix first: a game that is not known to have a video screen has a
    // dot matrix, whatever the frame's biggest dark square may be. The dark
    // hole and the light panel compete, the larger wins (a small dark patch
    // of artwork must not hide the real panel)
    if let Some(shape) = dmd_shape {
        let dark = find_hole_area(&image, shape);
        let big = dark.is_some_and(|(a, _)| a * 100 >= width * height * 12);
        let flat = if big {
            None
        } else {
            find_flat_panel(&image, shape, min_contrast, lenient)
        };
        let best = match (dark, flat) {
            (Some((a, d)), Some((b, f))) => Some(if b > a { f } else { d }),
            (Some((_, d)), None) => Some(d),
            (None, Some((_, f))) => Some(f),
            (None, None) => None,
        };
        if let Some(rect) = best {
            return Ok(Analysis::Missing(rect));
        }
    }
    if let Some(shape) = video_shape {
        let dark =
            find_hole_area(&image, shape).or_else(|| find_dark_video_area(&image, shape.aspect()));
        if let Some((_, rect)) = dark {
            return Ok(Analysis::MissingVideo(rect));
        }
        if let Some((_, rect)) = find_flat_panel(&image, shape, min_contrast, lenient) {
            return Ok(Analysis::MissingVideo(rect));
        }
    }
    Ok(Analysis::NoHole)
}

/// Size of the frame image written for a file that has none (the usual size of
/// a FullDMD frame).
const FRAME_SIZE: (u32, u32) = (1920, 1080);
/// Marks the `<DMDImage>` this module wrote, so that it can be told from an
/// authored one.
const FRAME_MARK: &str = "Generated=\"PinReady\"";

/// Where the display goes in the generated frame: the shape `aspect`, centred,
/// with a margin of 5% around it.
fn frame_rect(aspect: f32) -> DmdRect {
    let (fw, fh) = (FRAME_SIZE.0 as f32, FRAME_SIZE.1 as f32);
    let mut w = fw * 0.9;
    let mut h = w / aspect;
    if h > fh * 0.9 {
        h = fh * 0.9;
        w = h * aspect;
    }
    DmdRect {
        x: ((fw - w) / 2.0) as u32,
        y: ((fh - h) / 2.0) as u32,
        w: w as u32,
        h: h as u32,
    }
}

/// The generated frame: near black, with a dark grey bezel around the display.
fn frame_png(rect: DmdRect) -> Result<Vec<u8>> {
    const BEZEL: u32 = 14;
    let img = image::RgbaImage::from_fn(FRAME_SIZE.0, FRAME_SIZE.1, |x, y| {
        let inside = x + BEZEL >= rect.x
            && x < rect.x + rect.w + BEZEL
            && y + BEZEL >= rect.y
            && y < rect.y + rect.h + BEZEL;
        let hole = x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h;
        if hole {
            image::Rgba([0, 0, 0, 255])
        } else if inside {
            image::Rgba([46, 46, 50, 255])
        } else {
            image::Rgba([8, 8, 10, 255])
        }
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .context("cannot encode the frame image")?;
    Ok(out.into_inner())
}

/// `doc` with a `<DMDImage>` frame added at the start of its `<Images>`
/// element (the element is created when the file has none).
fn insert_frame(doc: &[u8], png: &[u8]) -> Result<Vec<u8>> {
    let image = format!(
        "<DMDImage {FRAME_MARK} Value=\"{}\" />",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    if let Some(at) = find(doc, b"<Images>", 0) {
        let at = at + b"<Images>".len();
        let mut out = Vec::with_capacity(doc.len() + image.len());
        out.extend_from_slice(&doc[..at]);
        out.extend_from_slice(image.as_bytes());
        out.extend_from_slice(&doc[at..]);
        Ok(out)
    } else {
        insert_element(doc, &format!("<Images>{image}</Images>"))
    }
}

/// `doc` with the element before the closing tag of the document.
fn insert_element(doc: &[u8], element: &str) -> Result<Vec<u8>> {
    const CLOSE: &[u8] = b"</DirectB2SData>";
    let at = doc
        .windows(CLOSE.len())
        .rposition(|w| w == CLOSE)
        .context("no </DirectB2SData> in the file")?;
    let mut out = Vec::with_capacity(doc.len() + element.len());
    out.extend_from_slice(&doc[..at]);
    out.extend_from_slice(element.as_bytes());
    out.extend_from_slice(&doc[at..]);
    Ok(out)
}

/// Where the original of `file` is kept: one folder per file path, so two
/// tables with the same file name never share a backup.
pub fn backup_path(backup_root: &Path, file: &Path) -> PathBuf {
    let digest = Sha256::digest(file.to_string_lossy().as_bytes());
    let folder: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "backglass.directb2s".into());
    backup_root.join(folder).join(format!("{name}.orig"))
}

/// The original of `doc`, when `doc` is that original plus the one `<Displays>`
/// element this module wrote and nothing else: the file is still ours to
/// redo (a better search, a game recognised as having no DMD). A file that
/// anyone else touched since is not.
fn own_original(doc: &[u8], backup: &Path) -> Option<Vec<u8>> {
    let orig = std::fs::read(backup).ok().filter(|o| !o.is_empty())?;
    let at = doc
        .windows(b"<Displays>".len())
        .rposition(|w| w == b"<Displays>")?;
    let end = find(doc, b"</Displays>", at)? + b"</Displays>".len();
    let mut stripped = Vec::with_capacity(doc.len());
    stripped.extend_from_slice(&doc[..at]);
    stripped.extend_from_slice(&doc[end..]);
    // the frame written for a file that had none goes too
    let mark = format!("<DMDImage {FRAME_MARK} Value=\"");
    if let Some(start) = find(&stripped, mark.as_bytes(), 0)
        && let Some(close) = find(&stripped, b"/>", start)
    {
        let mut without = Vec::with_capacity(stripped.len());
        without.extend_from_slice(&stripped[..start]);
        without.extend_from_slice(&stripped[close + 2..]);
        // the `<Images>` element created for it, when it was the whole of it
        let created = b"<Images></Images>";
        if let Some(i) = find(&without, created, 0) {
            without.drain(i..i + created.len());
        }
        stripped = without;
    }
    (stripped == orig).then_some(orig)
}

/// The rectangle of the frame to generate for `doc`, when it has no `<DMDImage>`
/// and the game has a screen.
fn analysis_frame(doc: &[u8], screen: Screen) -> Result<Option<DmdRect>> {
    if find(doc, b"<DMDImage", 0).is_some() {
        return Ok(None);
    }
    Ok(match analyze(doc, screen)? {
        Analysis::NoFrame(rect, _) => Some(rect),
        _ => None,
    })
}

/// Replace `file` with `bytes` in one rename, keeping its permissions.
fn replace_file(file: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp_name = file.as_os_str().to_owned();
    tmp_name.push(".pinready-tmp");
    let tmp = PathBuf::from(tmp_name);
    std::fs::write(&tmp, bytes).with_context(|| format!("cannot write {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(file) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = std::fs::rename(&tmp, file) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("cannot replace {}", file.display()));
    }
    Ok(())
}

/// Check `file` and add the DMD element when it is missing and the frame image
/// shows where it goes, for a game with the given `screen`. The original is
/// copied to `backup_root` first (never overwritten once it exists), and the
/// new file replaces it in one rename. A file this module fixed earlier is
/// checked again from its original: the element is replaced when the search
/// now finds it elsewhere, and taken out when the game turns out to have no
/// dot matrix. Returns the status to record.
pub fn fix_file(file: &Path, backup_root: &Path, screen: Screen) -> Result<&'static str> {
    let doc = std::fs::read(file).with_context(|| format!("cannot read {}", file.display()))?;
    let backup = backup_path(backup_root, file);
    let earlier = own_original(&doc, &backup);
    let base: &[u8] = earlier.as_deref().unwrap_or(&doc);
    let (rect, element) = match analyze(base, screen)? {
        Analysis::HasDisplays | Analysis::NoDmdImage => return Ok(status::NOT_NEEDED),
        absent @ (Analysis::NoScreen | Analysis::NoHole) => {
            if let Some(orig) = &earlier {
                replace_file(file, orig)?;
                log::info!(
                    "b2s_dmd: {} is back to its original: no DMD area for this game",
                    file.display()
                );
            }
            return Ok(if absent == Analysis::NoScreen {
                status::NOT_NEEDED
            } else {
                status::NO_HOLE
            });
        }
        Analysis::Missing(rect) => (rect, rect.element()),
        Analysis::MissingVideo(rect) => (rect, rect.video_element()),
        Analysis::NoFrame(rect, video) => {
            let element = if video {
                rect.video_element()
            } else {
                rect.element()
            };
            (rect, element)
        }
    };
    let framed = match analysis_frame(base, screen)? {
        Some(rect) => insert_frame(base, &frame_png(rect)?)?,
        None => base.to_vec(),
    };
    let fixed = insert_element(&framed, &element)?;
    if earlier.is_some() && fixed == doc {
        return Ok(status::APPLIED); // the same place as before
    }

    if !backup.exists() {
        if let Some(dir) = backup.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        std::fs::write(&backup, &doc)
            .with_context(|| format!("cannot write the backup {}", backup.display()))?;
    }
    // The backup has to be what was read, not an earlier copy of another version
    if std::fs::read(&backup).map(|b| b.len()).unwrap_or(0) == 0 {
        bail!("the backup {} is empty", backup.display());
    }

    replace_file(file, &fixed)?;
    log::info!(
        "b2s_dmd: {} got its DMD area X={} Y={} W={} H={} (original kept in {})",
        file.display(),
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        backup.display()
    );
    Ok(status::APPLIED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    /// Mr. Game's rotated CRT: 256x248.
    const MR_GAME: Screen = Screen::Video {
        width: 256,
        height: 248,
    };

    /// A frame image: grey artwork with a black 4:1 hole at (x, y, w, h).
    fn frame(width: u32, height: u32, hole: (u32, u32, u32, u32)) -> image::DynamicImage {
        let img = ImageBuffer::from_fn(width, height, |px, py| {
            let (x, y, w, h) = hole;
            if px >= x && px < x + w && py >= y && py < y + h {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([120, 120, 130, 255])
            }
        });
        image::DynamicImage::ImageRgba8(img)
    }

    fn png(img: &image::DynamicImage) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A document the way the writers of .directb2s produce it: the base64
    /// is cut in lines escaped as character references.
    fn document(img: Option<&image::DynamicImage>, displays: bool) -> Vec<u8> {
        let mut doc = String::from("<DirectB2SData Version=\"1.3\">\r\n  <Images>\r\n");
        if let Some(img) = img {
            let b64 = base64::engine::general_purpose::STANDARD.encode(png(img));
            let mut wrapped = String::new();
            for (i, c) in b64.chars().enumerate() {
                if i > 0 && i % 76 == 0 {
                    wrapped.push_str("&#xD;&#xA;");
                }
                wrapped.push(c);
            }
            doc.push_str(&format!("    <DMDImage Value=\"{wrapped}\" />\r\n"));
        }
        doc.push_str("  </Images>\r\n");
        if displays {
            doc.push_str(
                "<Displays><DMD Target=\"Backglass\" X=\"1\" Y=\"2\" W=\"3\" H=\"4\" /></Displays>",
            );
        }
        doc.push_str("</DirectB2SData>");
        doc.into_bytes()
    }

    #[test]
    fn finds_the_hole_of_a_frame() {
        let img = frame(1012, 309, (35, 37, 938, 234));
        let rect = find_hole(&img, Shape::Dmd(DMD_ASPECT)).expect("hole");
        // fitted to 4:1 inside the hole, which is 938x234 (4.008)
        assert!((rect.w as i64 - 936).abs() <= 2, "{rect:?}");
        assert!((rect.h as i64 - 234).abs() <= 1, "{rect:?}");
        assert!(
            (rect.x as i64 - 36).abs() <= 2 && (rect.y as i64 - 37).abs() <= 1,
            "{rect:?}"
        );
    }

    #[test]
    fn a_hole_that_is_not_pure_black_is_still_found() {
        // A hole with a gradient (luminance 18..40) inside a bright border,
        // under a band of dark artwork that touches the top of the image
        let img = ImageBuffer::from_fn(1280, 720, |x, y| {
            if (115..1165).contains(&x) && (450..710).contains(&y) {
                let v = 18 + ((x - 115) * 22 / 1050) as u8;
                Rgba([v, v, v, 255])
            } else if y < 400 {
                Rgba([12, 14, 24, 255]) // dark sky, no border
            } else {
                Rgba([150, 110, 40, 255]) // gold frame
            }
        });
        let rect = find_hole(
            &image::DynamicImage::ImageRgba8(img),
            Shape::Dmd(DMD_ASPECT),
        )
        .expect("hole");
        // inside the hole, and most of it
        assert!(rect.x >= 115 && rect.x + rect.w <= 1165, "{rect:?}");
        assert!(rect.y >= 450 && rect.y + rect.h <= 710, "{rect:?}");
        assert!(rect.w >= 1000, "{rect:?}");
    }

    /// A frame for a video screen: a nearly square hole flush with the bottom of the image.
    #[test]
    fn a_nearly_square_hole_is_a_video_screen() {
        // Motor Show's frame: 2190x698, hole about 750x660 from the bottom edge up
        let img = frame(2190, 698, (719, 38, 750, 660));
        let image = image::DynamicImage::ImageRgba8(img.to_rgba8());
        let doc = document(Some(&image), false);
        assert!(matches!(
            analyze(&doc, MR_GAME).unwrap(),
            Analysis::MissingVideo(_)
        ));
        let rect = find_hole(&image, Shape::Video(VIDEO_ASPECT)).expect("video hole");
        assert!(
            (rect.w as i64 - rect.h as i64).abs() <= 1,
            "square: {rect:?}"
        );
        assert!(rect.x >= 719 && rect.x + rect.w <= 719 + 750);
        assert!(rect.y >= 38 && rect.y + rect.h <= 698);
    }

    #[test]
    fn a_video_frame_is_written_with_the_screen_as_source() {
        let img = frame(2190, 698, (719, 38, 750, 660));
        let doc = document(Some(&img), false);
        let Analysis::MissingVideo(rect) = analyze(&doc, MR_GAME).unwrap() else {
            panic!("expected a video hole");
        };
        let fixed = insert_element(&doc, &rect.video_element()).unwrap();
        let text = String::from_utf8(fixed).unwrap();
        assert!(text.contains("Source=\"ctrl://default/display?dmd_only=0\""));
        assert!(text.contains("Target=\"ScoreView\""));
        assert_eq!(text.matches("<Displays>").count(), 1);
    }

    /// A video frame in dark artwork: no border stands out, only the shape and the place tell.
    fn dark_frame(hole: (u32, u32, u32, u32)) -> image::DynamicImage {
        let img = ImageBuffer::from_fn(2190, 698, |px, py| {
            let (x, y, w, h) = hole;
            if px >= x && px < x + w && py >= y && py < y + h {
                Rgba([6, 6, 6, 255])
            } else {
                Rgba([30, 30, 30, 255])
            }
        });
        image::DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn a_centred_video_hole_in_dark_artwork_is_found() {
        let doc = document(Some(&dark_frame((734, 63, 731, 635))), false);
        let Analysis::MissingVideo(rect) = analyze(&doc, MR_GAME).unwrap() else {
            panic!("expected a video hole");
        };
        // the shape of Mr. Game's screen, 256x248
        assert!(
            (rect.w as f32 / rect.h as f32 - 256.0 / 248.0).abs() < 0.02,
            "shape: {rect:?}"
        );
    }

    #[test]
    fn a_dark_square_off_centre_or_small_is_not_a_video_screen() {
        let off_centre = document(Some(&dark_frame((100, 63, 731, 635))), false);
        assert_eq!(
            analyze(&off_centre, Screen::Unknown).unwrap(),
            Analysis::NoHole
        );
        let small = document(Some(&dark_frame((1000, 200, 200, 200))), false);
        assert_eq!(analyze(&small, Screen::Unknown).unwrap(), Analysis::NoHole);
    }

    #[test]
    fn a_dmd_hole_wins_over_a_video_one() {
        let img = frame(2000, 600, (400, 250, 1200, 300));
        let doc = document(Some(&img), false);
        assert!(matches!(
            analyze(&doc, Screen::Unknown).unwrap(),
            Analysis::Missing(_)
        ));
    }

    #[test]
    fn a_plain_image_has_no_hole() {
        let img = image::DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            800,
            300,
            Rgba([90, 90, 90, 255]),
        ));
        assert!(find_hole(&img, Shape::Dmd(DMD_ASPECT)).is_none());
    }

    #[test]
    fn a_small_dark_spot_is_not_a_dmd() {
        let img = frame(1000, 600, (10, 10, 120, 30));
        assert!(find_hole(&img, Shape::Dmd(DMD_ASPECT)).is_none());
    }

    #[test]
    fn a_see_through_hole_counts() {
        let img = ImageBuffer::from_fn(800, 300, |x, y| {
            if (40..760).contains(&x) && (50..230).contains(&y) {
                Rgba([200, 200, 200, 0])
            } else {
                Rgba([100, 100, 100, 255])
            }
        });
        let rect = find_hole(
            &image::DynamicImage::ImageRgba8(img),
            Shape::Dmd(DMD_ASPECT),
        )
        .expect("hole");
        assert!(rect.w >= 700 && rect.h >= 170);
    }

    #[test]
    fn analyze_reads_the_escaped_base64() {
        let img = frame(1012, 309, (35, 37, 938, 234));
        match analyze(&document(Some(&img), false), Screen::Unknown).unwrap() {
            Analysis::Missing(r) => assert!(r.w > 900),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_existing_displays_element_is_left_alone() {
        let img = frame(1012, 309, (35, 37, 938, 234));
        assert_eq!(
            analyze(&document(Some(&img), true), Screen::Unknown).unwrap(),
            Analysis::HasDisplays
        );
    }

    #[test]
    fn no_frame_image_no_change() {
        assert_eq!(
            analyze(&document(None, false), Screen::Unknown).unwrap(),
            Analysis::NoDmdImage
        );
    }

    #[test]
    fn the_element_goes_before_the_closing_tag_and_nothing_else_changes() {
        let img = frame(1012, 309, (35, 37, 938, 234));
        let doc = document(Some(&img), false);
        let rect = DmdRect {
            x: 35,
            y: 37,
            w: 938,
            h: 234,
        };
        let fixed = insert_element(&doc, &rect.element()).unwrap();
        let element = rect.element().into_bytes();
        let at = fixed.len() - b"</DirectB2SData>".len() - element.len();
        assert_eq!(&fixed[at..at + element.len()], &element[..]);
        let mut back = fixed.clone();
        back.drain(at..at + element.len());
        assert_eq!(back, doc);
    }

    #[test]
    fn fix_file_keeps_the_original_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("Table (Maker 1990).directb2s");
        let img = frame(1012, 309, (35, 37, 938, 234));
        let original = document(Some(&img), false);
        std::fs::write(&file, &original).unwrap();

        assert_eq!(
            fix_file(&file, backups.path(), Screen::Unknown).unwrap(),
            status::APPLIED
        );
        let fixed = std::fs::read(&file).unwrap();
        assert!(find(&fixed, b"<Displays><DMD Target=\"ScoreView\"", 0).is_some());
        assert_eq!(
            std::fs::read(backup_path(backups.path(), &file)).unwrap(),
            original
        );
        assert!(
            !dir.path()
                .join("Table (Maker 1990).directb2s.pinready-tmp")
                .exists()
        );

        // a second pass finds its own element in the same place and does nothing
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Unknown).unwrap(),
            status::APPLIED
        );
        assert_eq!(std::fs::read(&file).unwrap(), fixed);
        assert_eq!(
            std::fs::read(backup_path(backups.path(), &file)).unwrap(),
            original
        );
    }

    #[test]
    fn a_file_with_no_hole_is_not_touched() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.directb2s");
        let img = image::DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            600,
            200,
            Rgba([90, 90, 90, 255]),
        ));
        let original = document(Some(&img), false);
        std::fs::write(&file, &original).unwrap();
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Unknown).unwrap(),
            status::NO_HOLE
        );
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert!(!backup_path(backups.path(), &file).exists());
    }

    #[test]
    fn backups_of_same_named_files_do_not_collide() {
        let root = Path::new("/b");
        let a = backup_path(root, Path::new("/t/one/x.directb2s"));
        let b = backup_path(root, Path::new("/t/two/x.directb2s"));
        assert_ne!(a, b);
        assert_eq!(a.file_name(), b.file_name());
    }

    /// Port of the VPX B2S plugin's `Renderer::SearchDmdSubFrame` (plugins/b2s/DMDOverlay),
    /// to see what its own detection finds in a frame image. Returns (x, y, w, h).
    fn plugin_search(rgba: &image::RgbaImage, ar: f32) -> (i32, i32, i32, i32) {
        let (tw, th) = (rgba.width() as i32, rgba.height() as i32);
        let lum: Vec<f32> = rgba
            .pixels()
            .map(|p| 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]))
            .collect();
        let heuristic = |f: (i32, i32, i32, i32)| {
            f.2 as f32 / tw as f32 - (0.5 - (f.0 + f.2 / 2) as f32 / tw as f32).abs()
        };
        let padding = (2 * tw) / 1920;
        let (mut lum_min, mut lum_max) = (0f32, 256f32);
        let mut search = (0, 0, tw, th);
        let mut best = (0, 0, 0, 0);
        for _ in 0..7 {
            let limit = (lum_min + lum_max) * 0.5;
            let mut sub = (0, 0, 0, 0);
            let mut search_sub = (0, 0, 0, 0);
            let mut st: Vec<i32> = Vec::new();
            let mut heights = vec![0i32; tw as usize];
            for y in search.1..search.1 + search.3 {
                for x in search.0..search.0 + search.2 {
                    if lum[(y * tw + x) as usize] < limit {
                        heights[x as usize] += 1;
                    } else {
                        heights[x as usize] = 0;
                    }
                }
                for x in search.0..=search.0 + search.2 {
                    while let Some(&top) = st.last() {
                        if !(x == search.0 + search.2
                            || heights[top as usize] > heights[x as usize])
                        {
                            break;
                        }
                        let height = heights[top as usize];
                        st.pop();
                        let width = match st.last() {
                            None => x - search.0,
                            Some(&t) => x - t - 1,
                        };
                        if width > 6 * padding && height > 3 * padding {
                            let ux = st.last().map_or(search.0, |&t| t + 1);
                            let unpadded = (ux, y - height + 1, width, height);
                            if search_sub.2 == 0 {
                                search_sub = unpadded;
                            } else {
                                let nx = search_sub.0.min(unpadded.0);
                                let ny = search_sub.1.min(unpadded.1);
                                let nz =
                                    (search_sub.0 + search_sub.2).max(unpadded.0 + unpadded.2) - nx;
                                let nw =
                                    (search_sub.1 + search_sub.3).max(unpadded.1 + unpadded.3) - ny;
                                search_sub = (nx, ny, nz, nw);
                            }
                            let mut f = (
                                unpadded.0 + padding,
                                unpadded.1 + padding,
                                unpadded.2 - 2 * padding,
                                unpadded.3 - 2 * padding,
                            );
                            if f.3 > 0 && f.2 as f32 / f.3 as f32 > ar {
                                let w = (f.3 as f32 * ar) as i32;
                                f.0 += (f.2 - w) / 2;
                                f.2 = w;
                            } else if f.3 > 0 {
                                let h = (f.2 as f32 / ar) as i32;
                                f.1 += (f.3 - h) / 2;
                                f.3 = h;
                            }
                            if 3 * f.2 >= tw && heuristic(f) > heuristic(sub) {
                                sub = f;
                            }
                        }
                    }
                    if x < search.0 + search.2 {
                        st.push(x);
                    }
                }
            }
            if sub.3 > 0 {
                lum_max = limit;
                best = sub;
                search = search_sub;
            } else {
                lum_min = limit;
            }
        }
        best
    }

    /// `B2S_DMD_FILES=a:b cargo test --profile=fast plugin_detect -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn plugin_detect() {
        for file in std::env::var("B2S_DMD_FILES")
            .unwrap_or_default()
            .split(':')
            .filter(|f| !f.is_empty())
        {
            let doc = std::fs::read(file).unwrap();
            let Some(bytes) = dmd_image_bytes(&doc).unwrap() else {
                println!("{file}: NOIMAGE");
                continue;
            };
            let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
            println!(
                "{file}: PLUGIN {:?} img={}x{}",
                plugin_search(&img, 4.0),
                img.width(),
                img.height()
            );
        }
    }

    /// Development aid: `B2S_DMD_FILES=a.directb2s:b.directb2s cargo test --profile=fast real_files -- --ignored --nocapture`
    /// prints what the check finds in real files.
    #[test]
    #[ignore]
    fn real_files() {
        for file in std::env::var("B2S_DMD_FILES")
            .unwrap_or_default()
            .split(':')
            .filter(|f| !f.is_empty())
        {
            let doc = std::fs::read(file).unwrap();
            println!("{file}: {:?}", analyze(&doc, Screen::Unknown));
        }
    }

    /// A frame image: dark artwork with a flat panel of `colour` at (x, y, w, h),
    /// and some black specks on the panel.
    fn light_panel(colour: [u8; 3], noisy: bool) -> image::DynamicImage {
        let img = ImageBuffer::from_fn(1200, 700, |px, py| {
            let (x, y, w, h) = (30u32, 400u32, 1100u32, 260u32);
            if px >= x && px < x + w && py >= y && py < y + h {
                if noisy && (px * 7 + py * 13) % 61 == 0 {
                    Rgba([10, 10, 10, 255])
                } else {
                    Rgba([colour[0], colour[1], colour[2], 255])
                }
            } else {
                // textured artwork: never flat
                let v = ((px * 131 + py * 71 + px * py) % 97) as u8 * 2 + 20;
                Rgba([v, 60, 255 - v, 255])
            }
        });
        image::DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn a_light_panel_is_found_when_there_is_no_dark_hole() {
        let doc = document(Some(&light_panel([225, 235, 233], false)), false);
        let Analysis::Missing(rect) = analyze(&doc, Screen::Unknown).unwrap() else {
            panic!("no DMD area");
        };
        // inside the panel, 4:1
        assert!(rect.x >= 30 && rect.x + rect.w <= 1130, "{rect:?}");
        assert!(rect.y >= 400 && rect.y + rect.h <= 660, "{rect:?}");
        assert!(
            ((rect.w as f32 / rect.h as f32) - 4.0).abs() < 0.2,
            "{rect:?}"
        );
    }

    #[test]
    fn a_few_dark_specks_do_not_break_the_panel() {
        let doc = document(Some(&light_panel([225, 235, 233], true)), false);
        assert!(matches!(
            analyze(&doc, Screen::Unknown).unwrap(),
            Analysis::Missing(_)
        ));
    }

    #[test]
    fn plain_artwork_has_no_light_panel() {
        let img = ImageBuffer::from_fn(1200, 700, |px, py| {
            let v = ((px * 131 + py * 71 + px * py) % 97) as u8 * 2 + 20;
            Rgba([v, 60, 255 - v, 255])
        });
        let doc = document(Some(&image::DynamicImage::ImageRgba8(img)), false);
        assert_eq!(analyze(&doc, Screen::Unknown).unwrap(), Analysis::NoHole);
    }

    #[test]
    fn the_year_of_a_table_folder_name() {
        assert_eq!(year_of("Expressway (Bally 1971)"), Some(1971));
        assert_eq!(year_of("The Crow (Original 2025)"), Some(2025));
        assert_eq!(year_of("No year here"), None);
        assert_eq!(year_of("Odd (Maker) (Original 25)"), None);
    }

    #[test]
    fn a_game_with_segment_displays_only_is_left_alone() {
        let doc = document(Some(&frame(1000, 600, (100, 400, 800, 200))), false);
        assert_eq!(analyze(&doc, Screen::Segments).unwrap(), Analysis::NoScreen);
    }

    #[test]
    fn the_hole_has_the_shape_of_the_screen_of_the_game() {
        // a 8:1 hole (128x16) in the middle of a 4:1 one's reach
        let doc = document(Some(&frame(1200, 600, (100, 400, 1000, 125))), false);
        let Analysis::Missing(rect) = analyze(
            &doc,
            Screen::Dmd {
                width: 128,
                height: 16,
            },
        )
        .unwrap() else {
            panic!("no DMD area");
        };
        assert!(
            ((rect.w as f32 / rect.h as f32) - 8.0).abs() < 0.2,
            "{rect:?}"
        );
    }

    #[test]
    fn a_fix_of_an_earlier_version_is_redone_and_undone() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("t.directb2s");
        let original = document(Some(&frame(1000, 600, (100, 400, 800, 200))), false);
        std::fs::write(&file, &original).unwrap();
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Unknown).unwrap(),
            status::APPLIED
        );
        let first = std::fs::read(&file).unwrap();
        // the same game again: unchanged
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Unknown).unwrap(),
            status::APPLIED
        );
        assert_eq!(std::fs::read(&file).unwrap(), first);
        // the game turns out to have no dot matrix: back to the original
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Segments).unwrap(),
            status::NOT_NEEDED
        );
        assert_eq!(std::fs::read(&file).unwrap(), original);
        // an element somebody else wrote is never touched
        // (written by the in-game editor, which rewrites the whole file)
        let mut authored = String::from_utf8(original.clone())
            .unwrap()
            .replace("Version=\"1.3\"", "Version=\"1.4\"")
            .into_bytes();
        authored = insert_element(
            &authored,
            "<Displays><DMD Target=\"ScoreView\" X=\"1\" Y=\"2\" W=\"3\" H=\"4\" /></Displays>",
        )
        .unwrap();
        std::fs::write(&file, &authored).unwrap();
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Segments).unwrap(),
            status::NOT_NEEDED
        );
        assert_eq!(std::fs::read(&file).unwrap(), authored);
    }

    /// Development aid: what the fix would do to every table of a folder, with the
    /// screen of its ROM. `B2S_DMD_TABLES=/path/to/Tables cargo test --profile=fast
    /// sweep -- --ignored --nocapture`; nothing is written.
    #[test]
    #[ignore]
    fn sweep() {
        let root = std::env::var("B2S_DMD_TABLES").unwrap_or_default();
        let mut tables: Vec<PathBuf> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter_map(|d| {
                let dir = d.path();
                let name = dir.file_name()?.to_string_lossy().into_owned();
                let vpx = dir.join(format!("{name}.vpx"));
                dir.join(format!("{name}.directb2s"))
                    .exists()
                    .then_some(vpx)
            })
            .collect();
        tables.sort();
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let Some(vpx) = tables.get(i) else { break };
                        let screen = screen_for_table(vpx);
                        let doc = std::fs::read(vpx.with_extension("directb2s")).unwrap();
                        let result = match analyze(&doc, screen) {
                            Ok(Analysis::Missing(r)) => {
                                format!("DMD {} {} {} {}", r.x, r.y, r.w, r.h)
                            }
                            Ok(Analysis::MissingVideo(r)) => {
                                format!("VIDEO {} {} {} {}", r.x, r.y, r.w, r.h)
                            }
                            Ok(other) => format!("{other:?}"),
                            Err(e) => format!("ERROR {e}"),
                        };
                        let name = vpx.file_stem().unwrap().to_string_lossy();
                        println!("SWEEP|{name}|{screen:?}|{result}");
                    }
                });
            }
        });
    }

    #[test]
    fn a_file_with_no_frame_gets_one_for_a_game_with_a_dmd() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("t.directb2s");
        let original = document(None, false);
        std::fs::write(&file, &original).unwrap();
        let dmd = Screen::Dmd {
            width: 128,
            height: 32,
        };
        // no DMD in the game: nothing to add
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Unknown).unwrap(),
            status::NOT_NEEDED
        );
        assert_eq!(std::fs::read(&file).unwrap(), original);
        // a DMD game: a frame and the display in it
        assert_eq!(
            fix_file(&file, backups.path(), dmd).unwrap(),
            status::APPLIED
        );
        let fixed = std::fs::read(&file).unwrap();
        let text = String::from_utf8_lossy(&fixed);
        assert!(text.contains("<DMDImage Generated=\"PinReady\""));
        assert_eq!(text.matches("<Displays>").count(), 1);
        let image = dmd_image_bytes(&fixed).unwrap().expect("a frame");
        let img = image::load_from_memory(&image).unwrap();
        assert_eq!((img.width(), img.height()), FRAME_SIZE);
        // the display is a 4:1 rectangle inside the frame
        let rect = frame_rect(4.0);
        assert!(rect.x + rect.w <= FRAME_SIZE.0 && rect.y + rect.h <= FRAME_SIZE.1);
        assert!(text.contains(&format!("W=\"{}\" H=\"{}\"", rect.w, rect.h)));
        // again: the same file
        assert_eq!(
            fix_file(&file, backups.path(), dmd).unwrap(),
            status::APPLIED
        );
        assert_eq!(std::fs::read(&file).unwrap(), fixed);
        // the game has no DMD after all: the original, frame and display gone
        assert_eq!(
            fix_file(&file, backups.path(), Screen::Segments).unwrap(),
            status::NOT_NEEDED
        );
        assert_eq!(std::fs::read(&file).unwrap(), original);
    }
}
