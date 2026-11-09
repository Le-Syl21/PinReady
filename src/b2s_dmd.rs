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
use std::path::{Path, PathBuf};

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
    /// The frame image has no dark hole of the right shape.
    NoHole,
    /// The element is missing and this is where the DMD goes.
    Missing(DmdRect),
    /// The element is missing and the hole is for a video (CRT/LCD) screen.
    MissingVideo(DmdRect),
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

/// What a hole is looked for as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// The DMD's 4:1.
    Dmd,
    /// A nearly square video screen.
    Video,
}

impl Shape {
    /// The width kept of a dark `w` x `h` rectangle, `None` when its shape is wrong.
    fn kept_width(self, w: usize, h: usize) -> Option<usize> {
        let ar = w as f32 / h as f32;
        match self {
            Shape::Dmd => (ar >= DMD_ASPECT * (1.0 - ASPECT_TOLERANCE))
                .then(|| w.min((h as f32 * DMD_ASPECT * (1.0 + ASPECT_TOLERANCE)) as usize)),
            Shape::Video => (VIDEO_ASPECT_MIN..=VIDEO_ASPECT_MAX)
                .contains(&ar)
                .then_some(w),
        }
    }

    fn aspect(self) -> f32 {
        match self {
            Shape::Dmd => DMD_ASPECT,
            Shape::Video => VIDEO_ASPECT,
        }
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
fn find_dark_video_area(image: &image::DynamicImage) -> Option<(usize, DmdRect)> {
    find_hole_with(image, Shape::Video, true)
}

/// `relaxed` drops the contrast test for a dark rectangle that is centred
/// (within 8% of the image width) and covers at least a fifth of the image.
fn find_hole_with(
    image: &image::DynamicImage,
    shape: Shape,
    relaxed: bool,
) -> Option<(usize, DmdRect)> {
    const THRESHOLDS: [u8; 5] = [DARK_THRESHOLD, 24, 32, 44, 60];
    /// A border at least this much brighter than the inside, in luminance levels.
    const MIN_CONTRAST: f32 = 25.0;
    let (luma, width, height) = luminance(image);
    if width < 64 || height < 24 {
        return None;
    }
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
            let strict = shape == Shape::Dmd;
            let accepted = if relaxed {
                let centre = (x * 2 + w) as f32 / 2.0;
                big_enough
                    && (centre - width as f32 / 2.0).abs() <= width as f32 * 0.08
                    && (w * h) as f32 >= (width * height) as f32 * 0.2
            } else {
                big_enough
                    && edge_contrast(&luma, width, height, (x, y, w, h), strict) >= MIN_CONTRAST
            };
            if accepted {
                if best.is_none_or(|(a, _)| area > a) {
                    best = Some((area, fit_aspect(x, y, w, h, shape.aspect())));
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

/// Look at the text of a `.directb2s`.
pub fn analyze(doc: &[u8]) -> Result<Analysis> {
    if find(doc, b"<Displays", 0).is_some() {
        return Ok(Analysis::HasDisplays);
    }
    let Some(bytes) = dmd_image_bytes(doc)? else {
        return Ok(Analysis::NoDmdImage);
    };
    let image = image::load_from_memory(&bytes).context("DMDImage cannot be decoded")?;
    // A square hole holds a 4:1 rectangle too, and a 4:1 hole holds a square one:
    // the shape with the larger area is the real one
    let dmd = find_hole_area(&image, Shape::Dmd);
    let video = find_hole_area(&image, Shape::Video);
    Ok(match (dmd, video) {
        (Some((a, rect)), Some((b, _))) if a >= b => Analysis::Missing(rect),
        (Some((_, rect)), None) => Analysis::Missing(rect),
        (_, Some((_, rect))) => Analysis::MissingVideo(rect),
        (None, None) => match find_dark_video_area(&image) {
            Some((_, rect)) => Analysis::MissingVideo(rect),
            None => Analysis::NoHole,
        },
    })
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

/// Check `file` and add the DMD element when it is missing and the frame image
/// shows where it goes. The original is copied to `backup_root` first (never
/// overwritten once it exists), and the new file replaces it in one rename.
/// Returns the status to record.
pub fn fix_file(file: &Path, backup_root: &Path) -> Result<&'static str> {
    let doc = std::fs::read(file).with_context(|| format!("cannot read {}", file.display()))?;
    let (rect, element) = match analyze(&doc)? {
        Analysis::HasDisplays | Analysis::NoDmdImage => return Ok(status::NOT_NEEDED),
        Analysis::NoHole => return Ok(status::NO_HOLE),
        Analysis::Missing(rect) => (rect, rect.element()),
        Analysis::MissingVideo(rect) => (rect, rect.video_element()),
    };
    let fixed = insert_element(&doc, &element)?;

    let backup = backup_path(backup_root, file);
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

    let mut tmp_name = file.as_os_str().to_owned();
    tmp_name.push(".pinready-tmp");
    let tmp = PathBuf::from(tmp_name);
    std::fs::write(&tmp, &fixed).with_context(|| format!("cannot write {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(file) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = std::fs::rename(&tmp, file) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("cannot replace {}", file.display()));
    }
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
        let rect = find_hole(&img, Shape::Dmd).expect("hole");
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
        let rect = find_hole(&image::DynamicImage::ImageRgba8(img), Shape::Dmd).expect("hole");
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
        assert!(matches!(analyze(&doc).unwrap(), Analysis::MissingVideo(_)));
        let rect = find_hole(&image, Shape::Video).expect("video hole");
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
        let Analysis::MissingVideo(rect) = analyze(&doc).unwrap() else {
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
        let Analysis::MissingVideo(rect) = analyze(&doc).unwrap() else {
            panic!("expected a video hole");
        };
        assert!(
            (rect.w as i64 - rect.h as i64).abs() <= 1,
            "square: {rect:?}"
        );
    }

    #[test]
    fn a_dark_square_off_centre_or_small_is_not_a_video_screen() {
        let off_centre = document(Some(&dark_frame((100, 63, 731, 635))), false);
        assert_eq!(analyze(&off_centre).unwrap(), Analysis::NoHole);
        let small = document(Some(&dark_frame((1000, 200, 200, 200))), false);
        assert_eq!(analyze(&small).unwrap(), Analysis::NoHole);
    }

    #[test]
    fn a_dmd_hole_wins_over_a_video_one() {
        let img = frame(2000, 600, (400, 250, 1200, 300));
        let doc = document(Some(&img), false);
        assert!(matches!(analyze(&doc).unwrap(), Analysis::Missing(_)));
    }

    #[test]
    fn a_plain_image_has_no_hole() {
        let img = image::DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            800,
            300,
            Rgba([90, 90, 90, 255]),
        ));
        assert!(find_hole(&img, Shape::Dmd).is_none());
    }

    #[test]
    fn a_small_dark_spot_is_not_a_dmd() {
        let img = frame(1000, 600, (10, 10, 120, 30));
        assert!(find_hole(&img, Shape::Dmd).is_none());
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
        let rect = find_hole(&image::DynamicImage::ImageRgba8(img), Shape::Dmd).expect("hole");
        assert!(rect.w >= 700 && rect.h >= 170);
    }

    #[test]
    fn analyze_reads_the_escaped_base64() {
        let img = frame(1012, 309, (35, 37, 938, 234));
        match analyze(&document(Some(&img), false)).unwrap() {
            Analysis::Missing(r) => assert!(r.w > 900),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_existing_displays_element_is_left_alone() {
        let img = frame(1012, 309, (35, 37, 938, 234));
        assert_eq!(
            analyze(&document(Some(&img), true)).unwrap(),
            Analysis::HasDisplays
        );
    }

    #[test]
    fn no_frame_image_no_change() {
        assert_eq!(
            analyze(&document(None, false)).unwrap(),
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

        assert_eq!(fix_file(&file, backups.path()).unwrap(), status::APPLIED);
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

        // a second pass finds the element and does nothing
        assert_eq!(fix_file(&file, backups.path()).unwrap(), status::NOT_NEEDED);
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
        assert_eq!(fix_file(&file, backups.path()).unwrap(), status::NO_HOLE);
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
            println!("{file}: {:?}", analyze(&doc));
        }
    }
}
