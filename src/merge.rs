//! Asset bundling ("merge") — port of the core of MajorFrenchy's
//! [VPXmerge.py](https://github.com/MajorFrenchy/VPX-Standalone-Merging-Tool).
//!
//! For each `.vpx` in the user's tables directory, scan three optional
//! source roots (`vpinmame/`, `pupvideos/`, `music/`) and place the
//! companion files (ROM, altsound, altcolor `.vni`, Serum `.crz`, PUP
//! pack, NVRAM, CFG, music, `.directb2s`, POV `.ini`) into the modern
//! folder-per-table layout that VPinballX 10.8.1 expects.
//!
//! Three I/O strategies — `Copy` (default, non-destructive), `Move`
//! (rename + cross-fs fallback), `Symlink` (Unix only by default; on
//! Windows it requires Developer Mode or admin). Idempotency: every
//! placement skips a destination that already has the same size.
//!
//! Spawned on a `std::thread` and emits `MergeEvent`s over a
//! `crossbeam_channel`. A `cancel: Arc<AtomicBool>` mirrors the
//! catalog enrichment worker — checked between tables so a
//! second click doesn't double-run.

use anyhow::Result;
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct MergeSources {
    /// Path to a `VPINMAME` directory with `roms/`, `nvram/`, `cfg/`,
    /// `altsound/`, `altcolor/` subfolders. None = skip the ROM /
    /// altsound / altcolor / serum / nvram / cfg detections.
    pub vpinmame: Option<PathBuf>,
    /// Path to a directory holding PUP pack subfolders.
    pub pupvideos: Option<PathBuf>,
    /// Path to a directory whose subfolders are per-table music sets.
    pub music: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    Copy,
    Move,
    Symlink,
}

impl MergeStrategy {
    pub fn as_db_str(self) -> &'static str {
        match self {
            MergeStrategy::Copy => "copy",
            MergeStrategy::Move => "move",
            MergeStrategy::Symlink => "symlink",
        }
    }
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "move" => MergeStrategy::Move,
            "symlink" => MergeStrategy::Symlink,
            _ => MergeStrategy::Copy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    /// Detect and report what *would* be placed, without touching disk.
    DryRun,
    /// Detect and place files according to the chosen strategy.
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Rom,
    Directb2s,
    PovIni,
    AltSound,
    AltColorVni,
    Serum,
    PupPack,
    Nvram,
    Cfg,
    Music,
}

