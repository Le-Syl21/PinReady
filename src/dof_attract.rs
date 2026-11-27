//! Cabinet lights while PinReady waits for a table.
//!
//! DOF is started on the ROM name `pinupmenu`, the entry the DOF Config Tool
//! ships for front ends ("PinUP Menu"): every generated DirectOutput config
//! carries it, so each cabinet plays the lights its owner already set up there
//! (static and blinking toys, button lights, addressable LED effects). The
//! steady ones light at `Init`; the animated ones answer the events PinUP
//! Popper sends on that table, which the launcher sends the same way:
//! `E800`/`E801` pulsed when the selection moves back/forward, `E805`/`E804`
//! for a page (row) back/forward, and a random `E901`-`E990` held while the
//! selection rests on a table, released for the next one.
//!
//! The library is the one the installed Visual Pinball ships in its DOF plugin
//! folder, so PinReady carries no native dependency. It runs in a child
//! process (`pinready --dof-attract <libdof>`): a C++ exception thrown by
//! libdof (a broken config) cannot cross into Rust, and a crash there must not
//! take the launcher down. The child's working directory is the folder that
//! holds `directoutputconfig/` (libdof looks there when no base path is set);
//! its `DirectOutput.log` lands there too.
//! The launcher writes one event per line on the child's standard input
//! (`E 801 1`); closing it makes the child call `Finish`, which releases the
//! devices for Visual Pinball.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The DOF Config Tool's front-end table.
pub const ATTRACT_ROM: &str = "pinupmenu";

/// Command line switch of the child process.
pub const CHILD_ARG: &str = "--dof-attract";

/// libdof in the DOF plugin folder of the Visual Pinball whose executable is
/// `vpx_exe` (`plugins/dof` beside it, `PlugIns/dof` in a macOS bundle).
pub fn find_libdof(vpx_exe: &Path) -> Option<PathBuf> {
    let dir = vpx_exe.parent()?;
    let folders = [dir.join("plugins").join("dof"), dir.join("../PlugIns/dof")];
    let names = ["libdof.so", "dof64.dll", "libdof.dylib"];
    folders
        .iter()
        .flat_map(|f| names.iter().map(move |n| f.join(n)))
        .find(|p| p.is_file())
}

/// Exported names of the four `DOF::DOF` members used: Itanium mangling
/// (Linux, macOS, MinGW) then MSVC.
const CTOR: [&str; 2] = ["_ZN3DOF3DOFC1Ev", "??0DOF@0@QEAA@XZ"];
const DTOR: [&str; 2] = ["_ZN3DOF3DOFD1Ev", "??1DOF@0@QEAA@XZ"];
const INIT: [&str; 2] = ["_ZN3DOF3DOF4InitEPKcS2_", "?Init@DOF@1@QEAAXPEBD0@Z"];
const FINISH: [&str; 2] = ["_ZN3DOF3DOF6FinishEv", "?Finish@DOF@1@QEAAXXZ"];
const DATA: [&str; 2] = [
    "_ZN3DOF3DOF11DataReceiveEcii",
    "?DataReceive@DOF@1@QEAAXDHH@Z",
];

/// Popper's events on the `pinupmenu` table.
pub const EV_PRIOR: u16 = 800;
pub const EV_NEXT: u16 = 801;
pub const EV_PAGE_NEXT: u16 = 804;
pub const EV_PAGE_PRIOR: u16 = 805;
/// Range of the random effect held while a table is selected.
pub const RANDOM_FIRST: u16 = 901;
pub const RANDOM_LAST: u16 = 990;

type Method0 = unsafe extern "C" fn(*mut u8);
type DataFn = unsafe extern "C" fn(*mut u8, std::ffi::c_char, i32, i32);
type InitFn = unsafe extern "C" fn(*mut u8, *const std::ffi::c_char, *const std::ffi::c_char);

fn open(lib: &Path) -> Result<libloading::Library, libloading::Error> {
    // The library's own dependencies sit beside it.
    #[cfg(windows)]
    unsafe {
        libloading::os::windows::Library::load_with_flags(
            lib,
            libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH,
        )
        .map(Into::into)
    }
    #[cfg(not(windows))]
    unsafe {
        libloading::Library::new(lib)
    }
}

fn symbol<T: Copy>(lib: &libloading::Library, names: &[&str]) -> Option<T> {
    names
        .iter()
        .find_map(|n| unsafe { lib.get::<T>(n.as_bytes()).ok().map(|s| *s) })
}

/// One event line as the launcher writes it: `E 801 1`.
fn parse_event(line: &str) -> Option<(u8, i32, i32)> {
    let mut it = line.split_ascii_whitespace();
    let kind = it.next()?.as_bytes();
    let number = it.next()?.parse().ok()?;
    let value = it.next()?.parse().ok()?;
    (kind.len() == 1 && kind[0].is_ascii_uppercase() && it.next().is_none())
        .then_some((kind[0], number, value))
}

