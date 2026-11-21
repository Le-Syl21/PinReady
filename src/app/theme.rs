//! Look of the GUI: two palettes sharing one accent, the plasma orange of a
//! dot-matrix score display (DMD) — dark "cabinet" and light "playfield" —
//! and the egui styles built from them. Widgets read the palette of the
//! active theme through [`pal`], so every color follows a theme switch.
//!
//! The one ornament is the DMD itself: the wordmark and the wizard's step
//! numbers are drawn as lit dots, the way a score display draws them.

use egui::{
    Color32, Context, CornerRadius, FontFamily, FontId, Margin, Pos2, Shadow, Stroke, Style,
    TextStyle, Theme, ThemePreference, Visuals, vec2,
};

/// Every color the GUI paints, for one theme.
#[derive(Clone, Copy)]
pub struct Palette {
    /// Window ground
    pub bg: Color32,
    /// Header and footer bars
    pub bar: Color32,
    /// Cards and grouped content
    pub surface: Color32,
    /// Fields, tracks, hovered cards
    pub raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    /// Primary buttons, the current step, the selected table
    pub accent: Color32,
    /// Text on an accent fill
    pub on_accent: Color32,
    /// The accent used as text on the ground (darker in light mode)
    pub accent_text: Color32,
    /// The unlit dots of the DMD
    pub dmd_off: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    /// Links, information, "an update is available"
    pub info: Color32,
}

impl Palette {
    pub const DARK: Palette = Palette {
        bg: Color32::from_rgb(16, 16, 21),
        bar: Color32::from_rgb(11, 11, 15),
        surface: Color32::from_rgb(25, 25, 33),
        raised: Color32::from_rgb(36, 37, 48),
        border: Color32::from_rgb(48, 49, 63),
        text: Color32::from_rgb(236, 234, 242),
        text_muted: Color32::from_rgb(164, 162, 180),
        text_faint: Color32::from_rgb(110, 109, 124),
        accent: Color32::from_rgb(255, 138, 31),
        on_accent: Color32::from_rgb(28, 14, 0),
        accent_text: Color32::from_rgb(255, 166, 77),
        dmd_off: Color32::from_rgb(46, 30, 20),
        success: Color32::from_rgb(92, 208, 138),
        warning: Color32::from_rgb(242, 193, 78),
        error: Color32::from_rgb(242, 95, 92),
        info: Color32::from_rgb(76, 201, 240),
    };

    pub const LIGHT: Palette = Palette {
        bg: Color32::from_rgb(243, 242, 247),
        bar: Color32::from_rgb(252, 252, 254),
        surface: Color32::from_rgb(255, 255, 255),
        raised: Color32::from_rgb(232, 231, 239),
        border: Color32::from_rgb(211, 210, 222),
        text: Color32::from_rgb(23, 22, 29),
        text_muted: Color32::from_rgb(84, 82, 98),
        text_faint: Color32::from_rgb(134, 132, 148),
        accent: Color32::from_rgb(240, 120, 16),
        on_accent: Color32::from_rgb(28, 14, 0),
        accent_text: Color32::from_rgb(166, 75, 0),
        dmd_off: Color32::from_rgb(232, 218, 204),
        success: Color32::from_rgb(24, 132, 74),
        warning: Color32::from_rgb(168, 112, 0),
        error: Color32::from_rgb(196, 48, 44),
        info: Color32::from_rgb(18, 118, 168),
    };

    pub fn of(theme: Theme) -> Palette {
        let base = match theme {
            Theme::Dark => Self::DARK,
            Theme::Light => Self::LIGHT,
        };
        let (ground, accent) = custom_colors();
        base.with_custom(ground, accent)
    }

    /// The palette with the user's own ground and accent (selection) colours:
    /// bars, cards, fields and borders are derived from the ground, the text
    /// follows its brightness so it stays readable.
    pub fn with_custom(mut self, ground: Option<Color32>, accent: Option<Color32>) -> Palette {
        if let Some(g) = ground {
            let dark = luminance(g) < 0.4;
            let (text_from, lift) = if dark {
                (Self::DARK, Color32::WHITE)
            } else {
                (Self::LIGHT, Color32::BLACK)
            };
            self.text = text_from.text;
            self.text_muted = text_from.text_muted;
            self.text_faint = text_from.text_faint;
            self.success = text_from.success;
            self.warning = text_from.warning;
            self.error = text_from.error;
            self.info = text_from.info;
            self.bg = g;
            self.bar = mix(g, Color32::BLACK, if dark { 0.35 } else { 0.04 });
            self.surface = mix(g, lift, if dark { 0.06 } else { 0.5 });
            self.raised = mix(g, lift, if dark { 0.12 } else { 0.25 });
            self.border = mix(g, lift, if dark { 0.2 } else { 0.35 });
            self.dmd_off = mix(g, lift, 0.1);
        }
        if let Some(a) = accent {
            self.accent = a;
            self.on_accent = if luminance(a) < 0.45 {
                Color32::WHITE
            } else {
                Color32::from_rgb(28, 14, 0)
            };
            // the accent as text on the ground: lifted on a dark ground,
            // darkened on a light one
            self.accent_text = if luminance(self.bg) < 0.4 {
                mix(a, Color32::WHITE, 0.25)
            } else {
                mix(a, Color32::BLACK, 0.3)
            };
        }
        self
    }

