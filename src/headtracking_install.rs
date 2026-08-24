//! One-click install of the headtracking plugin + demo.
//!
//! Wired to the outputs (cab peripherals) page: for cabs with a Kinect or
//! a camera on the backglass/topper. Downloads the latest release of
//! Le-Syl21/headtracking (`releases/latest` — the `-preview` channel
//! publishes as the Latest release by policy), extracts the whole ZIP into
//! `<vpx_install>/headtracking-tools/` (so the demo keeps its `setup/`
//! driver installer next to it and can be relaunched any time), copies the
//! plugin into `<vpx_install>/plugins/headtracking/`, enables it in
//! `VPinballX.ini`, then launches the demo so the user can install the
//! camera drivers (Windows WinUSB / Linux udev banner) and see themselves
//! tracked before ever starting VPX.

use anyhow::{bail, Context, Result};
use crossbeam_channel::{Receiver, Sender};
use std::path::{Path, PathBuf};

const HT_REPO: &str = "Le-Syl21/headtracking";

/// Release-asset suffix for the current platform. `None` = headtracking
/// publishes no build for it (macOS Intel, Windows ARM) — the button greys
/// out with an explanation instead of failing at download time.
pub fn platform_asset_suffix() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64.tar.gz")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-aarch64.tar.gz")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x86_64.zip")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-aarch64.tar.gz")
    } else {
        None
    }
}

fn plugin_lib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "headtracking.dll"
    } else if cfg!(target_os = "macos") {
        "libheadtracking.dylib"
    } else {
        "libheadtracking.so"
    }
}

fn demo_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "headtracking-demo.exe"
    } else {
        "headtracking-demo"
    }
}

fn tools_dir(vpx_install_dir: &Path) -> PathBuf {
    vpx_install_dir.join("headtracking-tools")
}

fn plugin_dir(vpx_install_dir: &Path) -> PathBuf {
    vpx_install_dir.join("plugins").join("headtracking")
}

/// Version of the installed plugin, read from its `plugin.cfg`.
/// `None` = not installed.
pub fn installed_version(vpx_install_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(plugin_dir(vpx_install_dir).join("plugin.cfg")).ok()?;
    text.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("version")?.trim_start();
        let v = rest.strip_prefix('=')?.trim().trim_matches('"');
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// The release the button would install, as its bare version (`0.0.30`,
/// tags carry a `v` prefix the plugin.cfg does not).
fn latest_release() -> Result<serde_json::Value> {
    let url = format!("https://api.github.com/repos/{HT_REPO}/releases/latest");
    let response = ureq::get(&url)
        .header("User-Agent", "PinReady")
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .context("query headtracking release")?;
    let body = response.into_body().read_to_string()?;
    serde_json::from_str(&body).context("parse release JSON")
}

/// Strip the `v` a git tag carries so `v0.0.30` and the plugin.cfg's
/// `0.0.30` compare equal.
pub fn bare_version(tag: &str) -> &str {
    tag.trim().strip_prefix('v').unwrap_or(tag.trim())
}

/// Fire-and-forget lookup of the available version, so the button can
/// name where the update *goes*, not just where it comes from. Sends
/// exactly one message: the version, or `None` when offline.
pub fn spawn_latest_version() -> Receiver<Option<String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("pinready-ht-version".into())
        .spawn(move || {
            let version = latest_release().ok().and_then(|json| {
                json["tag_name"]
                    .as_str()
                    .map(|t| bare_version(t).to_string())
            });
            let _ = tx.send(version);
        })
        .expect("spawn headtracking version thread");
    rx
}

#[derive(Debug, Clone)]
pub enum HtEvent {
    /// Translation key of the current step (checking / downloading /
    /// installing) — the UI renders `t!(key)`.
    Status(&'static str),
    Done {
        tag: String,
    },
    Error(String),
}

/// Fire-and-forget install worker. Progress arrives on the receiver;
/// the final event is always `Done` or `Error`.
pub fn spawn_install(vpx_install_dir: PathBuf) -> Receiver<HtEvent> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("pinready-ht-install".into())
        .spawn(move || match install(&vpx_install_dir, &tx) {
            Ok(tag) => {
                let _ = tx.send(HtEvent::Done { tag });
            }
            Err(e) => {
                let _ = tx.send(HtEvent::Error(format!("{e:#}")));
            }
        })
        .expect("spawn headtracking install thread");
    rx
}