/// Child process: start DOF on [`ATTRACT_ROM`], pass on the events read from
/// standard input until it closes, then finish.
pub fn run_child(lib_path: &Path) -> anyhow::Result<()> {
    // libdof appends to ./DirectOutput.log at each start: keep it bounded.
    let log = Path::new("DirectOutput.log");
    if std::fs::metadata(log).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::rename(log, "DirectOutput.old.log");
    }
    let lib = open(lib_path)?;
    let (Some(ctor), Some(dtor), Some(init), Some(finish)) = (
        symbol::<Method0>(&lib, &CTOR),
        symbol::<Method0>(&lib, &DTOR),
        symbol::<InitFn>(&lib, &INIT),
        symbol::<Method0>(&lib, &FINISH),
    ) else {
        anyhow::bail!("{}: DOF entry points not found", lib_path.display());
    };
    // `DOF::DOF` holds one pointer; a generous, aligned block whatever the version.
    let mut object = vec![0u64; 64];
    let this = object.as_mut_ptr().cast::<u8>();
    let table = c"";
    let rom = std::ffi::CString::new(ATTRACT_ROM)?;
    // SAFETY: the members of the class exported by the library, called on
    // one object in the order the VPX DOF plugin uses.
    unsafe {
        ctor(this);
        init(this, table.as_ptr(), rom.as_ptr());
    }
    let data = symbol::<DataFn>(&lib, &DATA);
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if let (Some(data), Some((kind, number, value))) = (data, parse_event(&line)) {
            // SAFETY: as above, on the same object, from this one thread.
            unsafe { data(this, kind as std::ffi::c_char, number, value) };
        }
    }
    unsafe {
        finish(this);
        dtor(this);
    }
    drop(object);
    Ok(())
}

/// The running child, seen from the launcher.
pub struct Attract {
    child: Child,
}

impl Attract {
    /// Start the lights with the libdof at `lib`, reading the DOF config in
    /// `<config_dir>/directoutputconfig`.
    pub fn start(lib: &Path, config_dir: &Path) -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let mut cmd = Command::new(exe);
        cmd.arg(CHILD_ARG)
            .arg(lib)
            .current_dir(config_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        match cmd.spawn() {
            Ok(child) => {
                log::info!("DOF lights started ({ATTRACT_ROM}, {})", lib.display());
                Some(Self { child })
            }
            Err(e) => {
                log::warn!("DOF lights: cannot start the helper: {e}");
                None
            }
        }
    }

    /// Send DOF event `E<number>` with `value`. A child that is gone is
    /// noticed by [`Self::ended`].
    pub fn event(&mut self, number: u16, value: u8) {
        if let Some(stdin) = self.child.stdin.as_mut() {
            let _ = writeln!(stdin, "E {number} {value}").and_then(|()| stdin.flush());
        }
    }

    /// Pulse `E<number>`: on, then off straight away, as Popper does.
    pub fn pulse(&mut self, number: u16) {
        self.event(number, 1);
        self.event(number, 0);
    }

    /// The child is gone (a broken config, a crash).
    pub fn ended(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    /// Finish DOF and wait for the devices to be released (killed after 3 s).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                log::info!("DOF lights stopped");
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        log::warn!("DOF lights: helper did not finish, killing it");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Attract {
    fn drop(&mut self) {
        if self.child.stdin.is_some() {
            self.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_event_lines() {
        assert_eq!(parse_event("E 801 1"), Some((b'E', 801, 1)));
        assert_eq!(parse_event(" E  950 0 "), Some((b'E', 950, 0)));
        assert_eq!(parse_event("E 801"), None);
        assert_eq!(parse_event("e 801 1"), None);
        assert_eq!(parse_event("E 801 1 2"), None);
        assert_eq!(parse_event("EE 801 1"), None);
    }

    #[test]
    fn finds_the_library_of_each_layout() {
        let base = std::env::temp_dir().join(format!("pinready-dof-{}", std::process::id()));
        let linux = base.join("vpx/plugins/dof");
        std::fs::create_dir_all(&linux).unwrap();
        std::fs::write(linux.join("libdof.so"), b"").unwrap();
        assert_eq!(
            find_libdof(&base.join("vpx/VPinballX_BGFX")),
            Some(linux.join("libdof.so"))
        );
        let mac = base.join("app/Contents/PlugIns/dof");
        std::fs::create_dir_all(&mac).unwrap();
        std::fs::create_dir_all(base.join("app/Contents/MacOS")).unwrap();
        std::fs::write(mac.join("libdof.dylib"), b"").unwrap();
        let found = find_libdof(&base.join("app/Contents/MacOS/VPinballX_BGFX")).unwrap();
        assert!(found.ends_with("PlugIns/dof/libdof.dylib"));
        assert_eq!(find_libdof(&base.join("none/VPinballX_BGFX")), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}
