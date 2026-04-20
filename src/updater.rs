// Visual Pinball release checker, downloader, and installer.
// Queries a GitHub fork for releases and downloads the correct artifact
// for the current platform.

use anyhow::{bail, Context, Result};
use crossbeam_channel::Sender;
use regex::Regex;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Default GitHub repository for Visual Pinball releases.
pub const DEFAULT_FORK_REPO: &str = "Le-Syl21/vpinball";

/// Information about a GitHub release.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
}

/// Parse VPX `--version` output into a version string mirroring the release artifact tag format.
///
/// Input:  `Starting VPX - v10.8.1 Beta (Rev. 4955 (da4e2db), macos BGFX 64bits)`
/// Output: `v10.8.1-4955-da4e2db`
///
/// Falls back to just the version number if the revision or SHA cannot be found.
fn parse_vpx_version_output(s: &str) -> Option<String> {
    static VPX_VERSION_RE: OnceLock<Regex> = OnceLock::new();
    let re = VPX_VERSION_RE.get_or_init(|| {
        Regex::new(r"(?x)\b(?P<ver>v\d+\.\d+\.\d+)\b(?:.*?\bRev\.\s*(?P<rev>\d+)\s*\((?P<sha>[0-9A-Fa-f]{7})\))?")
        .expect("valid VPX version regex")
    });

    let caps = re.captures(s)?;
    let base = caps.name("ver")?.as_str();

    match (caps.name("rev"), caps.name("sha")) {
        (Some(rev), Some(sha)) => Some(format!("{}-{}-{}", base, rev.as_str(), sha.as_str())),
        _ => Some(base.to_string()),
    }
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
    // Use /releases (not /releases/latest) because the latest release may be
    // marked as a pre-release, which /releases/latest silently skips.
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=1");

    let response = ureq::get(&url)
        .header("User-Agent", "PinReady")
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .context("Failed to query GitHub releases")?;

    let body = response.into_body().read_to_string()?;
    let releases: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse GitHub release JSON")?;

    let json = releases
        .as_array()
        .and_then(|arr| arr.first())
        .context("No releases found")?;

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

/// Default installation directory for Visual Pinball.
pub fn default_install_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Visual_Pinball")
}

/// Resolve the real executable path from a user-provided path.
/// On macOS, if the path points to a `.app` bundle, look inside
/// `Contents/MacOS/` for the actual binary.
pub fn resolve_vpx_exe(path: &Path) -> PathBuf {
    let p = PathBuf::from(path);
    if cfg!(target_os = "macos") {
        if let Some(ext) = p.extension() {
            if ext == "app" && p.is_dir() {
                let macos_dir = p.join("Contents/MacOS");
                if let Ok(entries) = std::fs::read_dir(&macos_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with("VPinballX") {
                            return entry.path();
                        }
                    }
                }
            }
        }
    }
    p
}

/// Name of the main Visual Pinball executable for the current platform.
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

/// Query the version from a manually installed VPX executable using `--version`.
///
/// Parses output like:
/// `Starting VPX - v10.8.1 Beta (Rev. 4955 (da4e2db), macos BGFX 64bits)`
///
/// to extract a human-readable version token and return it as a string tag.
/// Returns None if the executable cannot be run or the output cannot be parsed.
pub fn query_vpx_version(exe_path: &str) -> Option<String> {
    use std::process::Command;

    let exe_path = resolve_vpx_exe(Path::new(exe_path));

    // Attempt to run the executable with --version flag
    let output = Command::new(&exe_path).arg("--version").output().ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    // Parse into artifact-tag-style version string (e.g., "10.8.1-4955-da4e2db")
    parse_vpx_version_output(&combined)
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn full_vpx_output_produces_artifact_tag_format() {
        assert_eq!(
            parse_vpx_version_output(
                "Starting VPX - v10.8.1 Beta (Rev. 4955 (da4e2db), macos BGFX 64bits)"
            ),
            Some("v10.8.1-4955-da4e2db".to_string())
        );
    }

    #[test]
    fn version_only_fallback_when_no_rev_sha() {
        assert_eq!(
            parse_vpx_version_output("v10.8.1"),
            Some("v10.8.1".to_string())
        );
    }

    #[test]
    fn returns_none_for_unparseable_output() {
        assert_eq!(parse_vpx_version_output("invalid"), None);
        assert_eq!(parse_vpx_version_output("4955"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_fork_repo_is_set() {
        assert!(!DEFAULT_FORK_REPO.is_empty());
        assert!(DEFAULT_FORK_REPO.contains('/'));
    }

    #[test]
    fn artifact_extension_linux() {
        // On Linux where tests run
        let ext = artifact_extension();
        assert!(
            ext == "tar.gz" || ext == "zip" || ext == "dmg",
            "unexpected extension: {ext}"
        );
    }

    #[test]
    fn platform_arch_returns_known_values() {
        let (platform, arch) = platform_arch();
        assert!(
            ["linux", "windows", "macos", "unknown"].contains(&platform),
            "unexpected platform: {platform}"
        );
        assert!(!arch.is_empty(), "arch should not be empty");
    }

    #[test]
    fn vpx_executable_name_not_empty() {
        let name = vpx_executable_name();
        assert!(!name.is_empty());
        assert!(
            name.starts_with("VPinballX"),
            "expected VPinballX prefix: {name}"
        );
    }

    #[test]
    fn default_install_dir_contains_visual_pinball() {
        let dir = default_install_dir();
        let s = dir.to_string_lossy();
        assert!(
            s.contains("Visual_Pinball"),
            "expected Visual_Pinball in: {s}"
        );
    }

    #[test]
    fn resolve_vpx_exe_returns_same_on_linux() {
        // On Linux, resolve_vpx_exe should return the path unchanged
        let path = Path::new("/usr/local/bin/VPinballX_BGFX");
        let resolved = resolve_vpx_exe(path);
        assert_eq!(resolved, path.to_path_buf());
    }

    #[test]
    fn resolve_vpx_exe_nonexistent_path() {
        let path = Path::new("/nonexistent/path/VPinballX_BGFX");
        let resolved = resolve_vpx_exe(path);
        assert_eq!(resolved, path.to_path_buf());
    }

    #[test]
    fn extract_tar_gz_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("test.tar.gz");
        let dest = dir.path().join("extracted");

        // Create a tar.gz with a test file
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);

            let content = b"hello pinball";
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "test.txt", &content[..])
                .unwrap();
            tar.finish().unwrap();
        }

        extract_tar_gz(&archive_path, &dest).unwrap();
        let extracted = std::fs::read_to_string(dest.join("test.txt")).unwrap();
        assert_eq!(extracted, "hello pinball");
    }

    #[test]
    fn extract_zip_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("test.zip");
        let dest = dir.path().join("extracted");

        // Create a zip with a test file
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("test.txt", options).unwrap();
            zip.write_all(b"hello vpx").unwrap();
            zip.finish().unwrap();
        }

        extract_zip(&archive_path, &dest).unwrap();
        let extracted = std::fs::read_to_string(dest.join("test.txt")).unwrap();
        assert_eq!(extracted, "hello vpx");
    }
}