impl AssetKind {
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Rom => "ROM",
            AssetKind::Directb2s => "directb2s",
            AssetKind::PovIni => "POV.ini",
            AssetKind::AltSound => "altsound",
            AssetKind::AltColorVni => "altcolor (.vni)",
            AssetKind::Serum => "serum (.crz)",
            AssetKind::PupPack => "pup pack",
            AssetKind::Nvram => "nvram",
            AssetKind::Cfg => "cfg",
            AssetKind::Music => "music",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Several fields are only read by the UI's render branch.
pub enum MergeEvent {
    TableStarted {
        name: String,
    },
    AssetFound {
        kind: AssetKind,
        src: PathBuf,
        dst: PathBuf,
    },
    AssetSkipped {
        kind: AssetKind,
        reason: SkipReason,
    },
    AssetApplied {
        kind: AssetKind,
        dst: PathBuf,
    },
    AssetError {
        kind: AssetKind,
        msg: String,
    },
    TableDone {
        name: String,
    },
    Done(MergeReport),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    AlreadyPresent,
    SourceMissing,
    NoSourceRoot,
    DryRun,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::AlreadyPresent => "already present",
            SkipReason::SourceMissing => "source missing",
            SkipReason::NoSourceRoot => "source root not configured",
            SkipReason::DryRun => "dry run",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MergeReport {
    pub tables_processed: usize,
    pub assets_found: usize,
    pub assets_applied: usize,
    pub assets_skipped: usize,
    pub assets_errored: usize,
}

// ---------------------------------------------------------------------------
// FsOp: copy / move / symlink, behind a single trait
// ---------------------------------------------------------------------------

trait FsOp: Send + Sync {
    /// Place a single file at `dst`. Caller has already created `dst`'s parent.
    fn place_file(&self, src: &Path, dst: &Path) -> std::io::Result<()>;
    /// Place a directory tree at `dst`.
    fn place_tree(&self, src: &Path, dst: &Path) -> std::io::Result<()>;
}

struct CopyOp;
impl FsOp for CopyOp {
    fn place_file(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
        std::fs::copy(src, dst).map(|_| ())
    }
    fn place_tree(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
        copy_dir_recursive(src, dst)
    }
}

struct MoveOp;
impl FsOp for MoveOp {
    fn place_file(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
        match std::fs::rename(src, dst) {
            Ok(()) => Ok(()),
            // Cross-filesystem: fall back to copy + remove.
            Err(e) if e.raw_os_error() == Some(libc_exdev()) => {
                std::fs::copy(src, dst)?;
                std::fs::remove_file(src)?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
    fn place_tree(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
        match std::fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(libc_exdev()) => {
                copy_dir_recursive(src, dst)?;
                std::fs::remove_dir_all(src)?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

struct SymlinkOp;
impl FsOp for SymlinkOp {
    fn place_file(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
        symlink_file(src, dst)
    }
    fn place_tree(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
        symlink_dir(src, dst)
    }
}

#[cfg(unix)]
fn symlink_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}
#[cfg(unix)]
fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}
#[cfg(windows)]
fn symlink_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dst)
}
#[cfg(windows)]
fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(src, dst)
}

/// EXDEV errno value (cross-device link). Hardcoded — `libc::EXDEV`
/// would pull a transitive dep we don't otherwise need.
fn libc_exdev() -> i32 {
    #[cfg(target_os = "linux")]
    {
        18
    }
    #[cfg(target_os = "macos")]
    {
        18
    }
    #[cfg(target_os = "windows")]
    {
        17
    } // ERROR_NOT_SAME_DEVICE
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        18
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src)
        .min_depth(1)
        .into_iter()
        .flatten()
    {
        let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn pick_op(strategy: MergeStrategy) -> Box<dyn FsOp> {
    match strategy {
        MergeStrategy::Copy => Box::new(CopyOp),
        MergeStrategy::Move => Box::new(MoveOp),
        MergeStrategy::Symlink => Box::new(SymlinkOp),
    }
}

// ---------------------------------------------------------------------------
// Worker entry point
// ---------------------------------------------------------------------------

/// Spawn a merge worker thread. Returns the event receiver and the
/// cancel token. Drop the receiver to ignore further events; flip the
/// token to `true` to ask the worker to stop after the current table.
pub fn spawn(
    tables_dir: PathBuf,
    sources: MergeSources,
    strategy: MergeStrategy,
    mode: MergeMode,
) -> (
    crossbeam_channel::Receiver<MergeEvent>,
    Arc<AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    let (tx, rx) = crossbeam_channel::unbounded::<MergeEvent>();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let handle = std::thread::Builder::new()
        .name("pinready-merge".into())
        .spawn(move || {
            if let Err(e) = run(&tables_dir, &sources, strategy, mode, &tx, &cancel_clone) {
                let _ = tx.send(MergeEvent::AssetError {
                    kind: AssetKind::Rom, // generic carrier
                    msg: format!("merge worker failed: {e}"),
                });
            }
        })
        .expect("spawn merge thread");
    (rx, cancel, handle)
}

fn run(
    tables_dir: &Path,
    sources: &MergeSources,
    strategy: MergeStrategy,
    mode: MergeMode,
    tx: &Sender<MergeEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let op = pick_op(strategy);
    let mut report = MergeReport::default();

    let entries = match std::fs::read_dir(tables_dir) {
        Ok(it) => it,
        Err(e) => {
            let _ = tx.send(MergeEvent::AssetError {
                kind: AssetKind::Rom,
                msg: format!("cannot read tables dir {}: {e}", tables_dir.display()),
            });
            let _ = tx.send(MergeEvent::Done(report));
            return Ok(());
        }
    };

    for entry in entries.flatten() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let table_dir = entry.path();
        if !table_dir.is_dir() {
            continue;
        }
        let Some(vpx_path) = find_vpx_in(&table_dir) else {
            continue;
        };
        let table_name = table_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let _ = tx.send(MergeEvent::TableStarted {
            name: table_name.clone(),
        });

        process_table(
            &table_dir,
            &vpx_path,
            sources,
            mode,
            op.as_ref(),
            tx,
            &mut report,
        );
        report.tables_processed += 1;
        let _ = tx.send(MergeEvent::TableDone { name: table_name });
    }

    let _ = tx.send(MergeEvent::Done(report));
    Ok(())
}

fn find_vpx_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) == Some("vpx") {
            Some(p)
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Per-table processing
// ---------------------------------------------------------------------------

struct TableContext {
    rom: Option<String>,
    table_name_embedded: Option<String>,
    pgame_names: Vec<String>,
    cpup_pack: Option<String>,
    base: String, // .vpx file stem, e.g. "Apollo 13 (Sega 1995)"
}

fn process_table(
    table_dir: &Path,
    vpx_path: &Path,
    sources: &MergeSources,
    mode: MergeMode,
    op: &dyn FsOp,
    tx: &Sender<MergeEvent>,
    report: &mut MergeReport,
) {
    let base = vpx_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let (rom_via_meta, table_name_embedded) = crate::vpsdb::matcher::read_vpx_meta(vpx_path);

    // Sidecar .vbs for pGameName / cPuPPack hints. Falls back silently
    // if the user only ships .vpx without a sidecar.
    let sidecar_vbs = table_dir.join(format!("{base}.vbs"));
    let sidecar_text = std::fs::read_to_string(&sidecar_vbs).unwrap_or_default();
    let pgame_names = extract_pgame_names(&sidecar_text);
    let cpup_pack = extract_cpup_pack(&sidecar_text);

    let ctx = TableContext {
        rom: rom_via_meta,
        table_name_embedded,
        pgame_names,
        cpup_pack,
        base,
    };

    // 1. ROM
    if let Some(rom) = &ctx.rom {
        place_file_asset(
            AssetKind::Rom,
            sources
                .vpinmame
                .as_ref()
                .map(|r| r.join("roms").join(format!("{rom}.zip"))),
            table_dir.join("pinmame/roms").join(format!("{rom}.zip")),
            mode,
            op,
            tx,
            report,
        );
    }

    // 2. .directb2s — same basename as .vpx, in table dir or its parent
    {
        let candidates = [
            table_dir.join(format!("{}.directb2s", ctx.base)),
            table_dir
                .parent()
                .map(|p| p.join(format!("{}.directb2s", ctx.base)))
                .unwrap_or_else(|| table_dir.join(format!("{}.directb2s", ctx.base))),
        ];
        let src = candidates.into_iter().find(|p| p.is_file());
        let dst = table_dir.join(format!("{}.directb2s", ctx.base));
        place_file_asset(AssetKind::Directb2s, src, dst, mode, op, tx, report);
    }

    // 3. POV .ini
    {
        let candidates = [
            table_dir.join(format!("{}.ini", ctx.base)),
            table_dir
                .parent()
                .map(|p| p.join(format!("{}.ini", ctx.base)))
                .unwrap_or_else(|| table_dir.join(format!("{}.ini", ctx.base))),
        ];
        let src = candidates.into_iter().find(|p| p.is_file());
        let dst = table_dir.join(format!("{}.ini", ctx.base));
        place_file_asset(AssetKind::PovIni, src, dst, mode, op, tx, report);
    }

    // 4. AltSound — directory keyed by ROM
    if let (Some(rom), Some(vpinmame)) = (&ctx.rom, sources.vpinmame.as_ref()) {
        let src = vpinmame.join("altsound").join(rom);
        let dst = table_dir.join("pinmame/altsound").join(rom);
        place_dir_asset(
            AssetKind::AltSound,
            src.is_dir().then_some(src),
            dst,
            mode,
            op,
            tx,
            report,
        );
    } else {
        report_skipped(AssetKind::AltSound, SkipReason::NoSourceRoot, tx, report);
    }

    // 5. AltColor (.vni) — under altcolor/<key>/, key tried as rom, base, or each pGameName.
    let color_keys: Vec<String> = std::iter::once(ctx.rom.clone().unwrap_or_default())
        .chain(std::iter::once(ctx.base.clone()))
        .chain(ctx.pgame_names.iter().cloned())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(vpinmame) = sources.vpinmame.as_ref() {
        let altcolor_root = vpinmame.join("altcolor");
        let primary_key = ctx
            .rom
            .clone()
            .or_else(|| color_keys.first().cloned())
            .unwrap_or_default();
        let mut found_vni = None;
        for key in &color_keys {
            let dir = altcolor_root.join(key);
            if dir.is_dir() && contains_extension(&dir, "vni") {
                found_vni = Some(dir);
                break;
            }
        }
        let dst = table_dir.join("vni").join(&primary_key);
        place_dir_asset(AssetKind::AltColorVni, found_vni, dst, mode, op, tx, report);

        // 6. Serum (.crz) — either altcolor/<key>.crz directly, or a .crz inside altcolor/<key>/
        let mut found_crz: Option<PathBuf> = None;
        for key in &color_keys {
            let direct = altcolor_root.join(format!("{key}.crz"));
            if direct.is_file() {
                found_crz = Some(direct);
                break;
            }
            let nested_dir = altcolor_root.join(key);
            if nested_dir.is_dir() {
                if let Some(crz) = first_with_extension(&nested_dir, "crz") {
                    found_crz = Some(crz);
                    break;
                }
            }
        }
        if let Some(src) = found_crz {
            let dst = table_dir.join("serum").join(
                src.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("colorize.crz")),
            );
            place_file_asset(AssetKind::Serum, Some(src), dst, mode, op, tx, report);
        } else {
            report_skipped(AssetKind::Serum, SkipReason::SourceMissing, tx, report);
        }
    } else {
        report_skipped(AssetKind::AltColorVni, SkipReason::NoSourceRoot, tx, report);
        report_skipped(AssetKind::Serum, SkipReason::NoSourceRoot, tx, report);
    }

    // 7. PUP pack — fuzzy folder match
    if let Some(pup_root) = sources.pupvideos.as_ref() {
        let folders = list_subdirs(pup_root);
        let targets: Vec<&str> = std::iter::empty::<&str>()
            .chain(ctx.cpup_pack.iter().map(|s| s.as_str()))
            .chain(ctx.pgame_names.iter().map(|s| s.as_str()))
            .chain(ctx.table_name_embedded.iter().map(|s| s.as_str()))
            .chain(std::iter::once(ctx.base.as_str()))
            .chain(ctx.rom.iter().map(|s| s.as_str()))
            .collect();
        let folder_strs: Vec<&str> = folders.iter().map(|s| s.as_str()).collect();
        if let Some(matched) = fuzzy::find_pup_folder(&targets, &folder_strs) {
            let src = pup_root.join(&matched);
            let dst = table_dir.join("pupvideos").join(&matched);
            place_dir_asset(AssetKind::PupPack, Some(src), dst, mode, op, tx, report);
        } else {
            report_skipped(AssetKind::PupPack, SkipReason::SourceMissing, tx, report);
        }
    } else {
        report_skipped(AssetKind::PupPack, SkipReason::NoSourceRoot, tx, report);
    }

    // 8. NVRAM + 8b. CFG
    if let (Some(rom), Some(vpinmame)) = (&ctx.rom, sources.vpinmame.as_ref()) {
        let nvram_src = vpinmame.join("nvram").join(format!("{rom}.nv"));
        let nvram_dst = table_dir.join("pinmame/nvram").join(format!("{rom}.nv"));
        place_file_asset(
            AssetKind::Nvram,
            nvram_src.is_file().then_some(nvram_src),
            nvram_dst,
            mode,
            op,
            tx,
            report,
        );

        let cfg_src = vpinmame.join("cfg").join(format!("{rom}.cfg"));
        let cfg_dst = table_dir.join("pinmame/cfg").join(format!("{rom}.cfg"));
        place_file_asset(
            AssetKind::Cfg,
            cfg_src.is_file().then_some(cfg_src),
            cfg_dst,
            mode,
            op,
            tx,
            report,
        );
    } else {
        report_skipped(AssetKind::Nvram, SkipReason::NoSourceRoot, tx, report);
        report_skipped(AssetKind::Cfg, SkipReason::NoSourceRoot, tx, report);
    }

    // 9. Music — match a subdir whose name equals base or rom (case-insensitive).
    if let Some(music_root) = sources.music.as_ref() {
        let mut matched: Option<PathBuf> = None;
        for cand in std::iter::once(ctx.base.as_str()).chain(ctx.rom.iter().map(|s| s.as_str())) {
            let dir = music_root.join(cand);
            if dir.is_dir() {
                matched = Some(dir);
                break;
            }
            // case-insensitive scan
            if let Ok(entries) = std::fs::read_dir(music_root) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir())
                        && entry
                            .file_name()
                            .to_str()
                            .is_some_and(|n| n.eq_ignore_ascii_case(cand))
                    {
                        matched = Some(entry.path());
                        break;
                    }
                }
            }
            if matched.is_some() {
                break;
            }
        }
        let primary = ctx.base.clone();
        let dst = table_dir.join("music").join(&primary);
        place_dir_asset(AssetKind::Music, matched, dst, mode, op, tx, report);
    } else {
        report_skipped(AssetKind::Music, SkipReason::NoSourceRoot, tx, report);
    }
}

