//! Let the VPX binary itself (re)write `VPinballX.ini` with its current
//! schema: the complete documented file on a fresh machine, or the existing
//! one migrated while preserving every value. PinReady then only layers its
//! own values on top.
//!
//! The trigger is `VPX -h`. VPX loads and saves its settings in
//! `VPApp::InitInstance`, before it runs any command, so the help command is
//! the cheapest run that ends with a saved ini. Two things make it more than
//! a plain spawn:
//!
//! - It is skipped when the ini was already saved by this very build: VPX
//!   stamps `[Version] VPinball` on every save, and a run of the same build
//!   would only write the same file again (any table launched from the
//!   launcher keeps it current the same way).
//! - On Windows the help command ends in a modal `MessageBox` ("Visual
//!   Pinball Usage") that only a click dismisses, and the process exits
//!   after that click. So VPX is started on a desktop of its own that nobody
//!   looks at, and stopped as soon as the rewritten file has settled. Waiting
//!   for the exit, as before, kept the box on screen in front of the wizard
//!   for as long as nobody clicked it.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

/// Give up (and stop VPX) after this long.
const TIMEOUT: Duration = Duration::from_secs(20);
/// How often the ini is looked at while VPX runs.
const POLL: Duration = Duration::from_millis(150);
/// Consecutive polls the rewritten ini must hold still before VPX may be
/// stopped: VPX writes the file in place, so stopping it on the very first
/// sign of a change could cut the write short.
const SETTLED_POLLS: u32 = 2;

/// The value VPX writes to `[Version] VPinball` for the build behind a
/// PinReady version tag (`v10.8.1-5430-ac957dacb` → `10815430`): major,
/// minor, revision and git revision count, concatenated
/// (`VP_VERSION_STRING_DIGITS` in VPX's `vpversion.h`). `None` when the tag
/// does not carry a git revision count, so the build cannot be pinned down.
pub fn ini_version_digits(tag: &str) -> Option<String> {
    let tag = tag.trim();
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = tag.split('-');
    let base = parts.next()?;
    let rev = parts.next()?;
    let numbers: Vec<&str> = base.split('.').collect();
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if numbers.len() != 3 || !numbers.iter().all(|n| all_digits(n)) || !all_digits(rev) {
        return None;
    }
    Some(format!("{}{}{}{rev}", numbers[0], numbers[1], numbers[2]))
}

/// True when the ini was last saved by the build `exe_tag` names, i.e. it
/// already has that build's schema and a rewrite would change nothing.
pub fn already_current(ini_version: Option<&str>, exe_tag: Option<&str>) -> bool {
    match (
        ini_version.map(str::trim),
        exe_tag.and_then(ini_version_digits),
    ) {
        (Some(saved), Some(digits)) => saved == digits,
        _ => false,
    }
}

/// What identifies one state of the ini on disk.
type FileStamp = (SystemTime, u64);

fn stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Tells when VPX has rewritten the ini and the file has held still since.
struct RewriteWatch {
    before: Option<FileStamp>,
    last: Option<FileStamp>,
    still: u32,
}

impl RewriteWatch {
    fn new(before: Option<FileStamp>) -> Self {
        Self {
            before,
            last: before,
            still: 0,
        }
    }

    /// Feed the file's current stamp; true once it differs from the one
    /// before the run and has not moved for `SETTLED_POLLS` polls.
    fn observe(&mut self, now: Option<FileStamp>) -> bool {
        if now.is_none() || now == self.before {
            self.last = now;
            self.still = 0;
            return false;
        }
        if now == self.last {
            self.still += 1;
        } else {
            self.last = now;
            self.still = 0;
        }
        self.still >= SETTLED_POLLS
    }
}

/// A running `VPX -h`.
trait Writer {
    /// `Some(description)` once the process has exited by itself.
    fn exited(&mut self) -> Option<String>;
    /// Stop the process if it is still running, and reap it.
    fn stop(&mut self);
}

#[cfg(not(target_os = "windows"))]
impl Writer for std::process::Child {
    fn exited(&mut self) -> Option<String> {
        match self.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            _ => None,
        }
    }

    fn stop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

