//! How a picture goes into a launcher tile whose shape it does not have.
//!
//! Three ways ([`TileFit`]): the whole picture with bands, the tile filled by
//! the picture cropped to its centre, or the picture stretched to the tile.
//! The bands themselves ([`BandFill`]) are black, the picture blurred behind
//! itself, or the colour of the picture's edges (seamless for wheels on a
//! plain background).
//!
//! The blurred backdrop and the edge colour are made once per picture on the
//! CPU, from its thumbnail ([`Backdrop::compute`]): a tiny blurred copy that
//! the GPU enlarges, which is what blurs it.

use egui::{Color32, ColorImage, TextureHandle};

/// How the picture fits the tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileFit {
    /// The whole picture, bands where the shapes differ.
    Bands,
    /// The tile filled, the picture cropped to its centre.
    Crop,
    /// The picture stretched to the tile (distorted).
    Stretch,
}

impl TileFit {
    pub const ALL: [Self; 3] = [Self::Bands, Self::Crop, Self::Stretch];

    pub fn key(self) -> &'static str {
        match self {
            Self::Bands => "bands",
            Self::Crop => "crop",
            Self::Stretch => "stretch",
        }
    }

    /// From the settings: `tile_fit`, else the older `tile_fill` ("1" was
    /// the crop), else bands.
    pub fn from_config(fit: Option<&str>, old_fill: Option<&str>) -> Self {
        match fit {
            Some(k) => Self::ALL
                .into_iter()
                .find(|f| f.key() == k)
                .unwrap_or(Self::Bands),
            None if old_fill == Some("1") => Self::Crop,
            None => Self::Bands,
        }
    }
}

/// What a tile shows: the table's picture, or its wheel (the picture when
/// the table has no wheel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilePicture {
    Picture,
    Wheel,
}

impl TilePicture {
    pub const ALL: [Self; 2] = [Self::Picture, Self::Wheel];

    pub fn key(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::Wheel => "wheel",
        }
    }

    pub fn from_key(k: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|p| Some(p.key()) == k)
            .unwrap_or(Self::Picture)
    }
}

/// A wheel in `area`: whole, a little inside the edges, over whatever is
/// behind it (wheels have transparent surroundings).
pub(super) fn paint_wheel(
    painter: &egui::Painter,
    tex: &TextureHandle,
    area: egui::Rect,
    tint: Color32,
) {
    let size = tex.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let inner = area.shrink(area.height().min(area.width()) * 0.06);
    painter.image(
        tex.id(),
        fit_rect(size, inner),
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        tint,
    );
}

/// Where [`paint_wheel`] draws the wheel inside `area`.
pub(super) fn wheel_rect(tex: &TextureHandle, area: egui::Rect) -> egui::Rect {
    let inner = area.shrink(area.height().min(area.width()) * 0.06);
    fit_rect(tex.size_vec2(), inner)
}

/// What fills the bands of [`TileFit::Bands`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandFill {
    Black,
    /// The picture itself, enlarged to cover the tile, blurred and darkened.
    Blur,
    /// The average colour of the picture's edges.
    Color,
}

impl BandFill {
    pub const ALL: [Self; 3] = [Self::Black, Self::Blur, Self::Color];

    pub fn key(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Blur => "blur",
            Self::Color => "color",
        }
    }

    pub fn from_key(k: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|f| Some(f.key()) == k)
            .unwrap_or(Self::Black)
    }
}

/// Width of the blurred copy; the GPU enlarges it to the tile.
const BACKDROP_WIDTH: u32 = 48;
/// Brightness kept in the blurred copy, so the picture stands out of it.
const BACKDROP_DIM: f32 = 0.55;

/// The blurred copy (in the GPU) and the edge colour of one picture.
#[derive(Clone)]
pub struct Backdrop {
    pub blur: TextureHandle,
    pub edge: Color32,
}