// ---------------------------------------------------------------------------
// Placement helpers
// ---------------------------------------------------------------------------

fn place_file_asset(
    kind: AssetKind,
    src: Option<PathBuf>,
    dst: PathBuf,
    mode: MergeMode,
    op: &dyn FsOp,
    tx: &Sender<MergeEvent>,
    report: &mut MergeReport,
) {
    let Some(src) = src else {
        report_skipped(kind, SkipReason::SourceMissing, tx, report);
        return;
    };
    if !src.is_file() {
        report_skipped(kind, SkipReason::SourceMissing, tx, report);
        return;
    }
    if dst.exists() && file_size(&dst) == file_size(&src) {
        report_skipped(kind, SkipReason::AlreadyPresent, tx, report);
        return;
    }
    report.assets_found += 1;
    let _ = tx.send(MergeEvent::AssetFound {
        kind,
        src: src.clone(),
        dst: dst.clone(),
    });
    if matches!(mode, MergeMode::DryRun) {
        report.assets_skipped += 1;
        let _ = tx.send(MergeEvent::AssetSkipped {
            kind,
            reason: SkipReason::DryRun,
        });
        return;
    }
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match op.place_file(&src, &dst) {
        Ok(()) => {
            report.assets_applied += 1;
            let _ = tx.send(MergeEvent::AssetApplied { kind, dst });
        }
        Err(e) => {
            report.assets_errored += 1;
            let _ = tx.send(MergeEvent::AssetError {
                kind,
                msg: e.to_string(),
            });
        }
    }
}

