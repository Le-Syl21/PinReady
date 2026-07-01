use super::*;

impl App {
    pub(super) fn render_audio_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(t!("audio_heading"));
        ui.add_space(4.0);

        // 3-jack repurposing tip — shown for all modes (stereo users may
        // want to upgrade). OS-specific tool name + link to a README
        // with details + screenshots.
        #[cfg(target_os = "linux")]
        let (tool, url) = (
            "hdajackretask",
            "https://github.com/Le-Syl21/PinReady/tree/main/audiomapping_linux",
        );
        #[cfg(target_os = "windows")]
        let (tool, url) = (
            "Realtek HD Audio Manager",
            "https://github.com/Le-Syl21/PinReady/tree/main/audiomapping_windows",
        );
        #[cfg(target_os = "macos")]
        let (tool, url) = (
            "Audio MIDI Setup",
            "https://github.com/Le-Syl21/PinReady/tree/main/audiomapping_mac",
        );
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(200, 180, 100),
                t!("audio_3jack_tip", tool = tool),
            );
            ui.hyperlink_to(t!("audio_3jack_tip_more_info"), url);
        });
        ui.add_space(8.0);

        // Device assignment
        ui.strong(t!("audio_device_assignment"));
        ui.add_space(4.0);

        egui::Grid::new("audio_devices")
            .min_col_width(150.0)
            .show(ui, |ui| {
                ui.label(t!("audio_backglass"));
                egui::ComboBox::from_id_salt("device_bg")
                    .selected_text(if self.audio.device_bg.is_empty() {
                        t!("audio_default_device").to_string()
                    } else {
                        self.audio.device_bg.clone()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.audio.device_bg,
                            String::new(),
                            t!("audio_default_device"),
                        );
                        for dev in &self.audio.available_devices {
                            ui.selectable_value(&mut self.audio.device_bg, dev.clone(), dev);
                        }
                    });
                ui.end_row();

                ui.label(t!("audio_playfield"));
                egui::ComboBox::from_id_salt("device_pf")
                    .selected_text(if self.audio.device_pf.is_empty() {
                        t!("audio_default_device").to_string()
                    } else {
                        self.audio.device_pf.clone()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.audio.device_pf,
                            String::new(),
                            t!("audio_default_device"),
                        );
                        for dev in &self.audio.available_devices {
                            ui.selectable_value(&mut self.audio.device_pf, dev.clone(), dev);
                        }
                    });
                ui.end_row();
            });

        // Auto-detect the playfield device's native channel count and
        // pre-select the matching Sound3D mode. Stereo → mode 1 (rear
        // stereo). 5.1 → mode 3 (surround front at lockbar). 7.1 →
        // mode 5 (SSF new). The user can still override with the
        // radio buttons below.
        //
        // Two trigger conditions:
        //   1. The user picks a different playfield device (run again).
        //   2. The ini didn't provide a `Sound3D` value (typically a
        //      first-install with no ini yet) — fall back to detection
        //      so the user lands on a sensible default instead of the
        //      hard-coded FrontStereo enum default. Cleared after the
        //      first run so we don't keep overriding the user's later
        //      manual radio-button picks.
        let device_changed = self.audio.device_pf != self.audio.prev_device_pf;
        let need_first_detect = !self.audio.sound_3d_from_ini;
        if device_changed || need_first_detect {
            self.audio.prev_device_pf = self.audio.device_pf.clone();
            self.audio.sound_3d_from_ini = true;
            if let Some(channels) = AudioConfig::detect_native_channels(&self.audio.device_pf) {
                let suggested = AudioConfig::recommended_sound_3d_mode(channels);
                log::info!(
                    "Audio: device '{}' reports {} channels → suggesting Sound3D mode {:?}",
                    self.audio.device_pf,
                    channels,
                    suggested
                );
                self.audio.sound_3d_mode = suggested;
            }
        }

        ui.add_space(12.0);

        // Sound3D mode
        ui.strong(t!("audio_output_mode"));
        ui.add_space(4.0);
        for mode in Sound3DMode::all() {
            ui.radio_value(&mut self.audio.sound_3d_mode, *mode, mode.label());
        }

        // Wiring guide based on selected mode
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.strong(t!("audio_wiring_required"));
        ui.add_space(4.0);

        match self.audio.sound_3d_mode {
            Sound3DMode::FrontStereo | Sound3DMode::RearStereo => {
                ui.label(t!("audio_card_stereo"));
                ui.label(format!("  {}", t!("audio_wiring_stereo_front")));
                ui.label(format!("  {}", t!("audio_wiring_stereo_no_spatial")));
            }
            Sound3DMode::SurroundRearLockbar => {
                ui.label(t!("audio_card_51"));
                ui.label(format!("  {}", t!("audio_wiring_51_front_top")));
                ui.label(format!("  {}", t!("audio_wiring_51_rear_bottom")));
                ui.label(format!("  {}", t!("audio_wiring_51_center_sub")));
                ui.label(format!("  {}", t!("audio_wiring_51_bg_separate")));
            }
            Sound3DMode::SurroundFrontLockbar => {
                ui.label(t!("audio_card_51"));
                ui.label(format!("  {}", t!("audio_wiring_51_front_bottom")));
                ui.label(format!("  {}", t!("audio_wiring_51_rear_top")));
                ui.label(format!("  {}", t!("audio_wiring_51_center_sub")));
                ui.label(format!("  {}", t!("audio_wiring_51_bg_separate")));
            }
            Sound3DMode::SsfLegacy | Sound3DMode::SsfNew => {
                ui.label(t!("audio_card_71"));
                ui.add_space(4.0);
                egui::Grid::new("wiring_grid")
                    .striped(true)
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        ui.strong(t!("audio_wiring_col_output"));
                        ui.strong(t!("audio_wiring_col_channel"));
                        ui.strong(t!("audio_wiring_col_connection"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_green"));
                        ui.label(t!("audio_wiring_71_green_ch"));
                        ui.label(t!("audio_wiring_71_green_conn"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_black"));
                        ui.label(t!("audio_wiring_71_black_ch"));
                        ui.label(t!("audio_wiring_71_black_conn"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_grey"));
                        ui.label(t!("audio_wiring_71_grey_ch"));
                        ui.label(t!("audio_wiring_71_grey_conn"));
                        ui.end_row();

                        ui.label(t!("audio_wiring_71_orange"));
                        ui.label(t!("audio_wiring_71_orange_ch"));
                        ui.label(t!("audio_wiring_71_orange_conn"));
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.label(t!("audio_wiring_note"));
            }
        }

        ui.add_space(12.0);

        // Volumes
        ui.strong(t!("audio_volumes"));
        ui.add_space(4.0);
        egui::Grid::new("audio_volumes")
            .min_col_width(150.0)
            .show(ui, |ui| {
                ui.label(t!("audio_music_volume"));
                ui.add(egui::Slider::new(&mut self.audio.music_volume, 0..=100).suffix("%"));
                ui.end_row();

                ui.label(t!("audio_sound_volume"));
                ui.add(egui::Slider::new(&mut self.audio.sound_volume, 0..=100).suffix("%"));
                ui.end_row();
            });

        ui.add_space(12.0);

        // === Tests audio ===
        ui.strong(t!("audio_test_music"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let music_label = if self.audio.music_looping {
                "[Stop]"
            } else {
                "[Play]"
            };
            if ui.button(music_label).clicked() {
                self.audio.music_looping = !self.audio.music_looping;
                if let Some(tx) = &self.audio_cmd_tx {
                    if self.audio.music_looping {
                        self.audio.music_pan = 0.0;
                        let _ = tx.send(AudioCommand::StartMusic {
                            path: "music.ogg".to_string(),
                        });
                    } else {
                        let _ = tx.send(AudioCommand::StopMusic);
                    }
                }
            }
            if self.audio.music_looping {
                ui.label(t!("audio_pan"));
                let pan_slider = egui::Slider::new(&mut self.audio.music_pan, -1.0..=1.0)
                    .custom_formatter(|v, _| {
                        if v < -0.8 {
                            t!("audio_pan_left").to_string()
                        } else if v <= 0.2 {
                            t!("audio_pan_center").to_string()
                        } else {
                            t!("audio_pan_right").to_string()
                        }
                    });
                let response = ui.add_sized([ui.available_width(), 20.0], pan_slider);
                if response.changed() {
                    // Fire on every value change (mid-drag, keyboard, click on
                    // rail) instead of only on `drag_stopped`. The audio thread
                    // pushes the music in short chunks and smooths the applied
                    // pan toward the last-received target, so the music keeps
                    // playing continuously and the balance follows the cursor
                    // in real time without any restart-from-zero pop.
                    if let Some(tx) = &self.audio_cmd_tx {
                        let _ = tx.send(AudioCommand::SetMusicPan {
                            pan: self.audio.music_pan,
                        });
                    }
                }
            }
        });

        ui.add_space(12.0);
        ui.strong(t!("audio_test_speakers"));
        ui.add_space(4.0);
        ui.label(t!("audio_speakers_hint"));
        ui.add_space(4.0);

        // Layout adapts to the selected Sound3D mode:
        //   - Surround / SSF (4+ speakers): full square with Top/Bottom
        //     Left/Right buttons + ball sweeps top→bottom and left→right.
        //   - Stereo (2 channels): only Left / Right buttons + a single
        //     ball sweep left→right. The front↔lockbar dimension doesn't
        //     exist so hiding those buttons keeps the UI honest.
        let stereo = self.audio.sound_3d_mode.is_stereo();
        let btn_w = 140.0;
        let btn_h = 30.0;
        let gap = 20.0;

        // Row 1: Left / Right (labelled "Top" in surround mode where a
        // symmetric bottom pair exists below).
        let (left_label, right_label) = if stereo {
            (t!("audio_left").to_string(), t!("audio_right").to_string())
        } else {
            (
                t!("audio_top_left").to_string(),
                t!("audio_top_right").to_string(),
            )
        };
        ui.horizontal(|ui| {
            ui.add_space(gap);
            if ui
                .add_sized([btn_w, btn_h], egui::Button::new(left_label))
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayOnSpeaker {
                        path: "ball_roll.ogg".to_string(),
                        target: audio::SpeakerTarget::TopLeft,
                    });
                }
            }
            ui.add_space(gap * 2.0);
            if ui
                .add_sized([btn_w, btn_h], egui::Button::new(right_label))
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayOnSpeaker {
                        path: "ball_roll.ogg".to_string(),
                        target: audio::SpeakerTarget::TopRight,
                    });
                }
            }
        });

        // Row 2: Ball test buttons (centered).
        // Top→bottom test only makes sense with a front/rear pair.
        ui.add_space(4.0);
        if !stereo {
            ui.horizontal(|ui| {
                ui.add_space(gap + btn_w / 2.0);
                if ui
                    .add_sized(
                        [btn_w + gap, btn_h],
                        egui::Button::new(t!("audio_ball_top_bottom").to_string()),
                    )
                    .clicked()
                {
                    if let Some(tx) = &self.audio_cmd_tx {
                        let _ = tx.send(AudioCommand::PlayBallSequence {
                            path: "ball_roll.ogg".to_string(),
                            from: audio::SpeakerTarget::TopBoth,
                            to: audio::SpeakerTarget::BottomBoth,
                            hold_start_ms: 1500,
                            fade_ms: 3000,
                            hold_end_ms: 1500,
                        });
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(gap + btn_w / 2.0);
            if ui
                .add_sized(
                    [btn_w + gap, btn_h],
                    egui::Button::new(t!("audio_ball_left_right").to_string()),
                )
                .clicked()
            {
                if let Some(tx) = &self.audio_cmd_tx {
                    let _ = tx.send(AudioCommand::PlayBallSequence {
                        path: "ball_roll.ogg".to_string(),
                        from: audio::SpeakerTarget::LeftBoth,
                        to: audio::SpeakerTarget::RightBoth,
                        hold_start_ms: 1500,
                        fade_ms: 3000,
                        hold_end_ms: 1500,
                    });
                }
            }
        });
        ui.add_space(4.0);

        // Row 3: Bottom Left / Bottom Right — surround only.
        if !stereo {
            ui.horizontal(|ui| {
                ui.add_space(gap);
                if ui
                    .add_sized(
                        [btn_w, btn_h],
                        egui::Button::new(t!("audio_bottom_left").to_string()),
                    )
                    .clicked()
                {
                    if let Some(tx) = &self.audio_cmd_tx {
                        let _ = tx.send(AudioCommand::PlayOnSpeaker {
                            path: "ball_roll.ogg".to_string(),
                            target: audio::SpeakerTarget::BottomLeft,
                        });
                    }
                }
                ui.add_space(gap * 2.0);
                if ui
                    .add_sized(
                        [btn_w, btn_h],
                        egui::Button::new(t!("audio_bottom_right").to_string()),
                    )
                    .clicked()
                {
                    if let Some(tx) = &self.audio_cmd_tx {
                        let _ = tx.send(AudioCommand::PlayOnSpeaker {
                            path: "ball_roll.ogg".to_string(),
                            target: audio::SpeakerTarget::BottomRight,
                        });
                    }
                }
            });
        }
    }
}
