use std::io::BufReader;
use std::path::Path;

use base64::Engine;

// Illumination: all bulbs composited at this opacity
const BULB_OPACITY: f32 = 0.50;

// Brightness normalization: target median luminosity (0–255)
const TARGET_MEDIAN_LUM: f32 = 75.0;

// Max brightness adjustment factor to avoid blowing out dark images
const MAX_BRIGHTNESS_FACTOR: f32 = 2.5;
const MIN_BRIGHTNESS_FACTOR: f32 = 0.5;

/// Final resize target for both B2S and .vpx extraction — keeps the
/// in-memory DB blob small and the launcher rendering consistent.
const DISPLAY_WIDTH: u32 = 1280;
const DISPLAY_HEIGHT: u32 = 1024;

/// Decode base64 image data (with whitespace stripping) to a DynamicImage.
fn decode_b64_image(b64: &str) -> Option<image::DynamicImage> {
    let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .ok()?;
    image::load_from_memory(&bytes).ok()
}

/// Compute the median luminosity of an RGB image using its histogram.
pub(crate) fn median_luminosity(img: &image::RgbImage) -> f32 {
    let gray = image::DynamicImage::ImageRgb8(img.clone()).into_luma8();
    let mut hist = [0u32; 256];
    for p in gray.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total = gray.pixels().count() as u32;
    let half = total / 2;
    let mut cumul = 0u32;
    for (i, &count) in hist.iter().enumerate() {
        cumul += count;
        if cumul >= half {
            return i as f32;
        }
    }
    0.0
}

/// Apply brightness adjustment to an RGB image.
pub(crate) fn adjust_brightness(img: &mut image::RgbImage, factor: f32) {
    for p in img.pixels_mut() {
        p[0] = (p[0] as f32 * factor).min(255.0) as u8;
        p[1] = (p[1] as f32 * factor).min(255.0) as u8;
        p[2] = (p[2] as f32 * factor).min(255.0) as u8;
    }
}

/// Composite a single bulb sprite onto the base image at 50% opacity.
/// Only composites bulbs with Parent="Backglass".
fn composite_bulb(base: &mut image::RgbaImage, bulb: &directb2s::Bulb) {
    // Only Backglass bulbs
    match &bulb.parent {
        Some(p) if p == "Backglass" => {}
        _ => return,
    }

    if bulb.image.is_empty() || bulb.image == "[stripped]" {
        return;
    }

    let x: i32 = bulb.loc_x.parse().unwrap_or(0);
    let y: i32 = bulb.loc_y.parse().unwrap_or(0);
    if x < 0 || y < 0 {
        return;
    }

    let sprite = match decode_b64_image(&bulb.image) {
        Some(img) => img.to_rgba8(),
        None => return,
    };

    let bw = base.width();
    let bh = base.height();
    for sy in 0..sprite.height() {
        for sx in 0..sprite.width() {
            let dx = x as u32 + sx;
            let dy = y as u32 + sy;
            if dx >= bw || dy >= bh {
                continue;
            }
            let src = sprite.get_pixel(sx, sy);
            let alpha = (src[3] as f32 / 255.0) * BULB_OPACITY;
            if alpha < 0.01 {
                continue;
            }
            let dst = base.get_pixel(dx, dy);
            let blend = |s: u8, d: u8| -> u8 {
                ((s as f32 * alpha) + (d as f32 * (1.0 - alpha))).min(255.0) as u8
            };
            base.put_pixel(
                dx,
                dy,
                image::Rgba([
                    blend(src[0], dst[0]),
                    blend(src[1], dst[1]),
                    blend(src[2], dst[2]),
                    255,
                ]),
            );
        }
    }
}