    /// `color` laid over the surface at `alpha` (0-255): the ground of a
    /// pill, a banner or a hovered row tinted by its tone.
    pub fn tint(&self, color: Color32, alpha: u8) -> Color32 {
        let a = alpha as f32 / 255.0;
        let mix = |c: u8, s: u8| (c as f32 * a + s as f32 * (1.0 - a)).round() as u8;
        Color32::from_rgb(
            mix(color.r(), self.surface.r()),
            mix(color.g(), self.surface.g()),
            mix(color.b(), self.surface.b()),
        )
    }
}

// ── The user's own colours, kept in the DB ──────────────────────────

static CUSTOM: std::sync::RwLock<(Option<Color32>, Option<Color32>)> =
    std::sync::RwLock::new((None, None));

/// The user's ground and accent colours (`None` = the theme's own).
pub fn custom_colors() -> (Option<Color32>, Option<Color32>) {
    CUSTOM.read().map(|c| *c).unwrap_or((None, None))
}

/// Set the user's colours; [`setup`] must run again to restyle the widgets.
pub fn set_custom_colors(ground: Option<Color32>, accent: Option<Color32>) {
    if let Ok(mut c) = CUSTOM.write() {
        *c = (ground, accent);
    }
}

/// `#RRGGBB` → colour.
pub fn color_from_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn color_to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// Relative luminance (0 black … 1 white), sRGB weights.
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// `a` moved towards `b` by `t` (0..1).
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The palette of the theme egui is drawing with.
pub fn pal(ctx: &Context) -> Palette {
    Palette::of(ctx.theme())
}

pub const RADIUS: u8 = 10;
pub const RADIUS_SMALL: u8 = 6;

// ── Theme preference, kept in the DB ────────────────────────────────

pub fn theme_from_db(value: Option<&str>) -> ThemePreference {
    match value {
        Some("light") => ThemePreference::Light,
        Some("dark") => ThemePreference::Dark,
        _ => ThemePreference::System,
    }
}

pub fn theme_to_db(pref: ThemePreference) -> &'static str {
    match pref {
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
        ThemePreference::System => "system",
    }
}

/// Both styles, then the preference. Fonts are set by the caller.
pub fn setup(ctx: &Context, preference: ThemePreference) {
    ctx.set_style_of(Theme::Dark, style(Theme::Dark));
    ctx.set_style_of(Theme::Light, style(Theme::Light));
    ctx.set_theme(preference);
}

fn style(theme: Theme) -> Style {
    let p = Palette::of(theme);
    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(RADIUS);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if theme == Theme::Dark { 140 } else { 40 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(if theme == Theme::Dark { 110 } else { 30 }),
    };
    v.menu_corner_radius = CornerRadius::same(RADIUS_SMALL);
    v.extreme_bg_color = p.raised;
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.info;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.slider_trailing_fill = true;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.bg_fill = p.tint(p.accent, 46);
    w.hovered.weak_bg_fill = p.tint(p.accent, 46);
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.active.bg_fill = p.tint(p.accent, 80);
    w.active.weak_bg_fill = p.tint(p.accent, 80);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(2.0, p.text);
    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    for s in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        s.corner_radius = CornerRadius::same(RADIUS_SMALL);
        s.expansion = 0.0;
    }
    // Radio buttons, check marks and slider handles take the accent
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.5, p.on_accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);

    let mut style = Style {
        visuals: v,
        ..Style::default()
    };
    style.spacing.item_spacing = vec2(8.0, 7.0);
    style.spacing.button_padding = vec2(11.0, 5.0);
    style.spacing.window_margin = Margin::same(16);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.combo_height = 360.0;
    use FontFamily::{Monospace, Proportional};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(21.0, Proportional)),
        (TextStyle::Body, FontId::new(13.5, Proportional)),
        (TextStyle::Button, FontId::new(13.5, Proportional)),
        (TextStyle::Small, FontId::new(11.0, Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, Monospace)),
    ]
    .into();
    style
}

// ── The DMD ─────────────────────────────────────────────────────────

/// 5×7 dot-matrix glyphs, one row per byte, the five low bits left to right.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        _ => return None,
    })
}

/// The size of `text` drawn by [`paint_dmd`] with dots `pitch` apart.
pub fn dmd_size(text: &str, pitch: f32) -> egui::Vec2 {
    let n = text.chars().count() as f32;
    vec2((n * 6.0 - 1.0).max(0.0) * pitch, 7.0 * pitch)
}

