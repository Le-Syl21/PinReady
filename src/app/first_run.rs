//! First launch: look for a newer PinReady before anything else is set up.
//!
//! Someone who installs PinReady gets the version they downloaded, which may
//! already be behind a fix. On the very first launch (no `VPinballX.ini`, the
//! wizard never finished) the window starts with a short check, and offers the
//! update before the wizard opens. No network, a slow network or no newer
//! version: the wizard opens as it always did, after a few seconds at most.

use super::*;
use std::time::Duration;

/// How long the first launch waits for GitHub before opening the wizard.
const GATE_TIMEOUT: Duration = Duration::from_secs(4);

/// What the first-launch window shows right now.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum GateStep {
    /// Still asking GitHub.
    Checking,
    /// A newer version exists: update or carry on.
    Offer,
    /// The update is downloading.
    Updating,
    /// Nothing to say: open the wizard.
    Pass,
}

/// The step for the time spent, whether the answer is still awaited, whether
/// a newer version is known and whether its download has started.
pub(super) fn gate_step(
    elapsed: Duration,
    check_pending: bool,
    update_available: bool,
    updating: bool,
) -> GateStep {
    if updating {
        GateStep::Updating
    } else if update_available {
        GateStep::Offer
    } else if check_pending && elapsed < GATE_TIMEOUT {
        GateStep::Checking
    } else {
        GateStep::Pass
    }
}

/// Is this the first launch: the wizard never finished and no VPX settings file exists?
pub(super) fn is_first_run(wizard_completed: bool, ini_present: bool) -> bool {
    !wizard_completed && !ini_present
}

impl App {
    /// Draws the first-launch window while it is needed. Returns `true` when it
    /// took the whole window, `false` when the wizard can be drawn.
    pub(super) fn render_first_run_gate(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(start) = self.first_run_gate else {
            return false;
        };
        let ctx = ui.ctx().clone();
        self.process_pinready_update_check(&ctx);
        let step = gate_step(
            start.elapsed(),
            self.pinready_update_check_rx.is_some(),
            self.pinready_latest_release.is_some(),
            self.pinready_updating,
        );
        if step == GateStep::Pass {
            self.first_run_gate = None;
            return false;
        }

        let release = self.pinready_latest_release.clone();
        let mut carry_on = false;
        let mut update = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.1).max(16.0));
                theme::dmd_wordmark(ui, 3.0);
                ui.add_space(24.0);
                match step {
                    GateStep::Checking => {
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(t!("first_run_update_checking").to_string());
                        ctx.request_repaint_after(Duration::from_millis(100));
                    }
                    GateStep::Offer => {
                        if let Some(release) = &release {
                            ui.heading(t!("first_run_update_title").to_string());
                            ui.add_space(8.0);
                            let text = t!(
                                "first_run_update_text",
                                tag = release.tag.as_str(),
                                current = updater::CURRENT_PINREADY_VERSION
                            )
                            .to_string();
                            // A centred column has no width of its own: give the
                            // text one, so it wraps instead of running off both sides.
                            let width = (ui.available_width() - 48.0).clamp(120.0, 520.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 0.0),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    ui.add(egui::Label::new(text).wrap());
                                },
                            );
                            ui.add_space(16.0);
                            let label = t!("pinready_update_button", tag = release.tag.as_str())
                                .to_string();
                            if ui.button(egui::RichText::new(label).strong()).clicked() {
                                update = true;
                            }
                            ui.add_space(6.0);
                            if ui
                                .button(t!("first_run_update_later").to_string())
                                .clicked()
                            {
                                carry_on = true;
                            }
                            if let Some(err) = self.pinready_update_error.clone() {
                                ui.add_space(8.0);
                                ui.colored_label(
                                    theme::pal(ui.ctx()).error,
                                    t!("pinready_update_error", msg = err.as_str()).to_string(),
                                );
                            }
                        }
                    }
                    GateStep::Updating => {
                        let (current, total) = self.pinready_update_progress;
                        let (progress, txt) = if total > 0 {
                            let pct = (current as f32 / total as f32 * 100.0) as u32;
                            (
                                current as f32 / total as f32,
                                t!(
                                    "pinready_update_progress",
                                    mb = current / (1024 * 1024),
                                    total = total / (1024 * 1024),
                                    pct = pct
                                )
                                .to_string(),
                            )
                        } else {
                            (0.0, t!("pinready_update_extracting").to_string())
                        };
                        ui.add(
                            egui::ProgressBar::new(progress)
                                .desired_width(420.0)
                                .text(txt),
                        );
                        ctx.request_repaint_after(Duration::from_millis(100));
                    }
                    GateStep::Pass => {}
                }
            });
        });
        if update && let Some(release) = release {
            self.start_pinready_download(&release);
        }
        if carry_on {
            self.first_run_gate = None;
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_launch_is_only_when_nothing_was_ever_set_up() {
        assert!(is_first_run(false, false));
        assert!(!is_first_run(true, true));
        // The wizard was finished, or a settings file already exists: not new.
        assert!(!is_first_run(true, false));
        assert!(!is_first_run(false, true));
    }

    #[test]
    fn the_wizard_never_waits_longer_than_the_timeout() {
        let s = Duration::from_secs;
        // Waiting for the answer, inside the delay.
        assert_eq!(gate_step(s(1), true, false, false), GateStep::Checking);
        // Still no answer after the delay: go on without it.
        assert_eq!(gate_step(s(5), true, false, false), GateStep::Pass);
        // Answered, nothing new (or the check failed): go on at once.
        assert_eq!(gate_step(s(0), false, false, false), GateStep::Pass);
        // A newer version is offered, even after the delay.
        assert_eq!(gate_step(s(9), false, true, false), GateStep::Offer);
        // Once the download started, the window stays until the restart.
        assert_eq!(gate_step(s(9), false, true, true), GateStep::Updating);
    }
}