/// Extract the illuminated backglass image from a `.directb2s` file and
/// User-override sources: check `<table_dir>/media/launcher.(png|webp|jpg|jpeg)`
/// in that priority order. Returns the raw file bytes as-is — we don't
/// re-encode because (a) the user picked the format deliberately and (b) the
/// `image` crate's egui loader accepts PNG/WebP/JPEG uniformly.
///
/// Priority rationale: PNG first (lossless, most common for manually
/// assembled frames); WebP second (modern, often smaller at equal quality);
/// JPEG last (lossy, but widely supported).
pub fn extract_backglass_from_launcher_override(table_dir: &Path) -> Option<Vec<u8>> {
    let media = table_dir.join("media");
    for ext in ["png", "webp", "jpg", "jpeg"] {
        let candidate = media.join(format!("launcher.{ext}"));
        if candidate.is_file() {
            match std::fs::read(&candidate) {
                Ok(bytes) if !bytes.is_empty() => {
                    log::info!("Backglass: user override {}", candidate.display());
                    return Some(bytes);
                }
                Ok(_) => {
                    log::warn!("Backglass: {} is empty, skipping", candidate.display());
                }
                Err(e) => {
                    log::warn!("Backglass: failed to read {}: {e}", candidate.display());
                }
            }
        }
    }
    None
}

/// return it as JPEG bytes in memory. The caller stores the bytes in the
/// SQLite cache (`backglass` table) instead of writing a PNG next to the
/// table — see issue #6 for why we no longer drop `.pinready_bg_v*.png`
/// files in user table folders.
///
/// Pipeline:
///   1. Load BackglassImage (base artwork, typically dark/unlit state)
///   2. Crop out the grill/DMD area at the bottom using GrillHeight
///   3. Composite all Backglass bulbs at 50% opacity (GI + flashers)
///   4. Normalize brightness to median luminosity of 75
///   5. Resize to DISPLAY_WIDTH×DISPLAY_HEIGHT and encode as JPEG 85
pub fn extract_backglass_from_b2s(directb2s_path: &Path) -> Option<Vec<u8>> {
    log::info!(
        "Extracting backglass from .directb2s: {}",
        directb2s_path.display()
    );

    let file = match std::fs::File::open(directb2s_path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to open {}: {e}", directb2s_path.display());
            return None;
        }
    };
    let reader = BufReader::new(file);
    let data = match directb2s::read(reader) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to parse {}: {e}", directb2s_path.display());
            return None;
        }
    };

    // Step 1: Load base backglass image
    let bg_image = data.images.backglass_image.as_ref()?;
    if bg_image.value.is_empty() || bg_image.value == "[stripped]" {
        log::warn!("No backglass image data in {}", directb2s_path.display());
        return None;
    }

    let img = decode_b64_image(&bg_image.value)?;

    // Step 2: Crop grill/DMD area
    let grill_height: u32 = data.grill_height.value.parse().unwrap_or(0);
    let crop_height = if grill_height > 0 && grill_height < img.height() {
        img.height() - grill_height
    } else {
        img.height()
    };
    let cropped = img.crop_imm(0, 0, img.width(), crop_height);
    let mut rgba = cropped.to_rgba8();

    // Step 3: Composite Backglass bulbs at 50% opacity
    let bulb_count = if let Some(ref bulbs) = data.illumination.bulb {
        let bg_bulbs: Vec<_> = bulbs
            .iter()
            .filter(|b| {
                b.parent.as_deref() == Some("Backglass")
                    && !b.image.is_empty()
                    && b.image != "[stripped]"
            })
            .collect();
        let count = bg_bulbs.len();
        log::info!(
            "Compositing {} Backglass bulbs at {}% opacity",
            count,
            (BULB_OPACITY * 100.0) as u32
        );
        for bulb in bulbs {
            composite_bulb(&mut rgba, bulb);
        }
        count
    } else {
        0
    };

    // Step 4: Normalize brightness by median luminosity
    let mut rgb = image::DynamicImage::ImageRgba8(rgba).into_rgb8();
    let median = median_luminosity(&rgb);
    if median > 1.0 {
        let factor =
            (TARGET_MEDIAN_LUM / median).clamp(MIN_BRIGHTNESS_FACTOR, MAX_BRIGHTNESS_FACTOR);
        log::info!(
            "Brightness normalization: median {:.0} -> {:.0} (x{:.2})",
            median,
            median * factor,
            factor
        );
        adjust_brightness(&mut rgb, factor);
    } else {
        log::warn!(
            "Very dark image (median={:.0}), skipping normalization",
            median
        );
    }

    // Step 5: Resize and encode JPEG to memory
    let final_img = image::DynamicImage::ImageRgb8(rgb);
    let resized = final_img.resize(
        DISPLAY_WIDTH,
        DISPLAY_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let bytes = encode_jpeg(&resized)?;
    log::info!(
        "Extracted backglass ({}x{}, {} bulbs, {} bytes)",
        resized.width(),
        resized.height(),
        bulb_count,
        bytes.len()
    );
    Some(bytes)
}