/// Draw `text` as a dot-matrix display: lit dots in `lit`, the rest of each
/// glyph cell in `off` (the faint grid a real DMD shows). Characters without
/// a glyph are left blank.
pub fn paint_dmd(
    painter: &egui::Painter,
    top_left: Pos2,
    text: &str,
    pitch: f32,
    lit: Color32,
    off: Color32,
) {
    let r = pitch * 0.38;
    for (i, c) in text.chars().enumerate() {
        let Some(rows) = glyph(c) else { continue };
        let x0 = top_left.x + i as f32 * 6.0 * pitch;
        for (y, row) in rows.iter().enumerate() {
            for x in 0..5 {
                let on = row & (0x10 >> x) != 0;
                let center = Pos2::new(
                    x0 + (x as f32 + 0.5) * pitch,
                    top_left.y + (y as f32 + 0.5) * pitch,
                );
                painter.circle_filled(center, r, if on { lit } else { off });
            }
        }
    }
}

/// The PinReady wordmark as a widget, `pitch` points per dot.
pub fn dmd_wordmark(ui: &mut egui::Ui, pitch: f32) -> egui::Response {
    let p = pal(ui.ctx());
    let text = "PINREADY";
    let (rect, resp) = ui.allocate_exact_size(dmd_size(text, pitch), egui::Sense::hover());
    paint_dmd(ui.painter(), rect.min, text, pitch, p.accent, p.dmd_off);
    resp
}

// ── Buttons ─────────────────────────────────────────────────────────

/// The screen's main action: accent fill, dark text.
pub fn primary_button(
    text: impl Into<String>,
    p: &Palette,
    enabled: bool,
) -> egui::Button<'static> {
    let (fill, fg) = if enabled {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text_faint)
    };
    egui::Button::new(egui::RichText::new(text.into()).color(fg).strong())
        .fill(fill)
        .stroke(Stroke::NONE)
        .min_size(vec2(112.0, 30.0))
}

/// [`primary_button`] for an already laid-out label (an underlined mnemonic).
pub fn primary_button_text(
    text: egui::WidgetText,
    p: &Palette,
    enabled: bool,
) -> egui::Button<'static> {
    let fill = if enabled { p.accent } else { p.raised };
    egui::Button::new(text)
        .fill(fill)
        .stroke(Stroke::NONE)
        .min_size(vec2(112.0, 30.0))
}

/// Text colour of [`primary_button`].
pub fn primary_fg(p: &Palette, enabled: bool) -> Color32 {
    if enabled { p.on_accent } else { p.text_faint }
}

/// A secondary action: outlined, on the ground. The label is already laid
/// out (an underlined mnemonic, see `mnemonic.rs`).
pub fn secondary_button_text(text: egui::WidgetText, p: &Palette) -> egui::Button<'static> {
    egui::Button::new(text)
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, p.border))
        .min_size(vec2(0.0, 30.0))
}

// ── Page structure ──────────────────────────────────────────────────

/// A translated label as a title: without the trailing colon or ellipsis
/// some strings carry for inline use ("Mode d'affichage :").
pub fn clean(text: &str) -> &str {
    text.trim_end()
        .trim_end_matches([':', '\u{ff1a}', '\u{2026}'])
        .trim_end()
}

/// The page title, with an optional line under it.
pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    let p = pal(ui.ctx());
    ui.label(
        egui::RichText::new(clean(title))
            .size(24.0)
            .color(p.text)
            .strong(),
    );
    if let Some(sub) = subtitle {
        ui.label(egui::RichText::new(sub).color(p.text_muted));
    }
    ui.add_space(12.0);
}

/// One group of settings: a bordered card with its title, an optional
/// sentence saying what it is for, then its controls.
pub fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    intro: Option<&str>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let p = pal(ui.ctx());
    let r = egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(clean(title))
                    .size(17.0)
                    .color(p.text)
                    .strong(),
            );
            if let Some(intro) = intro {
                ui.label(egui::RichText::new(intro).color(p.text_muted));
            }
            ui.add_space(6.0);
            add_contents(ui)
        })
        .inner;
    ui.add_space(12.0);
    r
}

/// The label above a control inside a section.
pub fn field_label(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let p = pal(ui.ctx());
    ui.label(egui::RichText::new(clean(text)).color(p.text).strong())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_colours_stay_readable() {
        let bordeaux = color_from_hex("#5C0A1E").unwrap();
        let ferrari = color_from_hex("#FF2800").unwrap();
        let p = Palette::DARK.with_custom(Some(bordeaux), Some(ferrari));
        assert_eq!(p.bg, bordeaux);
        assert_eq!(p.accent, ferrari);
        assert_eq!(p.text, Palette::DARK.text); // light text on a dark ground
        assert_eq!(p.on_accent, Color32::WHITE); // white on Ferrari red
        assert!(luminance(p.surface) > luminance(p.bg) && luminance(p.bar) < luminance(p.bg));
        // a light ground takes the dark text, whatever the theme
        let p = Palette::DARK.with_custom(Some(Color32::from_rgb(250, 240, 230)), None);
        assert_eq!(p.text, Palette::LIGHT.text);
        assert_eq!(color_to_hex(ferrari), "#FF2800");
        assert_eq!(color_from_hex("nope"), None);
    }
}