fn install(vpx_install_dir: &Path, tx: &Sender<HtEvent>) -> Result<String> {
    let Some(suffix) = platform_asset_suffix() else {
        bail!("no headtracking build for this platform");
    };

    let _ = tx.send(HtEvent::Status("ht_status_checking"));
    let json = latest_release()?;
    let tag = json["tag_name"]
        .as_str()
        .context("missing tag_name")?
        .to_string();
    let asset = json["assets"]
        .as_array()
        .context("missing assets")?
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| n.starts_with("headtracking-") && n.ends_with(suffix))
        })
        .with_context(|| format!("no asset ending in {suffix} on release {tag}"))?;
    let asset_url = asset["browser_download_url"]
        .as_str()
        .context("missing download URL")?;

    let _ = tx.send(HtEvent::Status("ht_status_downloading"));
    let tools = tools_dir(vpx_install_dir);
    std::fs::create_dir_all(&tools).context("create headtracking-tools dir")?;
    let tmp = tools.join(if suffix.ends_with(".zip") {
        ".ht_download.zip"
    } else {
        ".ht_download.tar.gz"
    });
    {
        let response = ureq::get(asset_url)
            .header("User-Agent", "PinReady")
            .call()
            .context("download headtracking release")?;
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(&tmp).context("create download temp file")?;
        std::io::copy(&mut reader, &mut file).context("write download")?;
    }

    let _ = tx.send(HtEvent::Status("ht_status_installing"));
    let extract_result = if suffix.ends_with(".zip") {
        crate::updater::extract_zip(&tmp, &tools)
    } else {
        crate::updater::extract_tar_gz(&tmp, &tools)
    };
    let _ = std::fs::remove_file(&tmp);
    extract_result.context("extract headtracking release")?;

    // Plugin files → <vpx_install>/plugins/headtracking/. The demo and its
    // setup/ folder stay in headtracking-tools.
    let dest = plugin_dir(vpx_install_dir);
    std::fs::create_dir_all(&dest).context("create plugins/headtracking")?;
    for name in [plugin_lib_name(), "plugin.cfg"] {
        let src = tools.join(name);
        if !src.is_file() {
            bail!("release archive is missing {name}");
        }
        std::fs::copy(&src, dest.join(name)).with_context(|| format!("copy {name}"))?;
    }

    // Archives extracted by the zip path lose the exec bit; harmless
    // elsewhere.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let demo = tools.join(demo_bin_name());
        if demo.is_file() {
            let _ = std::fs::set_permissions(&demo, std::fs::Permissions::from_mode(0o755));
        }
    }

    // Enable the plugin in the ini — installing without enabling would
    // leave the user staring at a table that doesn't move.
    match crate::config::VpxConfig::load(None) {
        Ok(mut ini) => {
            ini.set("Plugin.HeadTracking", "Enable", "1");
            // ...and put the view set on the Window projection, the only one
            // that reads the eye position the plugin feeds VPX. Same reason as
            // above: an installed-but-inert feature just looks broken.
            // Absent key = VPX's own default (0, Desktop); guessing "cabinet"
            // here would write the projection into a view set VPX never reads.
            let view_mode = ini.get_i32("Player", "BGSet").unwrap_or(0);
            ini.set_window_projection(view_mode);
            if let Err(e) = ini.save() {
                log::warn!("headtracking: enabling in ini failed: {e:#}");
            }
        }
        Err(e) => log::warn!("headtracking: ini not loadable, plugin not auto-enabled: {e:#}"),
    }

    Ok(tag)
}

/// Launch the demo (non-blocking) from the tools dir so its `setup/`
/// driver installer resolves next to the binary.
pub fn launch_demo(vpx_install_dir: &Path) -> Result<()> {
    let tools = tools_dir(vpx_install_dir);
    let demo = tools.join(demo_bin_name());
    if !demo.is_file() {
        bail!("demo not found at {}", demo.display());
    }
    std::process::Command::new(&demo)
        .current_dir(&tools)
        .spawn()
        .with_context(|| format!("launch {}", demo.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_matches_release_matrix() {
        // On every platform PinReady builds for, the suffix is either a
        // real release asset suffix or None (greyed button) — never a
        // panic.
        let s = platform_asset_suffix();
        if let Some(s) = s {
            assert!(s.ends_with(".zip") || s.ends_with(".tar.gz"));
        }
    }

    #[test]
    fn installed_version_reads_plugin_cfg() {
        let dir = std::env::temp_dir().join(format!("pinready-ht-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plug = dir.join("plugins/headtracking");
        std::fs::create_dir_all(&plug).unwrap();
        std::fs::write(
            plug.join("plugin.cfg"),
            "[plugin]\nid=HeadTracking\nversion = 0.0.30\n",
        )
        .unwrap();
        assert_eq!(installed_version(&dir).as_deref(), Some("0.0.30"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_version_absent_when_not_installed() {
        let dir =
            std::env::temp_dir().join(format!("pinready-ht-test-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(installed_version(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