/// Extract a backglass image from inside a `.vpx` file by scanning its
/// `images/` catalog for any texture whose logical name contains
/// "backglass" (case-insensitive). Used as a fallback for tables shipped
/// without a companion `.directb2s` — issue #6 cases.
///
/// The `TableInfo.screenshot` field is empirically unusable (empty on
/// virtually every modern pincab table), so we ignore it and look at the
/// real image catalog. Examples this catches:
///
///   - Darkest Dungeon Original — `images/backglassimage.jpg`
///   - Die Hard Trilogy (VPW 2023) — `images/Backglass3scrnON.webp`,
///     `images/DHBackGlassOFF.webp`
///
/// The first matching image with decodable pixel data wins; we decode it
/// (the `.jpeg` field actually holds any encoded format — JPEG, PNG,
/// WebP, BMP), resize to the B2S pipeline dimensions, and return JPEG
/// bytes.
pub fn extract_backglass_from_vpx(vpx_path: &Path) -> Option<Vec<u8>> {
    log::info!(
        "Scanning .vpx images/ for a backglass asset: {}",
        vpx_path.display()
    );
    let mut vpx = match vpin::vpx::open(vpx_path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to open .vpx {}: {e}", vpx_path.display());
            return None;
        }
    };
    let images = match vpx.read_images() {
        Ok(i) => i,
        Err(e) => {
            log::error!(
                "Failed to read image catalog from {}: {e}",
                vpx_path.display()
            );
            return None;
        }
    };

    // Collect all images whose name/path contains "backglass" and rank
    // them by encoded blob size. The real backglass is always a large
    // high-res photographic asset (200-500 KB typical), whereas slot
    // collisions — placeholder textures authors reuse across projects
    // (e.g. the Color_Black.jpg copied from Indianapolis 500 seen in
    // Darkest Dungeon) — compress to < 50 KB. Picking the biggest
    // candidate reliably selects the real one.
    let mut candidates: Vec<_> = images
        .iter()
        .filter(|img| {
            let name = img.name.to_lowercase();
            let path = img.path.to_lowercase();
            name.contains("backglass") || path.contains("backglass")
        })
        .filter_map(|img| {
            // Only entries with an encoded blob are usable; BMP-bits
            // (`bits` field, LZW-compressed BGRA) would need additional
            // decode code and are uncommon for backglass textures.
            let data = img.jpeg.as_ref()?.data.as_slice();
            if data.is_empty() {
                None
            } else {
                Some((img, data))
            }
        })
        .collect();
    if candidates.is_empty() {
        log::debug!(
            "No backglass candidate with encoded data in {}",
            vpx_path.display()
        );
        return None;
    }
    // Sort by blob size DESC, keep the biggest.
    candidates.sort_by_key(|(_, data)| std::cmp::Reverse(data.len()));
    for (img, data) in &candidates {
        log::info!(
            "  candidate: name={:?} path={:?} size={}",
            img.name,
            img.path,
            data.len()
        );
    }
    let (candidate, raw) = candidates[0];
    log::info!(
        "Picked largest backglass candidate: name={:?} ({} bytes)",
        candidate.name,
        raw.len()
    );
    let img = match image::load_from_memory(raw) {
        Ok(i) => i,
        Err(e) => {
            log::warn!(
                "Failed to decode backglass candidate {} from {}: {e}",
                candidate.name,
                vpx_path.display()
            );
            return None;
        }
    };
    let resized = img.resize(
        DISPLAY_WIDTH,
        DISPLAY_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let bytes = encode_jpeg(&resized)?;
    log::info!(
        "Extracted embedded backglass from .vpx ({}x{}, {} bytes)",
        resized.width(),
        resized.height(),
        bytes.len()
    );
    Some(bytes)
}

/// JPEG quality for cached backglass blobs. 85 is the sweet spot for
/// photographic content: visually lossless at 1280×1024, ~5× smaller
/// than PNG, ~2× smaller than WebP lossless.
const BG_JPEG_QUALITY: u8 = 85;

