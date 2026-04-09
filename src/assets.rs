use std::io::BufReader;
use std::path::{Path, PathBuf};

use base64::Engine;

// Versioned cache filename — bump when extraction logic changes (resolution, image source, etc.)
const CACHE_FILENAME: &str = ".pinready_bg_v3.png";

/// Get the cached backglass image path for a table directory.
pub fn cached_bg_path(table_dir: &Path) -> PathBuf {
    table_dir.join(CACHE_FILENAME)
}

/// Decode base64 image data (with whitespace stripping) to a DynamicImage.
fn decode_b64_image(b64: &str) -> Option<image::DynamicImage> {
    let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(&clean).ok()?;
    image::load_from_memory(&bytes).ok()
}

/// Composite all Bulb sprites onto the base backglass image to produce the illuminated version.
fn composite_bulbs(base: &mut image::RgbaImage, bulbs: &[directb2s::Bulb]) {
    let composited = bulbs.iter().filter(|b| !b.image.is_empty() && b.image != "[stripped]").count();
    log::info!("Compositing {} bulb sprites onto backglass", composited);

    for bulb in bulbs {
        if bulb.image.is_empty() || bulb.image == "[stripped]" {
            continue;
        }
        let x: i32 = bulb.loc_x.parse().unwrap_or(0);
        let y: i32 = bulb.loc_y.parse().unwrap_or(0);
        if x < 0 || y < 0 { continue; }

        let sprite = match decode_b64_image(&bulb.image) {
            Some(img) => img.to_rgba8(),
            None => continue,
        };

        // Overlay sprite onto base at (x, y) with alpha blending
        let bw = base.width();
        let bh = base.height();
        for sy in 0..sprite.height() {
            for sx in 0..sprite.width() {
                let dx = x as u32 + sx;
                let dy = y as u32 + sy;
                if dx >= bw || dy >= bh { continue; }
                let src = sprite.get_pixel(sx, sy);
                let alpha = src[3] as f32 / 255.0;
                if alpha < 0.01 { continue; }
                let dst = base.get_pixel(dx, dy);
                let blend = |s: u8, d: u8| -> u8 {
                    ((s as f32 * alpha) + (d as f32 * (1.0 - alpha))).min(255.0) as u8
                };
                base.put_pixel(dx, dy, image::Rgba([
                    blend(src[0], dst[0]),
                    blend(src[1], dst[1]),
                    blend(src[2], dst[2]),
                    255,
                ]));
            }
        }
    }
}

/// Extract the backglass image from a .directb2s file and cache it as PNG.
/// Returns the path to the cached image, or None if extraction failed.
pub fn extract_backglass(directb2s_path: &Path, table_dir: &Path) -> Option<PathBuf> {
    let cache_path = cached_bg_path(table_dir);

    // Already cached?
    if cache_path.exists() {
        return Some(cache_path);
    }

    log::info!("Extracting backglass from {}", directb2s_path.display());

    let file = match std::fs::File::open(directb2s_path) {
        Ok(f) => f,
        Err(e) => { log::error!("Failed to open {}: {e}", directb2s_path.display()); return None; }
    };
    let reader = BufReader::new(file);
    let data = match directb2s::read(reader) {
        Ok(d) => d,
        Err(e) => { log::error!("Failed to parse {}: {e}", directb2s_path.display()); return None; }
    };

    // Priority: BackglassOnImage (pre-composited) > BackglassImage + Bulbs > thumbnail
    let b64_data = if let Some(ref on_img) = data.images.backglass_on_image {
        log::info!("Using pre-composited illuminated backglass");
        &on_img.value
    } else if let Some(ref bg) = data.images.backglass_image {
        &bg.value
    } else {
        log::info!("Using thumbnail (no backglass image available)");
        &data.images.thumbnail_image.value
    };

    if b64_data.is_empty() || b64_data == "[stripped]" {
        log::warn!("No backglass image data in {}", directb2s_path.display());
        return None;
    }

    log::info!("Decoding base64 ({} chars)...", b64_data.len());

    let img = match decode_b64_image(b64_data) {
        Some(i) => i,
        None => { log::error!("Image decode failed for {}", directb2s_path.display()); return None; }
    };

    // If we used BackglassImage (not OnImage), composite bulb sprites for illumination
    let has_on_image = data.images.backglass_on_image.is_some();
    let has_bulbs = data.illumination.bulb.as_ref().map_or(false, |b| !b.is_empty());
    let img = if !has_on_image && has_bulbs {
        let bulbs = data.illumination.bulb.as_ref().unwrap();
        log::info!("No BackglassOnImage — compositing {} bulbs onto base image", bulbs.len());
        let mut rgba = img.to_rgba8();
        composite_bulbs(&mut rgba, bulbs);
        image::DynamicImage::ImageRgba8(rgba)
    } else {
        img
    };

    // Crop out the grill/DMD area at the bottom using GrillHeight
    let grill_height: u32 = data.grill_height.value.parse().unwrap_or(0);
    let crop_height = if grill_height > 0 && grill_height < img.height() {
        img.height() - grill_height
    } else {
        img.height()
    };
    let cropped = img.crop_imm(0, 0, img.width(), crop_height);

    // Resize for backglass viewport display (keep aspect ratio)
    let resized = cropped.resize(1280, 1024, image::imageops::FilterType::Lanczos3);
    if let Err(e) = resized.save(&cache_path) {
        log::error!("Failed to save cache {}: {e}", cache_path.display());
        return None;
    }

    log::info!("Cached backglass: {} ({}x{})", cache_path.display(), resized.width(), resized.height());
    Some(cache_path)
}
