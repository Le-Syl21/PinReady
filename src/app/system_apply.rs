// Applying the System page's choices.
//
// The page's checkboxes only record what the user wants; nothing touches the
// system until Finish. On Windows every one of these actions shells out to
// PowerShell (a `.lnk` is written through `WScript.Shell`), which takes a
// second or more each: run from a checkbox handler on the UI thread, it froze
// the window with no hint of what was going on. So Finish hands the whole set
// to a worker thread and the footer shows a status until it reports back.

use super::*;

/// What the System page asks for, captured when the user validates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SystemChoices {
    pub autostart: bool,
    pub desktop_integration: bool,
    /// Where the `.vpx` association should point; empty = PinReady only.
    pub vpx_exe_path: String,
    pub desktop_shortcut: bool,
    /// `None` when the session has no dock we can pin to: the option was
    /// hidden, so there is no choice to apply.
    pub dock_pinned: Option<bool>,
}

/// The system side of the System page, behind a trait so the apply order and
/// the "only touch what changed" rule can be tested without a desktop.
pub(super) trait SystemOps {
    fn set_autostart(&mut self, enabled: bool) -> anyhow::Result<()>;
    fn set_desktop_integration(&mut self, enabled: bool, vpx_exe: &str) -> anyhow::Result<()>;
    fn is_desktop_shortcut_installed(&mut self) -> bool;
    fn set_desktop_shortcut(&mut self, enabled: bool) -> anyhow::Result<()>;
    fn is_pinned_to_dock(&mut self) -> bool;
    fn set_pinned_to_dock(&mut self, enabled: bool) -> anyhow::Result<()>;
}

/// The real thing.
pub(super) struct HostSystem;

impl SystemOps for HostSystem {
    fn set_autostart(&mut self, enabled: bool) -> anyhow::Result<()> {
        set_autostart(enabled)
    }
    fn set_desktop_integration(&mut self, enabled: bool, vpx_exe: &str) -> anyhow::Result<()> {
        set_desktop_integration(enabled, vpx_exe)
    }
    fn is_desktop_shortcut_installed(&mut self) -> bool {
        desktop_integration::is_desktop_shortcut_installed()
    }
    fn set_desktop_shortcut(&mut self, enabled: bool) -> anyhow::Result<()> {
        desktop_integration::set_desktop_shortcut(enabled)
    }
    fn is_pinned_to_dock(&mut self) -> bool {
        desktop_integration::is_pinned_to_dock()
    }
    fn set_pinned_to_dock(&mut self, enabled: bool) -> anyhow::Result<()> {
        desktop_integration::set_pinned_to_dock(enabled)
    }
}

/// Apply `choices` and return one message per failure (already logged).
///
/// Autostart and the menu integration are written every time, as before:
/// they are idempotent and also repair an entry pointing at an old binary.
/// The desktop shortcut and the dock pin are only touched when the choice
/// differs from what is there now — they are the user's own desktop, and
/// recreating an icon they kept would move it around.
///
/// Menu integration comes before the dock pin: the pin names
/// `pinready.desktop`, which the menu integration is what installs.
pub(super) fn apply_system_choices(
    ops: &mut impl SystemOps,
    choices: &SystemChoices,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut report = |what: &str, result: anyhow::Result<()>| {
        if let Err(e) = result {
            log::error!("{what}: {e:#}");
            failures.push(format!("{what}: {e:#}"));
        }
    };

    report(
        "Failed to set autostart",
        ops.set_autostart(choices.autostart),
    );
    report(
        "Failed to set desktop integration",
        ops.set_desktop_integration(choices.desktop_integration, &choices.vpx_exe_path),
    );
    if ops.is_desktop_shortcut_installed() != choices.desktop_shortcut {
        report(
            "Desktop shortcut",
            ops.set_desktop_shortcut(choices.desktop_shortcut),
        );
    }
    if let Some(pinned) = choices.dock_pinned
        && ops.is_pinned_to_dock() != pinned
    {
        report("Dock pinning", ops.set_pinned_to_dock(pinned));
    }
    failures
}

/// Run [`apply_system_choices`] on a worker thread, which logs what it did.
/// Nobody waits for it: the wizard closes at once and the launcher opens
/// while it finishes.
pub(super) fn spawn_apply(choices: SystemChoices) {
    std::thread::spawn(move || {
        let failures = apply_system_choices(&mut HostSystem, &choices);
        if failures.is_empty() {
            log::info!("System settings applied");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every call; installed state and failures are scripted.
    #[derive(Default)]
    struct FakeSystem {
        calls: Vec<String>,
        shortcut_installed: bool,
        pinned: bool,
        fail_shortcut: bool,
    }

    impl SystemOps for FakeSystem {
        fn set_autostart(&mut self, enabled: bool) -> anyhow::Result<()> {
            self.calls.push(format!("autostart={enabled}"));
            Ok(())
        }
        fn set_desktop_integration(&mut self, enabled: bool, vpx: &str) -> anyhow::Result<()> {
            self.calls.push(format!("integration={enabled},{vpx}"));
            Ok(())
        }
        fn is_desktop_shortcut_installed(&mut self) -> bool {
            self.shortcut_installed
        }
        fn set_desktop_shortcut(&mut self, enabled: bool) -> anyhow::Result<()> {
            self.calls.push(format!("shortcut={enabled}"));
            anyhow::ensure!(!self.fail_shortcut, "PowerShell said no");
            Ok(())
        }
        fn is_pinned_to_dock(&mut self) -> bool {
            self.pinned
        }
        fn set_pinned_to_dock(&mut self, enabled: bool) -> anyhow::Result<()> {
            self.calls.push(format!("dock={enabled}"));
            Ok(())
        }
    }

    fn choices() -> SystemChoices {
        SystemChoices {
            autostart: true,
            desktop_integration: true,
            vpx_exe_path: "vpx".into(),
            desktop_shortcut: true,
            dock_pinned: Some(true),
        }
    }

    #[test]
    fn creates_what_is_missing_in_order() {
        let mut sys = FakeSystem::default();
        assert!(apply_system_choices(&mut sys, &choices()).is_empty());
        assert_eq!(
            sys.calls,
            [
                "autostart=true",
                "integration=true,vpx",
                "shortcut=true",
                "dock=true"
            ]
        );
    }

    #[test]
    fn leaves_shortcut_and_pin_alone_when_unchanged() {
        let mut sys = FakeSystem {
            shortcut_installed: true,
            pinned: true,
            ..Default::default()
        };
        apply_system_choices(&mut sys, &choices());
        assert_eq!(sys.calls, ["autostart=true", "integration=true,vpx"]);
    }

    #[test]
    fn removes_an_unticked_shortcut() {
        let mut sys = FakeSystem {
            shortcut_installed: true,
            ..Default::default()
        };
        let c = SystemChoices {
            desktop_shortcut: false,
            dock_pinned: None,
            ..choices()
        };
        apply_system_choices(&mut sys, &c);
        assert!(sys.calls.contains(&"shortcut=false".to_string()));
        assert!(!sys.calls.iter().any(|c| c.starts_with("dock")));
    }

    #[test]
    fn failure_is_reported_and_the_rest_still_runs() {
        let mut sys = FakeSystem {
            fail_shortcut: true,
            ..Default::default()
        };
        let failures = apply_system_choices(&mut sys, &choices());
        assert_eq!(failures.len(), 1);
        assert!(failures[0].starts_with("Desktop shortcut"));
        assert_eq!(sys.calls.last().map(String::as_str), Some("dock=true"));
    }
}