/// Encode a [`image::DynamicImage`] as JPEG bytes at `BG_JPEG_QUALITY`
/// for the SQLite cache. Uses `image::codecs::jpeg::JpegEncoder` because
/// `DynamicImage::write_to(ImageFormat::Jpeg)` doesn't expose a quality
/// parameter.
fn encode_jpeg(img: &image::DynamicImage) -> Option<Vec<u8>> {
    // JPEG can't encode with alpha — flatten to RGB first. Backglass
    // compositing is already done upstream; transparency isn't useful
    // in the cached display copy anyway.
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
            std::io::Cursor::new(&mut buf),
            BG_JPEG_QUALITY,
        );
        enc.encode_image(&rgb).ok()?;
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    // --- median_luminosity ---

    #[test]
    fn median_luminosity_black_image() {
        let img = RgbImage::from_pixel(10, 10, Rgb([0, 0, 0]));
        assert!((median_luminosity(&img) - 0.0).abs() < 1.0);
    }

    #[test]
    fn median_luminosity_white_image() {
        let img = RgbImage::from_pixel(10, 10, Rgb([255, 255, 255]));
        assert!((median_luminosity(&img) - 255.0).abs() < 1.0);
    }

    #[test]
    fn median_luminosity_gray_image() {
        let img = RgbImage::from_pixel(10, 10, Rgb([128, 128, 128]));
        let median = median_luminosity(&img);
        assert!((median - 128.0).abs() < 2.0, "expected ~128, got {median}");
    }

    #[test]
    fn median_luminosity_mixed() {
        // Half black, half white → median should be around 128–255 range
        let mut img = RgbImage::new(10, 10);
        for x in 0..10 {
            for y in 0..10 {
                let val = if y < 5 { 0 } else { 255 };
                img.put_pixel(x, y, Rgb([val, val, val]));
            }
        }
        let median = median_luminosity(&img);
        // With 50 black and 50 white pixels, median is either 0 or 255
        // depending on which side crosses the half mark
        assert!(median == 0.0 || median == 255.0);
    }

    // --- adjust_brightness ---

    #[test]
    fn adjust_brightness_factor_1_unchanged() {
        let mut img = RgbImage::from_pixel(2, 2, Rgb([100, 150, 200]));
        adjust_brightness(&mut img, 1.0);
        let p = img.get_pixel(0, 0);
        assert_eq!(p, &Rgb([100, 150, 200]));
    }

    #[test]
    fn adjust_brightness_factor_2() {
        let mut img = RgbImage::from_pixel(1, 1, Rgb([50, 100, 200]));
        adjust_brightness(&mut img, 2.0);
        let p = img.get_pixel(0, 0);
        assert_eq!(p[0], 100);
        assert_eq!(p[1], 200);
        assert_eq!(p[2], 255); // clamped from 400
    }

    #[test]
    fn adjust_brightness_factor_half() {
        let mut img = RgbImage::from_pixel(1, 1, Rgb([100, 200, 50]));
        adjust_brightness(&mut img, 0.5);
        let p = img.get_pixel(0, 0);
        assert_eq!(p[0], 50);
        assert_eq!(p[1], 100);
        assert_eq!(p[2], 25);
    }

    #[test]
    fn adjust_brightness_clamps_to_255() {
        let mut img = RgbImage::from_pixel(1, 1, Rgb([255, 255, 255]));
        adjust_brightness(&mut img, 3.0);
        let p = img.get_pixel(0, 0);
        assert_eq!(p, &Rgb([255, 255, 255]));
    }

    // --- decode_b64_image ---

    #[test]
    fn decode_b64_invalid_returns_none() {
        assert!(decode_b64_image("not-valid-base64!!!").is_none());
    }

    #[test]
    fn decode_b64_empty_returns_none() {
        assert!(decode_b64_image("").is_none());
    }

    #[test]
    fn decode_b64_valid_png() {
        // Minimal 1x1 red PNG encoded in base64
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
        let img = decode_b64_image(png_b64);
        assert!(img.is_some());
        let img = img.unwrap();
        assert_eq!(img.width(), 1);
        assert_eq!(img.height(), 1);
    }

    #[test]
    fn decode_b64_with_whitespace() {
        // Same PNG with whitespace injected
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAf\n FcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
        assert!(decode_b64_image(png_b64).is_some());
    }
}
