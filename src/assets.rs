use std::io::BufReader;
use std::path::{Path, PathBuf};

use base64::Engine;

// Versioned cache filename — bump when extraction logic changes
const CACHE_FILENAME: &str = ".pinready_bg_v4.png";

// Illumination: all bulbs composited at this opacity
const BULB_OPACITY: f32 = 0.50;

// Brightness normalization: target median luminosity (0–255)
const TARGET_MEDIAN_LUM: f32 = 75.0;

// Max brightness adjustment factor to avoid blowing out dark images
const MAX_BRIGHTNESS_FACTOR: f32 = 2.5;
const MIN_BRIGHTNESS_FACTOR: f32 = 0.5;

/// Get the cached backglass image path for a table directory.
pub fn cached_bg_path(table_dir: &Path) -> PathBuf {
    table_dir.join(CACHE_FILENAME)
}

/// Decode base64 image data (with whitespace stripping) to a DynamicImage.
fn decode_b64_image(b64: &str) -> Option<image::DynamicImage> {
    let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .ok()?;
    image::load_from_memory(&bytes).ok()
}

/// Compute the median luminosity of an RGB image using its histogram.
fn median_luminosity(img: &image::RgbImage) -> f32 {
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
fn adjust_brightness(img: &mut image::RgbImage, factor: f32) {
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

/// Extract the backglass image from a .directb2s file and cache it as PNG.
///
/// Pipeline:
/// 1. Load BackglassImage (base artwork, typically dark/unlit state)
/// 2. Crop out the grill/DMD area at the bottom using GrillHeight
/// 3. Composite all Backglass bulbs at 50% opacity (GI + flashers)
/// 4. Normalize brightness to median luminosity of 75
/// 5. Resize for display and save as PNG cache
///
/// Returns the path to the cached image, or None if extraction failed.
pub fn extract_backglass(directb2s_path: &Path, table_dir: &Path) -> Option<PathBuf> {
    let cache_path = cached_bg_path(table_dir);

    if cache_path.exists() {
        return Some(cache_path);
    }

    log::info!("Extracting backglass from {}", directb2s_path.display());

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

    // Step 5: Resize and save
    let final_img = image::DynamicImage::ImageRgb8(rgb);
    let resized = final_img.resize(1280, 1024, image::imageops::FilterType::Lanczos3);
    if let Err(e) = resized.save(&cache_path) {
        log::error!("Failed to save cache {}: {e}", cache_path.display());
        return None;
    }

    log::info!(
        "Cached backglass: {} ({}x{}, {} bulbs)",
        cache_path.display(),
        resized.width(),
        resized.height(),
        bulb_count
    );
    Some(cache_path)
}