fn place_dir_asset(
    kind: AssetKind,
    src: Option<PathBuf>,
    dst: PathBuf,
    mode: MergeMode,
    op: &dyn FsOp,
    tx: &Sender<MergeEvent>,
    report: &mut MergeReport,
) {
    let Some(src) = src else {
        report_skipped(kind, SkipReason::SourceMissing, tx, report);
        return;
    };
    if !src.is_dir() {
        report_skipped(kind, SkipReason::SourceMissing, tx, report);
        return;
    }
    if dst.is_dir() && dir_nonempty(&dst) {
        report_skipped(kind, SkipReason::AlreadyPresent, tx, report);
        return;
    }
    report.assets_found += 1;
    let _ = tx.send(MergeEvent::AssetFound {
        kind,
        src: src.clone(),
        dst: dst.clone(),
    });
    if matches!(mode, MergeMode::DryRun) {
        report.assets_skipped += 1;
        let _ = tx.send(MergeEvent::AssetSkipped {
            kind,
            reason: SkipReason::DryRun,
        });
        return;
    }
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match op.place_tree(&src, &dst) {
        Ok(()) => {
            report.assets_applied += 1;
            let _ = tx.send(MergeEvent::AssetApplied { kind, dst });
        }
        Err(e) => {
            report.assets_errored += 1;
            let _ = tx.send(MergeEvent::AssetError {
                kind,
                msg: e.to_string(),
            });
        }
    }
}