/// [`Backdrop`] before its upload.
pub struct BackdropPixels {
    pub blur: ColorImage,
    pub edge: Color32,
}

impl Backdrop {
    /// The blurred copy and the edge colour of a decoded picture (CPU work,
    /// for a decoding thread).
    pub fn compute(image: &ColorImage) -> Option<BackdropPixels> {
        let [w, h] = image.size;
        if w == 0 || h == 0 {
            return None;
        }
        let edge = edge_color(image);
        let rgba = image::RgbaImage::from_raw(
            w as u32,
            h as u32,
            image.pixels.iter().flat_map(|c| c.to_array()).collect(),
        )?;
        let bh = ((h as u32 * BACKDROP_WIDTH) / w as u32).max(1);
        let small = image::imageops::resize(
            &rgba,
            BACKDROP_WIDTH,
            bh,
            image::imageops::FilterType::Triangle,
        );
        let mut blurred = image::imageops::blur(&small, 2.0);
        for p in blurred.pixels_mut() {
            for c in &mut p.0[..3] {
                *c = (*c as f32 * BACKDROP_DIM) as u8;
            }
            p.0[3] = 255;
        }
        Some(BackdropPixels {
            blur: ColorImage::from_rgba_unmultiplied(
                [blurred.width() as usize, blurred.height() as usize],
                blurred.as_raw(),
            ),
            edge,
        })
    }

    pub fn upload(ctx: &egui::Context, name: String, px: BackdropPixels) -> Self {
        Self {
            blur: ctx.load_texture(name, px.blur, egui::TextureOptions::LINEAR),
            edge: px.edge,
        }
    }
}

/// The average colour of the outer ring of a picture (4 % of each side, at
/// least one pixel), opaque.
fn edge_color(image: &ColorImage) -> Color32 {
    let [w, h] = image.size;
    let bx = (w / 25).max(1);
    let by = (h / 25).max(1);
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            if x < bx || x >= w - bx || y < by || y >= h - by {
                let [pr, pg, pb, _] = image.pixels[y * w + x].to_array();
                r += pr as u64;
                g += pg as u64;
                b += pb as u64;
                n += 1;
            }
        }
    }
    let n = n.max(1);
    Color32::from_rgb((r / n) as u8, (g / n) as u8, (b / n) as u8)
}

/// Paint a tile picture into `area` the way the settings say. `backdrop` is
/// the picture's blurred copy and edge colour, if made already (black bands
/// until then).
pub(super) fn paint_tile_picture(
    ui: &egui::Ui,
    tex: &TextureHandle,
    area: egui::Rect,
    fit: TileFit,
    bands: BandFill,
    backdrop: Option<&Backdrop>,
) {
    let size = tex.size_vec2();
    let painter = ui.painter_at(area);
    let radius = egui::CornerRadius::same(4);
    let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if size.x <= 0.0 || size.y <= 0.0 {
        painter.rect_filled(area, radius, Color32::BLACK);
        return;
    }
    let image = |uv: egui::Rect, rect: egui::Rect| {
        egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size))
            .corner_radius(radius)
            .uv(uv)
            .paint_at(ui, rect);
    };
    match fit {
        TileFit::Crop => image(fill_uv(size, area.size()), area),
        TileFit::Stretch => image(full_uv, area),
        TileFit::Bands => {
            match (bands, backdrop) {
                (BandFill::Blur, Some(b)) => {
                    egui::Image::from_texture(egui::load::SizedTexture::new(
                        b.blur.id(),
                        b.blur.size_vec2(),
                    ))
                    .corner_radius(radius)
                    .uv(fill_uv(b.blur.size_vec2(), area.size()))
                    .paint_at(ui, area);
                }
                (BandFill::Color, Some(b)) => {
                    painter.rect_filled(area, radius, b.edge);
                }
                _ => {
                    painter.rect_filled(area, radius, Color32::BLACK);
                }
            }
            image(full_uv, fit_rect(size, area));
        }
    }
}