#[cfg(not(target_os = "windows"))]
fn spawn(exe: &Path) -> std::io::Result<std::process::Child> {
    // Standalone builds print the help and exit by themselves, with no window.
    std::process::Command::new(exe)
        .arg("-h")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

#[cfg(target_os = "windows")]
fn spawn(exe: &Path) -> std::io::Result<hidden_desktop::HiddenProcess> {
    hidden_desktop::HiddenProcess::spawn(exe, "-h")
}

/// Windows: run a GUI process on a desktop of its own, created for it and
/// never switched to, so whatever window it opens — here the help
/// `MessageBox` — is never seen.
#[cfg(target_os = "windows")]
mod hidden_desktop {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, WAIT_OBJECT_0};
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, HDESK,
    };
    use windows::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
        STARTUPINFOW, TerminateProcess, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    pub struct HiddenProcess {
        desktop: HDESK,
        info: PROCESS_INFORMATION,
    }

    impl HiddenProcess {
        pub fn spawn(exe: &Path, args: &str) -> std::io::Result<Self> {
            let name = format!("PinReady-vpx-ini-{}", std::process::id());
            let mut desktop_name = wide(std::ffi::OsStr::new(&name));
            let exe_w = wide(exe.as_os_str());
            // CreateProcessW may write into the command line buffer.
            let mut command_line = wide(std::ffi::OsStr::new(&format!(
                "\"{}\" {args}",
                exe.display()
            )));
            unsafe {
                let desktop = CreateDesktopW(
                    PCWSTR(desktop_name.as_ptr()),
                    PCWSTR::null(),
                    None,
                    DESKTOP_CONTROL_FLAGS(0),
                    GENERIC_ALL.0,
                    None,
                )
                .map_err(std::io::Error::other)?;
                let startup = STARTUPINFOW {
                    cb: std::mem::size_of::<STARTUPINFOW>() as u32,
                    // A bare name (no backslash) is a desktop of this
                    // process's own window station.
                    lpDesktop: PWSTR(desktop_name.as_mut_ptr()),
                    ..Default::default()
                };
                let mut info = PROCESS_INFORMATION::default();
                if let Err(e) = CreateProcessW(
                    PCWSTR(exe_w.as_ptr()),
                    Some(PWSTR(command_line.as_mut_ptr())),
                    None,
                    None,
                    false,
                    PROCESS_CREATION_FLAGS(0),
                    None,
                    PCWSTR::null(),
                    &startup,
                    &mut info,
                ) {
                    let _ = CloseDesktop(desktop);
                    return Err(std::io::Error::other(e));
                }
                Ok(Self { desktop, info })
            }
        }
    }

    impl super::Writer for HiddenProcess {
        fn exited(&mut self) -> Option<String> {
            unsafe {
                if WaitForSingleObject(self.info.hProcess, 0) != WAIT_OBJECT_0 {
                    return None;
                }
                let mut code = 0u32;
                let _ = GetExitCodeProcess(self.info.hProcess, &mut code);
                Some(format!("exit code: {code}"))
            }
        }

        fn stop(&mut self) {
            unsafe {
                if WaitForSingleObject(self.info.hProcess, 0) != WAIT_OBJECT_0 {
                    let _ = TerminateProcess(self.info.hProcess, 1);
                    let _ = WaitForSingleObject(self.info.hProcess, 5000);
                }
            }
        }
    }

    impl Drop for HiddenProcess {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.info.hThread);
                let _ = CloseHandle(self.info.hProcess);
                // The desktop goes away with its last handle and its last
                // thread, so this is safe even if VPX were still running.
                let _ = CloseDesktop(self.desktop);
            }
        }
    }
}

