//! VPinPlay in the app: the settings section, the background requests and
//! what is done with their answers. The protocol lives in `crate::vpinplay`.
//!
//! Nothing here sends anything unless the player turned the sharing on and
//! chose a name; the community ratings are only downloaded.

use super::launcher::{rel_to, unix_now};
use super::*;
use crate::vpinplay::{self, Event, PushTable};

impl App {
    /// Table data worth sharing: matched to VPS with a reliable link, and
    /// rated or played.
    fn push_candidates(&self) -> Vec<(String, PushTable)> {
        let dir = std::path::PathBuf::from(&self.tables_dir);
        self.tables
            .iter()
            .filter_map(|t| {
                let vps_id = t.vps_id.clone().filter(|_| t.vps_shareable)?;
                if t.user.rating_tenths == 0 && t.user.start_count == 0 {
                    return None;
                }
                Some((
                    rel_to(&dir, &t.path),
                    PushTable {
                        vps_id,
                        vpx_path: t.path.clone(),
                        filename: t
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        rating_tenths: t.user.rating_tenths,
                        last_run: t.user.last_run,
                        start_count: t.user.start_count,
                        run_time_secs: t.user.run_time,
                    },
                ))
            })
            .collect()
    }

    fn spawn_vpinplay(&mut self, job: impl FnOnce() -> Event + Send + 'static) {
        let (tx, rx) = crossbeam_channel::unbounded();
        // One request at a time is enough: a newer one replaces the receiver
        // of an older one, whose answer is then dropped.
        self.vpinplay_rx = Some(rx);
        std::thread::Builder::new()
            .name("pinready-vpinplay".into())
            .spawn(move || {
                let _ = tx.send(job());
            })
            .ok();
    }

    /// Download the community ratings when sharing is on and the copy we
    /// have is older than a day. Read-only.
    pub(super) fn start_ratings_refresh_if_stale(&mut self) {
        if !self.vpinplay.enabled || self.vpinplay_rx.is_some() {
            return;
        }
        let fetched_at: i64 = self
            .db
            .get_config(vpinplay::KEY_RATINGS_AT)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if !self.community.is_empty() && unix_now() - fetched_at < vpinplay::RATINGS_MAX_AGE_SECS {
            return;
        }
        let endpoint = self.vpinplay.endpoint.clone();
        self.spawn_vpinplay(move || {
            Event::Ratings(vpinplay::fetch_ratings(&endpoint).map_err(|e| format!("{e:#}")))
        });
    }

    /// A table session just ended: share that table's new numbers.
    pub(super) fn push_after_session(&mut self, rel_path: &str) {
        if !self.vpinplay.ready() {
            return;
        }
        let Some((_, table)) = self
            .push_candidates()
            .into_iter()
            .find(|(rel, _)| rel == rel_path)
        else {
            return;
        };
        let settings = self.vpinplay.clone();
        self.spawn_vpinplay(move || {
            Event::Pushed(
                vpinplay::push(&settings, &[table], unix_now()).map_err(|e| format!("{e:#}")),
            )
        });
    }

    /// Share everything rated or played ("Send my data now").
    fn push_everything(&mut self) {
        if !self.vpinplay.ready() {
            return;
        }
        let tables: Vec<PushTable> = self.push_candidates().into_iter().map(|(_, t)| t).collect();
        let settings = self.vpinplay.clone();
        self.vpinplay_status = t!("vpinplay_sending", count = tables.len()).to_string();
        self.spawn_vpinplay(move || {
            Event::Pushed(
                vpinplay::push(&settings, &tables, unix_now()).map_err(|e| format!("{e:#}")),
            )
        });
    }

    fn check_user_name(&mut self) {
        let endpoint = self.vpinplay.endpoint.clone();
        let name = self.vpinplay.user_id.clone();
        self.vpinplay_status = t!("vpinplay_checking").to_string();
        self.spawn_vpinplay(move || {
            Event::NameChecked(
                vpinplay::user_available(&endpoint, &name).map_err(|e| format!("{e:#}")),
            )
        });
    }