/// The largest rectangle with the shape of `size`, centred in `area`.
pub(super) fn fit_rect(size: egui::Vec2, area: egui::Rect) -> egui::Rect {
    let scale = (area.width() / size.x).min(area.height() / size.y);
    egui::Rect::from_center_size(area.center(), size * scale)
}

/// The part of a picture of `size` (centred) that has the aspect ratio of `area`.
pub(super) fn fill_uv(size: egui::Vec2, area: egui::Vec2) -> egui::Rect {
    let (pic, tile) = (size.x / size.y, area.x / area.y.max(1.0));
    if pic > tile {
        let w = tile / pic;
        egui::Rect::from_min_max(
            egui::pos2((1.0 - w) / 2.0, 0.0),
            egui::pos2((1.0 + w) / 2.0, 1.0),
        )
    } else {
        let h = pic / tile;
        egui::Rect::from_min_max(
            egui::pos2(0.0, (1.0 - h) / 2.0),
            egui::pos2(1.0, (1.0 + h) / 2.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_crops_the_centre_with_the_tile_ratio() {
        // a 16:9 picture in a 5:4 tile keeps the middle 70 % of its width
        let uv = fill_uv(egui::vec2(1600.0, 900.0), egui::vec2(480.0, 384.0));
        assert!((uv.width() - 0.703).abs() < 0.001 && uv.height() == 1.0);
        assert!((uv.center().x - 0.5).abs() < 1e-6);
        // a tall picture keeps the middle band of its height
        let uv = fill_uv(egui::vec2(400.0, 800.0), egui::vec2(480.0, 384.0));
        assert!(uv.width() == 1.0 && (uv.height() - 0.4).abs() < 0.001);
    }

    #[test]
    fn bands_keep_the_picture_shape() {
        // the bug: a 1:1 wheel in a 16:9 tile was painted over the whole tile
        let area = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(480.0, 270.0));
        let r = fit_rect(egui::vec2(384.0, 384.0), area);
        assert!((r.width() - 270.0).abs() < 0.01 && (r.height() - 270.0).abs() < 0.01);
        assert!((r.center() - area.center()).length() < 0.01);
        // a wide picture in a tall tile: full width, bands above and below
        let tall = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(288.0, 384.0));
        let r = fit_rect(egui::vec2(1600.0, 900.0), tall);
        assert!((r.width() - 288.0).abs() < 0.01 && (r.height() - 162.0).abs() < 0.01);
    }

    #[test]
    fn old_fill_setting_maps_to_crop() {
        assert_eq!(TileFit::from_config(None, Some("1")), TileFit::Crop);
        assert_eq!(TileFit::from_config(None, Some("0")), TileFit::Bands);
        assert_eq!(TileFit::from_config(None, None), TileFit::Bands);
        assert_eq!(
            TileFit::from_config(Some("stretch"), Some("1")),
            TileFit::Stretch
        );
        assert_eq!(TileFit::from_config(Some("??"), None), TileFit::Bands);
        for f in BandFill::ALL {
            assert_eq!(BandFill::from_key(Some(f.key())), f);
        }
        assert_eq!(BandFill::from_key(None), BandFill::Black);
    }

    #[test]
    fn backdrop_takes_the_edge_colour_and_is_small() {
        // a blue picture with a yellow disc in the middle: the edges are blue
        let (w, h) = (100usize, 80usize);
        let mut img = ColorImage::new([w, h], vec![Color32::from_rgb(20, 40, 110); w * h]);
        for y in 30..50 {
            for x in 40..60 {
                img.pixels[y * w + x] = Color32::from_rgb(240, 180, 40);
            }
        }
        let b = Backdrop::compute(&img).unwrap();
        assert_eq!(b.edge, Color32::from_rgb(20, 40, 110));
        assert_eq!(b.blur.size[0], BACKDROP_WIDTH as usize);
        assert!(Backdrop::compute(&ColorImage::default()).is_none());
    }
}