/// Run the rewrite (blocking, call it off the UI thread). True when the ini
/// was rewritten by VPX and exists afterwards; false when it was skipped or
/// failed, in which case the file on disk is the one PinReady already holds.
pub fn run(exe: &Path, ini: &Path) -> bool {
    let exe_tag = crate::updater::query_vpx_version(&exe.to_string_lossy());
    let saved_by = crate::config::VpxConfig::load(Some(ini))
        .ok()
        .and_then(|c| c.get("Version", "VPinball"));
    if ini.is_file() && already_current(saved_by.as_deref(), exe_tag.as_deref()) {
        log::info!(
            "{} was last saved by this VPX build ({}): schema already current, not running `-h`",
            ini.display(),
            exe_tag.as_deref().unwrap_or_default()
        );
        return false;
    }

    log::info!(
        "Running `{} -h` to (re)write {} with VPX's current schema (ini saved by {}, binary is {})",
        exe.display(),
        ini.display(),
        saved_by.as_deref().unwrap_or("nothing"),
        exe_tag.as_deref().unwrap_or("unknown")
    );
    let before = stamp(ini);
    let mut writer = match spawn(exe) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("Could not launch VPX for ini rewrite: {e}");
            return false;
        }
    };

    let mut watch = RewriteWatch::new(before);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = writer.exited() {
            log::info!("VPX ini rewrite: process exited ({status})");
            break;
        }
        if watch.observe(stamp(ini)) {
            log::info!("VPX ini rewrite: file written and settled, stopping VPX");
            break;
        }
        if Instant::now() > deadline {
            log::warn!(
                "VPX ini rewrite timed out after {}s, stopping VPX",
                TIMEOUT.as_secs()
            );
            break;
        }
        std::thread::sleep(POLL);
    }
    writer.stop();

    let ok = ini.exists();
    log::info!(
        "VPX ini rewrite {}",
        if ok { "succeeded" } else { "failed" }
    );
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_digits_follow_vpx_stamp() {
        // VPX 10.8.1 rev 5430 stamps `VPinball = 10815430`.
        assert_eq!(
            ini_version_digits("v10.8.1-5430-ac957dacb").as_deref(),
            Some("10815430")
        );
        assert_eq!(
            ini_version_digits("10.8.1-5436").as_deref(),
            Some("10815436")
        );
    }

    #[test]
    fn version_digits_need_a_git_revision() {
        assert_eq!(ini_version_digits("v10.8.1"), None);
        assert_eq!(ini_version_digits(""), None);
        assert_eq!(ini_version_digits("v10.8-5430-abc"), None);
        assert_eq!(ini_version_digits("vX.8.1-5430"), None);
        assert_eq!(ini_version_digits("v10.8.1-abc"), None);
    }

    #[test]
    fn current_only_when_saved_by_the_same_build() {
        let tag = Some("v10.8.1-5430-ac957dacb");
        assert!(already_current(Some("10815430"), tag));
        assert!(already_current(Some(" 10815430 "), tag));
        assert!(!already_current(Some("10815414"), tag));
        assert!(!already_current(Some(""), tag));
        assert!(!already_current(None, tag));
        // Unknown binary version: never assume, rewrite.
        assert!(!already_current(Some("10815430"), None));
        assert!(!already_current(Some("10815430"), Some("v10.8.1")));
    }

    fn at(secs: u64, len: u64) -> Option<FileStamp> {
        Some((SystemTime::UNIX_EPOCH + Duration::from_secs(secs), len))
    }

    #[test]
    fn watch_waits_for_a_change_then_for_it_to_settle() {
        let mut w = RewriteWatch::new(at(1, 100));
        assert!(!w.observe(at(1, 100)), "untouched file");
        assert!(!w.observe(at(2, 50)), "write in progress");
        assert!(!w.observe(at(2, 90)), "still growing");
        assert!(!w.observe(at(2, 90)), "one still poll is not enough");
        assert!(w.observe(at(2, 90)), "settled");
    }

    #[test]
    fn watch_sees_a_fresh_file_appear() {
        let mut w = RewriteWatch::new(None);
        assert!(!w.observe(None));
        assert!(!w.observe(at(5, 4000)));
        assert!(!w.observe(at(5, 4000)));
        assert!(w.observe(at(5, 4000)));
    }

    #[test]
    fn watch_ignores_a_file_that_vanishes_mid_run() {
        let mut w = RewriteWatch::new(at(1, 100));
        assert!(!w.observe(at(2, 100)));
        assert!(!w.observe(None));
        assert!(!w.observe(at(2, 100)));
        assert!(!w.observe(at(2, 100)));
        assert!(w.observe(at(2, 100)));
    }
}