fn report_skipped(
    kind: AssetKind,
    reason: SkipReason,
    tx: &Sender<MergeEvent>,
    report: &mut MergeReport,
) {
    report.assets_skipped += 1;
    let _ = tx.send(MergeEvent::AssetSkipped { kind, reason });
}

// ---------------------------------------------------------------------------
// Small filesystem helpers
// ---------------------------------------------------------------------------

fn file_size(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().map(|m| m.len())
}

fn dir_nonempty(p: &Path) -> bool {
    std::fs::read_dir(p)
        .ok()
        .and_then(|mut it| it.next())
        .is_some()
}

fn list_subdirs(p: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(p) else {
        return vec![];
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

fn contains_extension(dir: &Path, ext: &str) -> bool {
    walkdir::WalkDir::new(dir)
        .max_depth(2)
        .into_iter()
        .flatten()
        .any(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case(ext))
        })
}

fn first_with_extension(dir: &Path, ext: &str) -> Option<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(2)
        .into_iter()
        .flatten()
        .find(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case(ext))
        })
        .map(|e| e.into_path())
}

// ---------------------------------------------------------------------------
// VBS hint extraction
// ---------------------------------------------------------------------------

fn extract_pgame_names(vbs: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in vbs.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('\'') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(idx) = lower.find("pgamename") {
            let after = &trimmed[idx + "pgamename".len()..];
            if let Some(eq) = after.find('=') {
                if let Some(rest) = after[eq + 1..].split('"').nth(1) {
                    let val = rest.trim();
                    if !val.is_empty() && !out.iter().any(|s: &String| s.eq_ignore_ascii_case(val))
                    {
                        out.push(val.to_string());
                    }
                }
            }
        }
    }
    out
}

