// VPinballX release checker, downloader, and installer.
// Queries a GitHub fork for releases and downloads the correct artifact
// for the current platform.

use anyhow::{bail, Context, Result};
use crossbeam_channel::Sender;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Default GitHub repository for VPinballX releases.
pub const DEFAULT_FORK_REPO: &str = "Le-Syl21/vpinball";

/// Information about a GitHub release.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
}

/// Progress updates sent during download/install.
pub enum UpdateProgress {
    Downloading(u64, u64), // (bytes_downloaded, total_bytes)
    Extracting,
    Done(PathBuf), // path to the installed executable
    Error(String),
}

// ---------------------------------------------------------------------------
// Platform detection
// ---------------------------------------------------------------------------

/// File extension of the artifact for the current platform.
fn artifact_extension() -> &'static str {
    if cfg!(target_os = "windows") {
        "zip"
    } else if cfg!(target_os = "macos") {
        "dmg"
    } else {
        "tar.gz"
    }
}

/// Returns (platform, arch) strings matching the workflow artifact names.
fn platform_arch() -> (&'static str, &'static str) {
    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else if cfg!(target_arch = "aarch64") {
        detect_arm_board()
    } else {
        "unknown"
    };

    (platform, arch)
}

/// On aarch64 Linux, distinguish between RPi and RK3588 SBCs by reading
/// the device-tree model string. Falls back to "aarch64" on other OSes.
#[cfg(target_arch = "aarch64")]
fn detect_arm_board() -> &'static str {
    // Only relevant on Linux SBCs
    if cfg!(target_os = "linux") {
        if let Ok(model) = std::fs::read_to_string("/proc/device-tree/model") {
            let lower = model.to_lowercase();
            if lower.contains("raspberry") {
                return "rpi-linux-aarch64";
            }
            if lower.contains("rk3588") || lower.contains("rock") || lower.contains("orange pi 5") {
                return "rk3588-linux-aarch64";
            }
        }
        // Generic ARM Linux — try RPi as default
        "rpi-linux-aarch64"
    } else if cfg!(target_os = "macos") {
        "arm64"
    } else {
        "aarch64"
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn detect_arm_board() -> &'static str {
    "unknown"
}

// ---------------------------------------------------------------------------
// For aarch64, the artifact name needs special handling because the board
// name is embedded differently. Override artifact_name for SBC builds.
// ---------------------------------------------------------------------------

// On aarch64 Linux, the artifact name includes the board type:
// `VPinballX_BGFX-{tag}-rpi-linux-aarch64-Release.zip`
// On other platforms, the standard format is used.
// This function handles both cases correctly because `platform_arch()`
// already returns the full board-platform-arch triple for SBCs.

// ---------------------------------------------------------------------------
// GitHub API
// ---------------------------------------------------------------------------

/// Check the latest release on a GitHub repository.
/// This is a blocking call — run it from a background thread.
pub fn check_latest_release(repo: &str) -> Result<ReleaseInfo> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");

    let response = ureq::get(&url)
        .header("User-Agent", "PinReady")
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .context("Failed to query GitHub releases")?;

    let body = response.into_body().read_to_string()?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse GitHub release JSON")?;

    let tag = json["tag_name"]
        .as_str()
        .context("Missing tag_name in release")?
        .to_string();

    // Find the matching asset for this platform by pattern
    // Assets may have different version numbers than the release tag,
    // so we match on the platform/arch/extension suffix.
    let (platform, arch) = platform_arch();
    let ext = artifact_extension();
    let suffix = format!("-{platform}-{arch}-Release.{ext}");
    let prefix = "VPinballX_BGFX-";

    let assets = json["assets"]
        .as_array()
        .context("Missing assets in release")?;

    for asset in assets {
        let name = asset["name"].as_str().unwrap_or_default();
        if name.starts_with(prefix) && name.ends_with(&suffix) {
            let url = asset["browser_download_url"]
                .as_str()
                .context("Missing download URL")?
                .to_string();
            let size = asset["size"].as_u64().unwrap_or(0);

            return Ok(ReleaseInfo {
                tag,
                asset_name: name.to_string(),
                asset_url: url,
                asset_size: size,
            });
        }
    }

    bail!(
        "No matching asset for this platform (expected: {prefix}*{suffix}). \
         Available: {}",
        assets
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ---------------------------------------------------------------------------
// Download & install
// ---------------------------------------------------------------------------

/// Default installation directory for VPinballX.
pub fn default_install_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("VPinballX")
}

/// Name of the main VPinballX executable for the current platform.
pub fn vpx_executable_name() -> &'static str {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "VPinballX_BGFX64.exe"
        } else {
            "VPinballX_BGFX.exe"
        }
    } else {
        "VPinballX_BGFX"
    }
}

/// Download a release asset and extract it into `install_dir`.
/// Sends progress updates through `progress_tx`.
/// This is a blocking call — run it from a background thread.
pub fn download_and_install(
    release: &ReleaseInfo,
    install_dir: &Path,
    progress_tx: Sender<UpdateProgress>,
) -> Result<()> {
    // Download to a temp file
    let tmp_path = install_dir
        .parent()
        .unwrap_or(install_dir)
        .join(".vpx_download.zip");

    // Ensure parent dir exists
    if let Some(parent) = tmp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let result = download_and_install_inner(release, install_dir, &tmp_path, &progress_tx);

    // Clean up temp file
    let _ = std::fs::remove_file(&tmp_path);

    result
}

fn download_and_install_inner(
    release: &ReleaseInfo,
    install_dir: &Path,
    tmp_path: &Path,
    progress_tx: &Sender<UpdateProgress>,
) -> Result<()> {
    // Download
    let response = ureq::get(&release.asset_url)
        .header("User-Agent", "PinReady")
        .header("Accept", "application/octet-stream")
        .call()
        .context("Failed to download release asset")?;

    let total = release.asset_size;
    let mut downloaded: u64 = 0;
    let mut file = std::fs::File::create(tmp_path)?;
    let mut buf = [0u8; 32768];

    let mut reader = response.into_body().into_reader();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])?;
        downloaded += n as u64;
        let _ = progress_tx.send(UpdateProgress::Downloading(downloaded, total));
    }
    drop(file);

    // Extract
    let _ = progress_tx.send(UpdateProgress::Extracting);

    std::fs::create_dir_all(install_dir)?;

    let ext = artifact_extension();
    if ext == "tar.gz" {
        extract_tar_gz(tmp_path, install_dir)?;
    } else if ext == "zip" {
        extract_zip(tmp_path, install_dir)?;
    } else {
        // dmg — just copy the file, user will mount manually
        let dest = install_dir.join(tmp_path.file_name().unwrap_or_default());
        std::fs::copy(tmp_path, &dest)?;
    }

    // Make sure the main executable is executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exe_path = install_dir.join(vpx_executable_name());
        if exe_path.exists() {
            std::fs::set_permissions(&exe_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    let exe_path = install_dir.join(vpx_executable_name());
    let _ = progress_tx.send(UpdateProgress::Done(exe_path));

    Ok(())
}

fn extract_tar_gz(archive_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.set_preserve_permissions(true);
    archive.unpack(dest).context("Failed to extract tar.gz")?;
    Ok(())
}

fn extract_zip(archive_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).context("Failed to open zip archive")?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        let out_path = dest.join(&name);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }
    Ok(())
}