    /// Fold the answer of the background request in. Called every frame.
    pub(super) fn process_vpinplay(&mut self) {
        let Some(rx) = &self.vpinplay_rx else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(event) => event,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.vpinplay_rx = None;
                return;
            }
        };
        self.vpinplay_rx = None;
        match event {
            Event::Ratings(Ok(ratings)) => {
                log::info!(
                    "VPinPlay: community ratings loaded for {} tables",
                    ratings.len()
                );
                if let Err(e) = self.db.replace_community_ratings(&ratings, unix_now()) {
                    log::error!("could not keep the community ratings: {e}");
                }
                self.vpinplay_status = t!("vpinplay_ratings_ok", count = ratings.len()).to_string();
                self.community = ratings;
            }
            Event::Ratings(Err(e)) => {
                log::warn!("VPinPlay ratings: {e}");
                self.vpinplay_status = t!("vpinplay_error", error = e).to_string();
            }
            Event::Pushed(Ok(count)) => {
                self.vpinplay_status = t!("vpinplay_sent", count = count).to_string();
            }
            Event::Pushed(Err(e)) => {
                log::warn!("VPinPlay sync: {e}");
                self.vpinplay_status = t!("vpinplay_error", error = e).to_string();
            }
            Event::NameChecked(Ok(true)) => {
                self.vpinplay_status = t!("vpinplay_name_free").to_string();
            }
            Event::NameChecked(Ok(false)) => {
                self.vpinplay_status = t!("vpinplay_name_taken").to_string();
            }
            Event::NameChecked(Err(e)) => {
                self.vpinplay_status = t!("vpinplay_error", error = e).to_string();
            }
        }
    }

    /// The VPinPlay section of the last configuration page.
    pub(super) fn render_vpinplay_section(&mut self, ui: &mut egui::Ui) {
        self.process_vpinplay();
        theme::section(
            ui,
            &t!("vpinplay_heading"),
            Some(&t!("vpinplay_intro")),
            |ui| {
                ui.colored_label(theme::pal(ui.ctx()).accent_text, t!("vpinplay_recommend"));
                ui.add_space(6.0);
                let mut enabled = self.vpinplay.enabled;
                ui.horizontal(|ui| {
                    ui.checkbox(&mut enabled, t!("vpinplay_toggle"));
                    help_marker(ui, &t!("vpinplay_privacy"));
                });
                if enabled != self.vpinplay.enabled {
                    if let Err(e) = self.vpinplay.set_enabled(&self.db, enabled) {
                        log::error!("could not store the VPinPlay choice: {e}");
                    }
                    if enabled {
                        self.start_ratings_refresh_if_stale();
                    }
                }
                ui.add_space(4.0);
                // Said in full, not only in the hover help: this is public.
                ui.label(egui::RichText::new(t!("vpinplay_privacy")).weak());

                if !self.vpinplay.enabled {
                    return;
                }
                ui.add_space(8.0);
                let mut user = self.vpinplay.user_id.clone();
                let mut initials = self.vpinplay.initials.clone();
                ui.horizontal(|ui| {
                    ui.label(t!("vpinplay_user"));
                    ui.add(egui::TextEdit::singleline(&mut user).desired_width(220.0));
                    ui.label(t!("vpinplay_initials"));
                    ui.add(egui::TextEdit::singleline(&mut initials).desired_width(60.0));
                });
                if (user != self.vpinplay.user_id || initials != self.vpinplay.initials)
                    && let Err(e) = self.vpinplay.set_identity(&self.db, &user, &initials)
                {
                    log::error!("could not store the VPinPlay identity: {e}");
                }
                if !self.vpinplay.ready() {
                    ui.colored_label(theme::pal(ui.ctx()).warning, t!("vpinplay_need_identity"));
                    return;
                }
                ui.horizontal(|ui| {
                    if ui.button(t!("vpinplay_check_name")).clicked() {
                        self.check_user_name();
                    }
                    if ui.button(t!("vpinplay_sync_now")).clicked() {
                        self.push_everything();
                    }
                });
                if !self.vpinplay_status.is_empty() {
                    ui.label(&self.vpinplay_status);
                }
                if self.vpinplay_rx.is_some() {
                    ui.ctx().request_repaint();
                }
            },
        );
    }
}
