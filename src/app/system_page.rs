use super::*;

/// Ground presets (colour, name key): deep colours that keep light text.
const GROUND_PRESETS: &[(&str, &str)] = &[
    ("#5C0A1E", "color_bordeaux"),
    ("#0B1A33", "color_night_blue"),
    ("#0F2A1D", "color_racing_green"),
    ("#2A2A2E", "color_slate"),
    ("#000000", "color_black"),
];

/// Selection presets (colour, name key): bright, they must stand out.
const ACCENT_PRESETS: &[(&str, &str)] = &[
    ("#FF2800", "color_ferrari_red"),
    ("#FF8A1F", "color_dmd_orange"),
    ("#FFD400", "color_yellow"),
    ("#00C853", "color_green"),
    ("#2F80FF", "color_blue"),
    ("#E040FB", "color_magenta"),
];

impl App {
    /// Ground and selection colours on top of the light / dark theme, shown
    /// live (the whole interface restyles on each change) and kept in the DB.
    fn render_colors_section(&mut self, ui: &mut egui::Ui) {
        theme::section(
            ui,
            &t!("system_colors_title"),
            Some(&t!("system_colors_intro")),
            |ui| {
                let base = match ui.ctx().theme() {
                    egui::Theme::Dark => theme::Palette::DARK,
                    egui::Theme::Light => theme::Palette::LIGHT,
                };
                let (mut ground, mut accent) = theme::custom_colors();
                let mut changed = false;
                let mut g = ground.unwrap_or(base.bg);
                let mut a = accent.unwrap_or(base.accent);
                ui.horizontal(|ui| {
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut g,
                        egui::color_picker::Alpha::Opaque,
                    )
                    .changed()
                    {
                        ground = Some(g);
                        changed = true;
                    }
                    ui.label(t!("system_colors_ground"));
                    ui.add_space(24.0);
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut a,
                        egui::color_picker::Alpha::Opaque,
                    )
                    .changed()
                    {
                        accent = Some(a);
                        changed = true;
                    }
                    ui.label(t!("system_colors_accent"));
                });
                // One choice for the ground, one for the selection: swatches,
                // then the original colours.
                let swatch = |ui: &mut egui::Ui, c: egui::Color32, name: String, on: bool| {
                    let (rect, r) =
                        ui.allocate_exact_size(egui::vec2(34.0, 24.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 4.0, c);
                    let stroke = if on {
                        egui::Stroke::new(2.5, ui.visuals().strong_text_color())
                    } else {
                        egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color)
                    };
                    ui.painter()
                        .rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
                    r.on_hover_text(name).clicked()
                };
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(t!("system_colors_ground"));
                    for (hex, key) in GROUND_PRESETS {
                        let c = theme::color_from_hex(hex);
                        if let Some(c) = c
                            && swatch(ui, c, t!(*key).to_string(), ground == Some(c))
                        {
                            ground = Some(c);
                            changed = true;
                        }
                    }
                    if ground.is_some() && ui.small_button(t!("system_colors_reset")).clicked() {
                        ground = None;
                        changed = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(t!("system_colors_accent"));
                    for (hex, key) in ACCENT_PRESETS {
                        let c = theme::color_from_hex(hex);
                        if let Some(c) = c
                            && swatch(ui, c, t!(*key).to_string(), accent == Some(c))
                        {
                            accent = Some(c);
                            changed = true;
                        }
                    }
                    if accent.is_some() && ui.small_button(t!("system_colors_reset")).clicked() {
                        accent = None;
                        changed = true;
                    }
                });
                if changed {
                    let hex =
                        |c: Option<egui::Color32>| c.map(theme::color_to_hex).unwrap_or_default();
                    let _ = self.db.set_config("ui_color_ground", &hex(ground));
                    let _ = self.db.set_config("ui_color_accent", &hex(accent));
                    theme::set_custom_colors(ground, accent);
                    theme::setup(ui.ctx(), self.theme_pref);
                }
            },
        );
    }

    pub(super) fn render_system_page(&mut self, ui: &mut egui::Ui) {
        theme::page_header(ui, &t!("system_heading"), Some(&t!("system_desc")));

        theme::section(ui, &t!("system_section_start"), None, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.autostart, t!("autostart_label"));
                help_marker(ui, &t!("autostart_hint"));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut self.desktop_integration,
                    t!("desktop_integration_label"),
                );
                help_marker(ui, &t!("desktop_integration_hint"));
            });

            // Two conveniences the menu entry doesn't cover. Separate toggles:
            // a cabinet wants the desktop icon, a workstation usually wants
            // neither. Like every box on this page they only record the choice:
            // Finish applies it (`system_apply`), off the UI thread — creating a
            // Windows shortcut from the click handler froze the window.
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.desktop_shortcut, t!("desktop_shortcut_label"));
                help_marker(ui, &t!("desktop_shortcut_hint"));
            });

            // Pinning is shell-specific and we only speak GNOME's dialect, so the
            // option simply isn't there on a session we can't serve.
            if self.dock_pinning_available {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.dock_pinned, t!("dock_pin_label"));
                    help_marker(ui, &t!("dock_pin_hint"));
                });
            }
        });

        // ---- Colours: the user's own ground and selection colour.
        self.render_colors_section(ui);

        // ---- Keyboard: what works without a mouse.
        theme::section(
            ui,
            &t!("system_keys_title"),
            Some(&t!("system_keys_intro")),
            |ui| {
                egui::Grid::new("keyboard_shortcuts")
                    .num_columns(2)
                    .spacing([18.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (keys, what) in [
                            ("system_keys_alt_letter", "system_keys_alt_letter_do"),
                            ("system_keys_alt_digit", "system_keys_alt_digit_do"),
                            ("system_keys_zoom", "system_keys_zoom_do"),
                            ("system_keys_nav", "system_keys_nav_do"),
                            ("system_keys_start", "system_keys_start_do"),
                            ("system_keys_ball", "system_keys_ball_do"),
                            ("system_keys_extraball", "system_keys_extraball_do"),
                        ] {
                            ui.label(egui::RichText::new(t!(keys)).strong());
                            ui.label(t!(what));
                            ui.end_row();
                        }
                    });
            },
        );

        // ---- VPinPlay: community ratings, and sharing the table usage.
        self.render_vpinplay_section(ui);

        // ---- Self-hosted mirror (VBS catalog + VPin media DB). Empty
        // = direct GitHub fetch (the default). When set, all index URLs
        // and per-asset URLs route through the mirror; the server is
        // responsible for rewriting URLs inside the manifests so they
        // point back at itself (cf. db.rs::mirror_base_url docs).
        theme::section(
            ui,
            &t!("system_mirror_label"),
            Some(&t!("system_mirror_intro")),
            |ui| {
                help_marker(ui, &t!("system_mirror_hint"));
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.mirror_base_url)
                            .hint_text("https://pinready.syl21.org")
                            .desired_width(420.0),
                    );
                    if resp.changed()
                        && let Err(e) = self.db.set_mirror_base_url(&self.mirror_base_url)
                    {
                        log::error!("Failed to persist mirror_base_url: {e}");
                    }
                    if ui
                        .button(t!("system_mirror_clear"))
                        .on_hover_text(t!("system_mirror_clear_hint"))
                        .clicked()
                    {
                        self.mirror_base_url.clear();
                        let _ = self.db.set_mirror_base_url("");
                    }
                });
            },
        );

        // ---- Credits — last wizard page is the natural spot for "thanks
        // to" since it's the screen the user sees just before Finish.
        // Names sorted alphabetically (case-insensitive).
        theme::section(
            ui,
            &t!("system_credits_title"),
            Some(&t!("system_credits_intro")),
            |ui| {
                for name in [
                    "Caviar4456",
                    "Francisdb",
                    "Jsm174",
                    "Major Frenchy",
                    "Somatik",
                    "Spielfool",
                    "Superhac",
                    "Toxie",
                    "Vbousquet",
                ] {
                    ui.label(format!("• {name}"));
                }
            },
        );

        // ---- The author's other cabinet projects: name = clickable link,
        // then a one-line description.
        theme::section(ui, &t!("system_projects_title"), None, |ui| {
            for (name, url, desc) in [
                (
                    "rom2altsound",
                    "https://github.com/Le-Syl21/rom2altsound",
                    t!("system_projects_rom2altsound"),
                ),
                (
                    "headtracking",
                    "https://github.com/Le-Syl21/headtracking",
                    t!("system_projects_headtracking"),
                ),
            ] {
                ui.hyperlink_to(name, url);
                ui.label(desc);
                ui.add_space(6.0);
            }
        });
    }
}