fn extract_cpup_pack(vbs: &str) -> Option<String> {
    for line in vbs.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('\'') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(idx) = lower.find("cpuppack") {
            let after = &trimmed[idx + "cpuppack".len()..];
            if let Some(eq) = after.find('=') {
                if let Some(rest) = after[eq + 1..].split('"').nth(1) {
                    let val = rest.trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Fuzzy matching for PUP packs (5-stage cascade, lifted from VPXmerge.py)
// ---------------------------------------------------------------------------

pub mod fuzzy {
    /// Tokens stripped before keyword overlap. Mirrors `_MEDIA_NOISE`
    /// in VPXmerge.py:12 — common cab/edition decorations that would
    /// otherwise blow up the false-positive rate.
    const NOISE: &[&str] = &[
        "limited",
        "edition",
        "le",
        "pro",
        "premium",
        "vr",
        "vpw",
        "mod",
        "sg1",
        "vpu",
        "the",
        "a",
        "an",
        "and",
        "of",
        "in",
        "remaster",
        "vpx",
        "remake",
        "ultimate",
        "deluxe",
        "special",
        "anniversary",
        "collector",
        "classic",
        "night",
        "jp",
        "fizx",
        "se",
        "ce",
    ];

    /// Lowercase + drop possessive `'s` + drop everything non-alphanumeric.
    pub fn compact(name: &str) -> String {
        let mut s = name.to_lowercase();
        // possessive 's
        for marker in [
            "'s ",
            "\u{2019}s ",
            "\u{2018}s ",
            "'s",
            "\u{2019}s",
            "\u{2018}s",
        ] {
            s = s.replace(marker, "");
        }
        s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
    }

    fn keywords(name: &str) -> std::collections::BTreeSet<String> {
        let lowered = name.to_lowercase();
        let mut clean = String::with_capacity(lowered.len());
        for c in lowered.chars() {
            if c.is_ascii_alphanumeric() {
                clean.push(c);
            } else {
                clean.push(' ');
            }
        }
        clean
            .split_whitespace()
            .filter(|w| !NOISE.contains(w))
            .map(|s| s.to_string())
            .collect()
    }

    /// 0.0–1.0 keyword overlap score — `intersection / max(|a|, |b|)`.
    pub fn keyword_overlap(a: &str, b: &str) -> f32 {
        let ka = keywords(a);
        let kb = keywords(b);
        if ka.is_empty() || kb.is_empty() {
            return 0.0;
        }
        let inter = ka.intersection(&kb).count();
        inter as f32 / ka.len().max(kb.len()) as f32
    }

    /// Levenshtein distance, naive O(n*m).
    fn levenshtein(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        if a.is_empty() {
            return b.len();
        }
        if b.is_empty() {
            return a.len();
        }
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        let mut curr = vec![0usize; b.len() + 1];
        for i in 1..=a.len() {
            curr[0] = i;
            for j in 1..=b.len() {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            }
            std::mem::swap(&mut prev, &mut curr);
        }
        prev[b.len()]
    }

    /// 0.0–1.0 normalized Levenshtein similarity.
    pub fn levenshtein_ratio(a: &str, b: &str) -> f32 {
        let max = a.chars().count().max(b.chars().count());
        if max == 0 {
            return 1.0;
        }
        1.0 - (levenshtein(a, b) as f32 / max as f32)
    }

    /// Match a PUP folder name from `folders` against any of `targets`,
    /// using the 5-stage cascade VPXmerge.py runs:
    ///   1. exact (case-insensitive)
    ///   2. compact equality
    ///   3. compact prefix/contains (smallest length-delta wins)
    ///   4. levenshtein ratio ≥ 0.86 on compact forms
    ///   5. keyword overlap ≥ 0.5 on raw names
    pub fn find_pup_folder(targets: &[&str], folders: &[&str]) -> Option<String> {
        // 1. exact CI
        for t in targets {
            for f in folders {
                if t.eq_ignore_ascii_case(f) {
                    return Some((*f).to_string());
                }
            }
        }
        // 2. compact ==
        let compact_targets: Vec<String> = targets
            .iter()
            .map(|t| compact(t))
            .filter(|s| !s.is_empty())
            .collect();
        for f in folders {
            let cf = compact(f);
            if !cf.is_empty() && compact_targets.iter().any(|ct| ct == &cf) {
                return Some((*f).to_string());
            }
        }
        // 3. compact prefix/contains, smallest length delta
        let mut best: Option<(usize, String)> = None;
        for f in folders {
            let cf = compact(f);
            if cf.is_empty() {
                continue;
            }
            for ct in &compact_targets {
                if cf.starts_with(ct.as_str())
                    || ct.starts_with(cf.as_str())
                    || ct.contains(cf.as_str())
                    || cf.contains(ct.as_str())
                {
                    let delta = cf.len().abs_diff(ct.len());
                    if best.as_ref().is_none_or(|(b, _)| delta < *b) {
                        best = Some((delta, (*f).to_string()));
                    }
                }
            }
        }
        if let Some((_, name)) = best {
            return Some(name);
        }
        // 4. Levenshtein ratio ≥ 0.86 on compact forms
        let mut best_ratio = 0.0_f32;
        let mut best_name: Option<String> = None;
        for f in folders {
            let cf = compact(f);
            if cf.is_empty() {
                continue;
            }
            for ct in &compact_targets {
                let r = levenshtein_ratio(ct, &cf);
                if r > best_ratio {
                    best_ratio = r;
                    best_name = Some((*f).to_string());
                }
            }
        }
        if best_ratio >= 0.86 {
            if let Some(n) = best_name {
                return Some(n);
            }
        }
        // 5. keyword overlap ≥ 0.5 on raw names
        let mut best_score = 0.0_f32;
        let mut best_kw: Option<String> = None;
        for t in targets {
            for f in folders {
                let s = keyword_overlap(t, f);
                if s > best_score {
                    best_score = s;
                    best_kw = Some((*f).to_string());
                }
            }
        }
        if best_score >= 0.5 {
            return best_kw;
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn compact_handles_apostrophes_and_punct() {
            assert_eq!(
                compact("Dragon's Lair (Williams 1990)"),
                "dragonlairwilliams1990"
            );
            assert_eq!(compact("AC/DC"), "acdc");
        }

        #[test]
        fn keyword_overlap_ignores_noise() {
            // "the", "limited", "edition", "vpx" are all noise tokens.
            let a = "The Walking Dead Limited Edition VPX";
            let b = "The Walking Dead";
            assert!(keyword_overlap(a, b) >= 0.99);
        }

        #[test]
        fn pup_finder_exact_ci() {
            let folders = ["Apollo13", "MM"];
            let got = find_pup_folder(&["apollo13"], &folders);
            assert_eq!(got.as_deref(), Some("Apollo13"));
        }

        #[test]
        fn pup_finder_compact() {
            let folders = ["dragonlair-3screen"];
            let got = find_pup_folder(&["Dragon's Lair"], &folders);
            assert_eq!(got.as_deref(), Some("dragonlair-3screen"));
        }

        #[test]
        fn pup_finder_typo() {
            let folders = ["dragonlain"];
            let got = find_pup_folder(&["dragonlair"], &folders);
            assert_eq!(got.as_deref(), Some("dragonlain"));
        }

        #[test]
        fn pup_finder_no_match() {
            let folders = ["zaccaria-magic-castle"];
            let got = find_pup_folder(&["Apollo 13"], &folders);
            assert!(got.is_none());
        }
    }
}
