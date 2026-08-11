//! Asset bundling ("import") — the matching logic descends from
//! MajorFrenchy's [VPXmerge.py](https://github.com/MajorFrenchy/VPX-Standalone-Merging-Tool),
//! the input model does not.
//!
//! The user points at **one** directory — a table collection, an old VPX
//! install, or a whole disk — and everything else is discovered. A single
//! recursive pass indexes the files worth knowing about (tables, scripts,
//! backglasses, ROM zips, NVRAM, colorizations, PUP packs, music) and
//! classifies the folders that carry them. Each `.vpx` then resolves its
//! companions against that index, so nobody has to tell us where
//! `altsound/`, `pupvideos/` or `roms/` live — or even that they exist.
//!
//! Output is a separate directory: the folder-per-table layout that
//! VPinballX 10.8.1 expects. Input and output are the same path only when
//! the user says the collection is already in the modern layout, in which
//! case tables stay where they are and only missing companions are pulled
//! in.
//!
//! Three I/O strategies — `Copy` (default, non-destructive), `Move`
//! (rename + cross-fs fallback), `Symlink` (CLI only; Unix by default, on
//! Windows it requires Developer Mode or admin). Idempotency: every
//! placement skips a destination that already has the same size.
//!
//! Spawned on a `std::thread` and emits `MergeEvent`s over a
//! `crossbeam_channel`. A `cancel: Arc<AtomicBool>` mirrors the
//! catalog enrichment worker — checked between tables, and during the
//! scan, so a whole-disk index can be interrupted.

use anyhow::Result;
use crossbeam_channel::Sender;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct MergeConfig {
    /// The single directory to index. May hold anything, at any depth —
    /// a tables folder, an old VPinMAME install, an entire drive.
    pub scan_root: PathBuf,
    /// Where the folder-per-table layout is written: the tables dir.
    /// Equal to `scan_root` when the collection is already modern.
    pub output_root: PathBuf,
    pub strategy: MergeStrategy,
    pub mode: MergeMode,
}

impl MergeConfig {
    /// True when the user declared the collection already
    /// folder-per-table: tables are completed where they sit, never
    /// duplicated into a second layout.
    pub fn is_in_place(&self) -> bool {
        canonical(&self.scan_root) == canonical(&self.output_root)
    }
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

// ---------------------------------------------------------------------------
// The index: one pass over the scan root, everything the matcher can need
// ---------------------------------------------------------------------------

/// File extensions worth remembering by name. Audio, `.vni`/`.pal` and
/// `.csv` are deliberately absent: they are only used to *classify the
/// folder that holds them* (an altsound pack is thousands of `.ogg`, and
/// indexing every one of them on a full disk buys nothing).
const INDEXED_EXTS: &[&str] = &[
    "vpx",
    "vbs",
    "directb2s",
    "ini",
    "pov",
    "res",
    "zip",
    "nv",
    "cfg",
    "crz",
];

/// Tables VPX and VPinMAME ship themselves. An old drive holds one copy
/// per install — five of each was typical on a real collection — and the
/// current VPX build already provides them, so importing them is pure
/// noise. Matched on the file stem, case-insensitively.
const SAMPLE_TABLES: &[&str] = &[
    "blanktable",
    "exampletable",
    "lightseqtable",
    "strippedtable",
    "flexdemo",
    "nudge test and calibration",
    "screen size calibration",
];

/// Same idea, but the physics test table carries its VPX revision in the
/// name ("JP's VPX8 Physics Rev3.1 Elasticity_Test"), so match the part
/// that does not move.
const SAMPLE_TABLE_MARKER: &str = "elasticity_test";

fn is_sample_table(vpx: &Path) -> bool {
    let Some(stem) = vpx.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let stem = stem.to_lowercase();
    SAMPLE_TABLES.contains(&stem.as_str()) || stem.contains(SAMPLE_TABLE_MARKER)
}

/// Directories never worth descending into. Kernel/pseudo filesystems
/// would make a `/`-rooted scan crawl forever, and package caches hold
/// nothing a pinball table wants.
const SKIP_DIRS: &[&str] = &[
    "proc",
    "sys",
    "dev",
    "run",
    "lost+found",
    "node_modules",
    "$recycle.bin",
    "system volume information",
    "windows",
];

#[derive(Default)]
pub struct AssetIndex {
    /// Lowercased file name → every path carrying it.
    files_by_name: HashMap<String, Vec<PathBuf>>,
    /// Lowercased directory name → every classified directory with it.
    dirs_by_name: HashMap<String, Vec<PathBuf>>,
    /// Directories holding a `playlists.pup` marker.
    pup_packs: Vec<PathBuf>,
    /// Directories holding audio *and* a `.csv` — an altsound pack.
    altsound_dirs: HashSet<PathBuf>,
    /// Directories holding `.vni` / `.pal` — a colorization.
    altcolor_dirs: HashSet<PathBuf>,
    /// Directories holding audio and nothing that marks them as altsound.
    music_dirs: HashSet<PathBuf>,
    /// Every `.vpx` found, sorted for a stable run order.
    pub tables: Vec<PathBuf>,
    /// Sample tables that were left out, so the count can be shown
    /// rather than the filtering being silent.
    pub samples_skipped: usize,
    pub files_indexed: usize,
    pub dirs_scanned: usize,
}

impl AssetIndex {
    fn add_dir_name(&mut self, dir: &Path) {
        if let Some(name) = dir.file_name().and_then(|s| s.to_str()) {
            let key = name.to_lowercase();
            let slot = self.dirs_by_name.entry(key).or_default();
            if !slot.contains(&dir.to_path_buf()) {
                slot.push(dir.to_path_buf());
            }
        }
    }

    /// Best path for `name`, case-insensitively.
    ///
    /// Ranking, in order: a file sitting next to the table wins (a
    /// collection often ships its own backglass), then one under a
    /// canonically named parent (`roms/`, `nvram/`…), then the shallowest
    /// path so the result does not depend on directory iteration order.
    fn file(&self, name: &str, preferred_parents: &[&str], near: Option<&Path>) -> Option<PathBuf> {
        let candidates = self.files_by_name.get(&name.to_lowercase())?;
        candidates
            .iter()
            .min_by_key(|p| {
                let beside = near.is_some_and(|d| p.parent() == Some(d));
                let parent_ok = p
                    .parent()
                    .and_then(|d| d.file_name())
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| preferred_parents.iter().any(|w| n.eq_ignore_ascii_case(w)));
                (
                    !beside,
                    !parent_ok,
                    p.components().count(),
                    p.to_string_lossy().into_owned(),
                )
            })
            .cloned()
    }

    /// Best directory named `name` among `set` (an altsound / altcolor /
    /// music classification), shallowest first for determinism.
    fn dir_in(&self, name: &str, set: &HashSet<PathBuf>) -> Option<PathBuf> {
        let candidates = self.dirs_by_name.get(&name.to_lowercase())?;
        candidates
            .iter()
            .filter(|p| set.contains(*p))
            .min_by_key(|p| (p.components().count(), p.to_string_lossy().into_owned()))
            .cloned()
    }
}

/// Walk `root` once and build the index. `on_progress` is called every
/// few thousand entries so the UI can move a bar instead of freezing.
pub fn build_index(
    root: &Path,
    skip_subtree: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(usize, usize, &Path),
) -> AssetIndex {
    let mut index = AssetIndex::default();
    let skip = skip_subtree.map(canonical);
    // Audio folders can only be told apart from altsound packs once the
    // whole folder is known (altsound = audio + a .csv manifest), so
    // classification is deferred to the end of the walk.
    let mut audio_dirs: HashSet<PathBuf> = HashSet::new();
    let mut csv_dirs: HashSet<PathBuf> = HashSet::new();

    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            if e.depth() > 0 {
                let name = e.file_name().to_string_lossy().to_lowercase();
                if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                    return false;
                }
            }
            // Never index our own output as a source.
            !skip.as_ref().is_some_and(|s| canonical(e.path()) == *s)
        });

    for entry in walker.flatten() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        if entry.file_type().is_dir() {
            index.dirs_scanned += 1;
            if index.dirs_scanned.is_multiple_of(200) {
                on_progress(index.files_indexed, index.dirs_scanned, entry.path());
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower_name = name.to_lowercase();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        let parent = path.parent().map(Path::to_path_buf);

        if lower_name == "playlists.pup" {
            if let Some(dir) = &parent {
                index.pup_packs.push(dir.clone());
                index.add_dir_name(dir);
            }
        }
        match ext.as_str() {
            "vni" | "pal" => {
                if let Some(dir) = &parent {
                    index.altcolor_dirs.insert(dir.clone());
                    index.add_dir_name(dir);
                }
            }
            "ogg" | "mp3" | "wav" | "flac" | "m4a" => {
                if let Some(dir) = &parent {
                    audio_dirs.insert(dir.clone());
                }
            }
            "csv" => {
                if let Some(dir) = &parent {
                    csv_dirs.insert(dir.clone());
                }
            }
            _ => {}
        }
        if !INDEXED_EXTS.contains(&ext.as_str()) {
            continue;
        }
        if ext == "vpx" {
            if is_sample_table(path) {
                index.samples_skipped += 1;
            } else {
                index.tables.push(path.to_path_buf());
            }
        }
        index
            .files_by_name
            .entry(lower_name)
            .or_default()
            .push(path.to_path_buf());
        index.files_indexed += 1;
        if index.files_indexed.is_multiple_of(500) {
            on_progress(
                index.files_indexed,
                index.dirs_scanned,
                path.parent().unwrap_or(path),
            );
        }
    }

    // An altsound pack is audio + its manifest; the rest of the audio
    // folders are music sets. Both get their names indexed so a table can
    // find them by ROM or by title.
    for dir in audio_dirs {
        if csv_dirs.contains(&dir) {
            index.altsound_dirs.insert(dir.clone());
        } else {
            index.music_dirs.insert(dir.clone());
        }
        index.add_dir_name(&dir);
    }
    index.tables.sort();
    index.tables.dedup();
    on_progress(index.files_indexed, index.dirs_scanned, root);
    index
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
    Vpx,
    Vbs,
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
            AssetKind::Vpx => "table (.vpx)",
            AssetKind::Vbs => "script (.vbs)",
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
    /// Step 1 — the recursive index of the scan root is underway.
    ScanProgress {
        files: usize,
        dirs: usize,
        /// Folder being walked right now — a counter alone looks frozen
        /// on a spinning disk.
        folder: PathBuf,
    },
    /// Step 1 done: how much was indexed, and how many tables came out.
    ScanDone {
        files: usize,
        dirs: usize,
        tables: usize,
    },
    TableStarted {
        name: String,
        /// 1-based position in the run, for the progress bar.
        index: usize,
        total: usize,
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
    /// Several copies of the same table in the scan root: the newest one
    /// wins, the others are surfaced — with the path of each — instead of
    /// silently overwriting it.
    TableSkipped {
        name: String,
        index: usize,
        total: usize,
        /// The copy that was left behind.
        src: PathBuf,
        /// The copy that was imported instead.
        kept: PathBuf,
    },
    Done(MergeReport),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    AlreadyPresent,
    SourceMissing,
    DryRun,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::AlreadyPresent => "already present",
            SkipReason::SourceMissing => "source missing",
            SkipReason::DryRun => "dry run",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MergeReport {
    pub files_indexed: usize,
    /// Assets the collection simply does not have. Not a decision, so
    /// kept out of the headline counters.
    pub assets_absent: usize,
    pub tables_processed: usize,
    pub tables_skipped: usize,
    /// VPX's own sample tables, left out on purpose.
    pub tables_sample_skipped: usize,
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
    config: MergeConfig,
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
            if let Err(e) = run(&config, &tx, &cancel_clone) {
                let _ = tx.send(MergeEvent::AssetError {
                    kind: AssetKind::Rom, // generic carrier
                    msg: format!("merge worker failed: {e}"),
                });
            }
        })
        .expect("spawn merge thread");
    (rx, cancel, handle)
}

fn run(config: &MergeConfig, tx: &Sender<MergeEvent>, cancel: &Arc<AtomicBool>) -> Result<()> {
    let mut sink = Sink {
        mode: config.mode,
        op: pick_op(config.strategy),
        tx,
        report: MergeReport::default(),
        table_dir: PathBuf::new(),
    };
    let in_place = config.is_in_place();

    // ---- Step 1: index the scan root -------------------------------
    //
    // One pass, however deep, however wide: the user gave us a single
    // directory precisely so they would not have to know where their
    // ROMs, PUP packs or altsound folders ended up.
    let index = build_index(
        &config.scan_root,
        (!in_place).then_some(config.output_root.as_path()),
        cancel,
        |files, dirs, folder| {
            let _ = tx.send(MergeEvent::ScanProgress {
                files,
                dirs,
                folder: folder.to_path_buf(),
            });
        },
    );
    sink.report.files_indexed = index.files_indexed;
    sink.report.tables_sample_skipped = index.samples_skipped;
    let _ = tx.send(MergeEvent::ScanDone {
        files: index.files_indexed,
        dirs: index.dirs_scanned,
        tables: index.tables.len(),
    });
    if cancel.load(Ordering::SeqCst) {
        let _ = tx.send(MergeEvent::Done(sink.report.clone()));
        return Ok(());
    }

    // ---- Step 2: one folder-per-table bundle per .vpx ---------------
    //
    // A collection built up over years holds the same table several times
    // — five copies of VPX's own sample tables, one per install, is
    // typical. They all target one bundle, so pick a winner up front:
    // the most recently modified copy, which is the one the user has been
    // playing. Everything else is reported with both paths so nothing
    // disappears quietly.
    let destinations: Vec<PathBuf> = index
        .tables
        .iter()
        .map(|vpx| canonical(&destination_dir(vpx, &config.output_root)))
        .collect();
    let mut groups: HashMap<&PathBuf, Vec<&PathBuf>> = HashMap::new();
    for (vpx, dest) in index.tables.iter().zip(&destinations) {
        groups.entry(dest).or_default().push(vpx);
    }
    let mut winners: HashMap<PathBuf, PathBuf> = HashMap::new();
    for (dest, copies) in groups {
        let winner = if copies.len() == 1 {
            copies[0].clone()
        } else {
            copies
                .iter()
                .map(|p| (table_rank(p), *p))
                .max_by(|a, b| a.0.cmp(&b.0))
                .map(|(_, p)| p.clone())
                .unwrap_or_else(|| copies[0].clone())
        };
        winners.insert(dest.clone(), winner);
    }

    let total = index.tables.len();
    for (i, vpx_src) in index.tables.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let Some(stem) = vpx_src.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let stem = stem.to_string();

        let table_dir = destination_dir(vpx_src, &config.output_root);
        // The .vpx needs placing only when it is not already sitting in the
        // folder it belongs to. Asking whether the destination differs from
        // `output_root/stem` answered a different question, and got it wrong
        // for the commonest layout of all — a folder named after its table —
        // where it asked for the file to be placed on top of itself. Unix
        // let that through; Windows returned a sharing violation, which
        // aborted the table and left an in-place merge processing nothing.
        let already_foldered = vpx_src
            .parent()
            .is_some_and(|p| canonical(p) == canonical(&table_dir));

        if let Some(kept) = winners.get(&destinations[i]) {
            if kept != vpx_src {
                sink.report.tables_skipped += 1;
                let _ = tx.send(MergeEvent::TableSkipped {
                    name: stem,
                    index: i + 1,
                    total,
                    src: vpx_src.clone(),
                    kept: kept.clone(),
                });
                continue;
            }
        }

        let _ = tx.send(MergeEvent::TableStarted {
            name: stem.clone(),
            index: i + 1,
            total,
        });

        // The table file itself, unless it is already in place.
        let mut effective_vpx = vpx_src.clone();
        if !already_foldered {
            let vpx_dst = table_dir.join(vpx_src.file_name().unwrap_or_default());
            sink.report.assets_found += 1;
            let _ = tx.send(MergeEvent::AssetFound {
                kind: AssetKind::Vpx,
                src: vpx_src.clone(),
                dst: vpx_dst.clone(),
            });
            if matches!(config.mode, MergeMode::DryRun) {
                let _ = tx.send(MergeEvent::AssetSkipped {
                    kind: AssetKind::Vpx,
                    reason: SkipReason::DryRun,
                });
            } else if let Err(e) = std::fs::create_dir_all(&table_dir) {
                sink.report.assets_errored += 1;
                let _ = tx.send(MergeEvent::AssetError {
                    kind: AssetKind::Vpx,
                    msg: format!("create {}: {e}", table_dir.display()),
                });
                let _ = tx.send(MergeEvent::TableDone { name: stem });
                continue;
            } else {
                // Folderizing inside the output root always moves: a copy
                // would leave a duplicate table beside its own folder.
                let loose_in_output = vpx_src
                    .parent()
                    .is_some_and(|p| canonical(p) == canonical(&config.output_root));
                let placed = if loose_in_output {
                    MoveOp.place_file(vpx_src, &vpx_dst)
                } else {
                    sink.op.place_file(vpx_src, &vpx_dst)
                };
                match placed {
                    Ok(()) => {
                        sink.report.assets_applied += 1;
                        let _ = tx.send(MergeEvent::AssetApplied {
                            kind: AssetKind::Vpx,
                            dst: vpx_dst.clone(),
                        });
                        effective_vpx = vpx_dst;
                    }
                    Err(e) => {
                        sink.report.assets_errored += 1;
                        let _ = tx.send(MergeEvent::AssetError {
                            kind: AssetKind::Vpx,
                            msg: e.to_string(),
                        });
                        let _ = tx.send(MergeEvent::TableDone { name: stem });
                        continue;
                    }
                }
            }
        }

        sink.table_dir = table_dir.clone();
        process_table(&table_dir, vpx_src, &effective_vpx, &index, &mut sink);
        sink.report.tables_processed += 1;
        let _ = tx.send(MergeEvent::TableDone { name: stem });
    }

    let _ = tx.send(MergeEvent::Done(sink.report.clone()));
    Ok(())
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

/// Where a table's bundle goes: its own folder when it already sits one
/// level under the output root, a fresh folder named after it otherwise.
fn destination_dir(vpx: &Path, output_root: &Path) -> PathBuf {
    let already_foldered = vpx
        .parent()
        .and_then(Path::parent)
        .is_some_and(|gp| canonical(gp) == canonical(output_root));
    if already_foldered {
        vpx.parent().unwrap_or(output_root).to_path_buf()
    } else {
        let stem = vpx.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        output_root.join(stem)
    }
}

/// Modification time, oldest-possible when unreadable so a file we cannot
/// stat never wins a tie.
fn modified_at(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// How good one copy of a table is, most significant first.
///
/// The file date only says when it was last written — a fresh copy of an
/// old table beats the newer one on that measure alone. The table itself
/// knows better: authors bump `table_version`, and VPX increments
/// `table_save_rev` on every save. Both are read from inside the .vpx,
/// so this costs a file open and is only computed for duplicates.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct TableRank {
    /// Declared version, split into numbers so "1.10" beats "1.9".
    /// `None` sorts first: a copy that declares one wins over one that
    /// does not.
    version: Option<Vec<u32>>,
    save_rev: Option<u64>,
    modified: std::time::SystemTime,
}

fn numeric_version(raw: &str) -> Option<Vec<u32>> {
    let parts: Vec<u32> = raw
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    (!parts.is_empty()).then_some(parts)
}

fn table_rank(vpx: &Path) -> TableRank {
    let info = vpin::vpx::open(vpx)
        .ok()
        .and_then(|mut v| v.read_tableinfo().ok());
    TableRank {
        version: info
            .as_ref()
            .and_then(|i| i.table_version.as_deref())
            .and_then(numeric_version),
        save_rev: info
            .as_ref()
            .and_then(|i| i.table_save_rev.as_deref())
            .and_then(|r| r.trim().parse().ok()),
        modified: modified_at(vpx),
    }
}

/// Bundle one table. `vpx_src` is where it was found — its neighbours are
/// the strongest hint about which of several same-named assets belongs to
/// it — while `vpx_path` is where it now lives (the same path in place).
fn process_table(
    table_dir: &Path,
    vpx_src: &Path,
    vpx_path: &Path,
    index: &AssetIndex,
    sink: &mut Sink,
) {
    let base = vpx_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let (rom_via_meta, table_name_embedded) = crate::vpsdb::matcher::read_vpx_meta(vpx_path);
    let near = vpx_src.parent();

    // Sidecar .vbs — both an asset to place and the source of the
    // pGameName / cPuPPack hints. Falls back silently if the user only
    // ships .vpx without a sidecar.
    let vbs_name = format!("{base}.vbs");
    let vbs_src = index.file(&vbs_name, &[], near);
    let sidecar_text = vbs_src
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| std::fs::read_to_string(table_dir.join(&vbs_name)).ok())
        .unwrap_or_default();
    let pgame_names = extract_pgame_names(&sidecar_text);
    let cpup_pack = extract_cpup_pack(&sidecar_text);

    // Legacy collections often ship the script *beside* the table rather
    // than embedded in it, so the ROM name has to come from the sidecar.
    // Without it, half the bundle (ROM, altsound, colorization, nvram,
    // cfg) can never be resolved.
    let rom = rom_via_meta.or_else(|| crate::vpsdb::matcher::extract_cgamename(&sidecar_text));

    let ctx = TableContext {
        rom,
        table_name_embedded,
        pgame_names,
        cpup_pack,
        base,
    };

    // 0. Script
    sink.file(AssetKind::Vbs, vbs_src, table_dir.join(&vbs_name));

    // 1. ROM
    if let Some(rom) = &ctx.rom {
        sink.file(
            AssetKind::Rom,
            index.file(&format!("{rom}.zip"), &["roms"], near),
            table_dir.join("pinmame/roms").join(format!("{rom}.zip")),
        );
    } else {
        sink.skip(AssetKind::Rom, SkipReason::SourceMissing);
    }

    // 2. .directb2s
    {
        let b2s_name = format!("{}.directb2s", ctx.base);
        sink.file(
            AssetKind::Directb2s,
            index.file(&b2s_name, &[], near),
            table_dir.join(&b2s_name),
        );
    }

    // 3. POV .ini — same stem as the table, so a stray VPinballX.ini
    // elsewhere on the disk can never be mistaken for one.
    {
        let ini_name = format!("{}.ini", ctx.base);
        sink.file(
            AssetKind::PovIni,
            index.file(&ini_name, &[], near),
            table_dir.join(&ini_name),
        );
    }

    // 4. AltSound — a folder of audio plus its .csv manifest, named
    // after the ROM.
    if let Some(rom) = &ctx.rom {
        let src = index.dir_in(rom, &index.altsound_dirs);
        sink.dir(
            AssetKind::AltSound,
            src,
            table_dir.join("pinmame/altsound").join(rom),
        );
    } else {
        sink.skip(AssetKind::AltSound, SkipReason::SourceMissing);
    }

    // 5. AltColor (.vni) and 6. Serum (.crz) — keyed by ROM, table name
    // or any pGameName the script declares.
    let color_keys: Vec<String> = std::iter::once(ctx.rom.clone().unwrap_or_default())
        .chain(std::iter::once(ctx.base.clone()))
        .chain(ctx.pgame_names.iter().cloned())
        .filter(|s| !s.is_empty())
        .collect();
    let primary_key = ctx
        .rom
        .clone()
        .or_else(|| color_keys.first().cloned())
        .unwrap_or_default();
    let color_dir = color_keys
        .iter()
        .find_map(|key| index.dir_in(key, &index.altcolor_dirs));
    sink.dir(
        AssetKind::AltColorVni,
        color_dir.clone(),
        table_dir.join("vni").join(&primary_key),
    );

    let crz = color_keys
        .iter()
        .find_map(|key| index.file(&format!("{key}.crz"), &[], near))
        .or_else(|| {
            color_dir
                .as_deref()
                .and_then(|d| first_with_extension(d, "crz"))
        });
    if let Some(src) = crz {
        let dst = table_dir.join("serum").join(
            src.file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("colorize.crz")),
        );
        sink.file(AssetKind::Serum, Some(src), dst);
    } else {
        sink.skip(AssetKind::Serum, SkipReason::SourceMissing);
    }

    // 7. PUP pack — every folder carrying a playlists.pup marker is a
    // candidate, wherever it sits; the name is matched fuzzily.
    {
        let names: Vec<String> = index
            .pup_packs
            .iter()
            .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(str::to_string))
            .collect();
        let targets: Vec<&str> = std::iter::empty::<&str>()
            .chain(ctx.cpup_pack.iter().map(|s| s.as_str()))
            .chain(ctx.pgame_names.iter().map(|s| s.as_str()))
            .chain(ctx.table_name_embedded.iter().map(|s| s.as_str()))
            .chain(std::iter::once(ctx.base.as_str()))
            .chain(ctx.rom.iter().map(|s| s.as_str()))
            .collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let matched = fuzzy::find_pup_folder(&targets, &name_refs).and_then(|name| {
            index
                .pup_packs
                .iter()
                .find(|p| p.file_name().and_then(|s| s.to_str()) == Some(name.as_str()))
                .map(|p| (name, p.clone()))
        });
        match matched {
            Some((name, src)) => sink.dir(
                AssetKind::PupPack,
                Some(src),
                table_dir.join("pupvideos").join(&name),
            ),
            None => sink.skip(AssetKind::PupPack, SkipReason::SourceMissing),
        }
    }

    // 8. NVRAM + CFG
    if let Some(rom) = &ctx.rom {
        sink.file(
            AssetKind::Nvram,
            index.file(&format!("{rom}.nv"), &["nvram"], near),
            table_dir.join("pinmame/nvram").join(format!("{rom}.nv")),
        );
        sink.file(
            AssetKind::Cfg,
            index.file(&format!("{rom}.cfg"), &["cfg"], near),
            table_dir.join("pinmame/cfg").join(format!("{rom}.cfg")),
        );
    } else {
        sink.skip(AssetKind::Nvram, SkipReason::SourceMissing);
        sink.skip(AssetKind::Cfg, SkipReason::SourceMissing);
    }

    // 9. Music — a folder of audio without an altsound manifest, named
    // after the table or its ROM.
    {
        let src = std::iter::once(ctx.base.as_str())
            .chain(ctx.rom.iter().map(|s| s.as_str()))
            .find_map(|key| index.dir_in(key, &index.music_dirs));
        sink.dir(
            AssetKind::Music,
            src,
            table_dir.join("music").join(&ctx.base),
        );
    }
}

// ---------------------------------------------------------------------------
// Placement helpers
// ---------------------------------------------------------------------------

/// Bump the skip tally and say why. Used by the placement methods, which
/// hold a split borrow of the sink and so cannot call `Sink::skip`.
fn report_skipped(
    kind: AssetKind,
    reason: SkipReason,
    tx: &Sender<MergeEvent>,
    report: &mut MergeReport,
) {
    match reason {
        // Neither is an outcome the user chose: one is an absence, the
        // other is the mode itself. Counting them made "skipped" the
        // biggest number on screen and the least meaningful.
        SkipReason::SourceMissing => report.assets_absent += 1,
        SkipReason::DryRun => {}
        SkipReason::AlreadyPresent => report.assets_skipped += 1,
    }
    let _ = tx.send(MergeEvent::AssetSkipped { kind, reason });
}

/// True when the file being placed already lives inside the table's own
/// folder. The placement is then a tidy-up, not an import: it moves the
/// file into the canonical subfolder instead of leaving a duplicate
/// behind. This is what lets a collection that is already
/// folder-per-table — but keeps its ROMs and packs loose, or in the
/// discouraged `roms/`-at-the-root layout — be put back in order.
fn is_tidying(table_dir: &Path, src: &Path) -> bool {
    !table_dir.as_os_str().is_empty() && src.starts_with(table_dir)
}

/// Everything a placement needs: the mode, the I/O strategy, the event
/// channel and the running tally. Owning the report keeps the borrow
/// checker out of the per-table loop.
struct Sink<'a> {
    mode: MergeMode,
    op: Box<dyn FsOp>,
    tx: &'a Sender<MergeEvent>,
    report: MergeReport,
    /// Folder of the table being bundled, so a file already inside it can
    /// be recognised as tidying rather than importing.
    table_dir: PathBuf,
}

impl Sink<'_> {
    fn file(&mut self, kind: AssetKind, src: Option<PathBuf>, dst: PathBuf) {
        static TIDY: MoveOp = MoveOp;
        let tidy = src
            .as_deref()
            .is_some_and(|s| is_tidying(&self.table_dir, s));
        let (mode, tx, report) = (self.mode, self.tx, &mut self.report);
        let op: &dyn FsOp = if tidy { &TIDY } else { &*self.op };
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

    fn dir(&mut self, kind: AssetKind, src: Option<PathBuf>, dst: PathBuf) {
        static TIDY: MoveOp = MoveOp;
        let tidy = src
            .as_deref()
            .is_some_and(|s| is_tidying(&self.table_dir, s));
        let (mode, tx, report) = (self.mode, self.tx, &mut self.report);
        let op: &dyn FsOp = if tidy { &TIDY } else { &*self.op };
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

    fn skip(&mut self, kind: AssetKind, reason: SkipReason) {
        report_skipped(kind, reason, self.tx, &mut self.report);
    }
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

#[cfg(test)]
mod run_tests {
    use super::*;

    fn fresh_dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("pinready-merge-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(path: PathBuf, body: &[u8]) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    fn run_and_collect(config: MergeConfig) -> (MergeReport, Vec<MergeEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        run(&config, &tx, &cancel).unwrap();
        drop(tx);
        let events: Vec<MergeEvent> = rx.iter().collect();
        let report = events
            .iter()
            .find_map(|e| match e {
                MergeEvent::Done(r) => Some(r.clone()),
                _ => None,
            })
            .expect("Done event");
        (report, events)
    }

    fn config(scan: &Path, out: &Path, mode: MergeMode) -> MergeConfig {
        MergeConfig {
            scan_root: scan.to_path_buf(),
            output_root: out.to_path_buf(),
            strategy: MergeStrategy::Copy,
            mode,
        }
    }

    /// The whole point of the single-root model: companions are found
    /// wherever they happen to live, with nobody naming their folders.
    #[test]
    fn one_root_finds_assets_scattered_anywhere() {
        let source = fresh_dir("scatter-src");
        let out = fresh_dir("scatter-out");
        let table = "Apollo 13 (Sega 1995)";
        write(source.join(format!("dump/tables/{table}.vpx")), b"stub");
        write(source.join(format!("dump/tables/{table}.vbs")), b"' script");
        write(
            source.join(format!("backglasses/{table}.directb2s")),
            b"<b2s/>",
        );
        write(source.join(format!("povs/{table}.ini")), b"[POV]");
        write(source.join("PUPVideos/Apollo13/playlists.pup"), b"x");
        write(source.join(format!("Music/{table}/theme.ogg")), b"ogg");

        let (report, _) = run_and_collect(config(&source, &out, MergeMode::Commit));
        assert_eq!(report.tables_processed, 1);
        let bundle = out.join(table);
        assert!(bundle.join(format!("{table}.vpx")).is_file());
        assert!(bundle.join(format!("{table}.vbs")).is_file());
        assert!(bundle.join(format!("{table}.directb2s")).is_file());
        assert!(bundle.join(format!("{table}.ini")).is_file());
        assert!(bundle.join("pupvideos/Apollo13/playlists.pup").is_file());
        assert!(bundle.join(format!("music/{table}/theme.ogg")).is_file());
        // Copy leaves the collection untouched.
        assert!(source.join(format!("dump/tables/{table}.vpx")).is_file());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A folder of audio *with* a manifest is an altsound pack, not a
    /// music set — the only thing that tells them apart is the .csv.
    #[test]
    fn audio_folder_with_a_manifest_is_not_music() {
        let root = fresh_dir("classify");
        write(root.join("altsound/apollo13/altsound.csv"), b"x");
        write(root.join("altsound/apollo13/1.ogg"), b"x");
        write(root.join("Music/Some Table/track.ogg"), b"x");

        let cancel = Arc::new(AtomicBool::new(false));
        let index = build_index(&root, None, &cancel, |_, _, _| {});
        assert!(index.dir_in("apollo13", &index.altsound_dirs).is_some());
        assert!(index.dir_in("apollo13", &index.music_dirs).is_none());
        assert!(index.dir_in("Some Table", &index.music_dirs).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Legacy collections keep the script beside the table. Without
    /// reading the ROM name from it, the ROM-keyed half of the bundle
    /// (ROM, altsound, nvram, cfg, colorization) stays unresolvable.
    #[test]
    fn rom_name_comes_from_the_sidecar_script() {
        let source = fresh_dir("sidecar-rom");
        let out = fresh_dir("sidecar-rom-out");
        let table = "Medieval Madness (Williams 1997)";
        write(source.join(format!("tables/{table}.vpx")), b"stub");
        write(
            source.join(format!("tables/{table}.vbs")),
            b"Const cGameName = \"mm_109c\"\n",
        );
        write(source.join("emu/VPinMAME/roms/mm_109c.zip"), b"zip");
        write(source.join("emu/VPinMAME/nvram/mm_109c.nv"), b"nv");

        let (report, _) = run_and_collect(config(&source, &out, MergeMode::Commit));
        assert_eq!(report.tables_processed, 1);
        let bundle = out.join(table);
        assert!(bundle.join("pinmame/roms/mm_109c.zip").is_file());
        assert!(bundle.join("pinmame/nvram/mm_109c.nv").is_file());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Modern layout: tables stay exactly where they are, and only the
    /// missing companion is pulled in.
    #[test]
    fn in_place_completes_without_moving_the_table() {
        let root = fresh_dir("inplace");
        let table = "MM (Williams 1997)";
        let vpx = write(root.join(format!("{table}/{table}.vpx")), b"stub");
        write(root.join(format!("loose-b2s/{table}.directb2s")), b"<b2s/>");

        let cfg = config(&root, &root, MergeMode::Commit);
        assert!(cfg.is_in_place());
        let (report, _) = run_and_collect(cfg);
        assert_eq!(
            report.tables_processed,
            1,
            "root={root:?} vpx={vpx:?} indexed={} skipped={} samples={} exists={}",
            report.files_indexed,
            report.tables_skipped,
            report.tables_sample_skipped,
            vpx.is_file()
        );
        assert!(vpx.is_file(), "an in-place table must not be moved");
        assert!(root.join(format!("{table}/{table}.directb2s")).is_file());
        assert!(!root.join(format!("{table}/{table}/{table}.vpx")).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A collection already folder-per-table but keeping its ROM loose
    /// (or in the discouraged roms/-at-the-root layout) gets tidied: the
    /// file moves into the canonical subfolder instead of being copied,
    /// which would leave the mess behind next to the tidy version.
    #[test]
    fn in_place_tidies_files_already_inside_the_table_folder() {
        let root = fresh_dir("tidy");
        let table = "Medieval Madness (Williams 1997)";
        write(root.join(format!("{table}/{table}.vpx")), b"stub");
        write(
            root.join(format!("{table}/{table}.vbs")),
            b"Const cGameName = \"mm_109c\"\n",
        );
        let loose_rom = write(root.join(format!("{table}/roms/mm_109c.zip")), b"zip");

        let (report, _) = run_and_collect(config(&root, &root, MergeMode::Commit));
        assert_eq!(report.tables_processed, 1);
        assert_eq!(
            std::fs::read(root.join(format!("{table}/{table}.vpx"))).unwrap(),
            b"stub",
            "an in-place table must not be rewritten"
        );
        assert!(root
            .join(format!("{table}/pinmame/roms/mm_109c.zip"))
            .is_file());
        assert!(
            !loose_rom.exists(),
            "tidying moves the file, it does not leave a copy behind"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Between two copies, the table's own declared version outranks the
    /// file date: a fresh copy of an old table is still the old table.
    #[test]
    fn a_declared_version_outranks_the_file_date() {
        let older = TableRank {
            version: numeric_version("1.9"),
            save_rev: Some(400),
            modified: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000),
        };
        let newer = TableRank {
            version: numeric_version("1.10"),
            save_rev: Some(12),
            modified: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000),
        };
        assert!(newer > older, "1.10 is a later version than 1.9");

        // No declared version at all loses to one that has it.
        let undeclared = TableRank {
            version: None,
            save_rev: None,
            modified: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(9_999),
        };
        assert!(older > undeclared);
    }

    /// Loose tables at the output root are folderized by moving: copying
    /// would leave a duplicate beside its own folder.
    #[test]
    fn loose_table_at_the_output_root_is_moved_into_its_folder() {
        let root = fresh_dir("folderize");
        let table = "Apollo 13 (Sega 1995)";
        write(root.join(format!("{table}.vpx")), b"stub");
        write(root.join(format!("{table}.vbs")), b"' s");

        let (report, _) = run_and_collect(config(&root, &root, MergeMode::Commit));
        assert_eq!(report.tables_processed, 1);
        assert!(root.join(format!("{table}/{table}.vpx")).is_file());
        assert!(root.join(format!("{table}/{table}.vbs")).is_file());
        assert!(!root.join(format!("{table}.vpx")).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two tables of the same name would land in the same bundle: the
    /// first wins and the second is reported, never silently overwritten.
    #[test]
    fn duplicate_table_names_are_surfaced() {
        let source = fresh_dir("dup-src");
        let out = fresh_dir("dup-out");
        write(source.join("a/TZ (Bally 1993).vpx"), b"first");
        write(source.join("b/TZ (Bally 1993).vpx"), b"second");

        let (report, events) = run_and_collect(config(&source, &out, MergeMode::Commit));
        assert_eq!(report.tables_processed, 1);
        assert_eq!(report.tables_skipped, 1);
        assert!(events
            .iter()
            .any(|e| matches!(e, MergeEvent::TableSkipped { .. })));
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// The output lives inside the scanned drive more often than not.
    /// Indexing it would re-import our own result as a source.
    #[test]
    fn the_output_subtree_is_never_indexed_as_a_source() {
        let source = fresh_dir("skip-out");
        let out = source.join("Tables");
        std::fs::create_dir_all(&out).unwrap();
        write(out.join("Old (Bally 1980)/Old (Bally 1980).vpx"), b"stub");
        write(source.join("new/Fresh (Gottlieb 1985).vpx"), b"stub");

        let (report, _) = run_and_collect(config(&source, &out, MergeMode::Commit));
        assert_eq!(
            report.tables_processed, 1,
            "only the table outside the output root is imported"
        );
        assert!(out.join("Fresh (Gottlieb 1985)").is_dir());
        let _ = std::fs::remove_dir_all(&source);
    }

    /// VPX ships four sample tables and an old drive holds one copy per
    /// install. They are noise, and the current VPX build provides them.
    #[test]
    fn vpx_sample_tables_are_left_out() {
        let source = fresh_dir("samples");
        let out = fresh_dir("samples-out");
        for name in [
            "blankTable",
            "exampleTable",
            "lightSeqTable",
            "strippedTable",
            "FlexDemo",
            "JP's VPX8 Physics Rev3.1 Elasticity_Test",
        ] {
            write(source.join(format!("install/assets/{name}.vpx")), b"stub");
        }
        write(source.join("mine/TZ (Bally 1993).vpx"), b"stub");

        let (report, _) = run_and_collect(config(&source, &out, MergeMode::Commit));
        assert_eq!(
            report.tables_processed, 1,
            "only the real table is imported"
        );
        assert_eq!(report.tables_sample_skipped, 6);
        assert!(out.join("TZ (Bally 1993)").is_dir());
        assert!(!out.join("blankTable").exists());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Among several copies of one table, the one the user has been
    /// playing is the most recently modified — importing an older backup
    /// instead would be a silent downgrade.
    #[test]
    fn the_newest_copy_of_a_duplicate_wins() {
        let source = fresh_dir("newest");
        let out = fresh_dir("newest-out");
        let old = write(source.join("backup/TZ (Bally 1993).vpx"), b"old");
        let new = write(source.join("current/TZ (Bally 1993).vpx"), b"new");
        let base =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(base)
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&new)
            .unwrap()
            .set_modified(base + std::time::Duration::from_secs(86_400))
            .unwrap();

        let (report, events) = run_and_collect(config(&source, &out, MergeMode::Commit));
        assert_eq!(report.tables_processed, 1);
        assert_eq!(report.tables_skipped, 1);
        assert_eq!(
            std::fs::read_to_string(out.join("TZ (Bally 1993)/TZ (Bally 1993).vpx")).unwrap(),
            "new"
        );
        // The dropped copy is named, so nothing vanishes quietly.
        assert!(events.iter().any(|e| matches!(
            e,
            MergeEvent::TableSkipped { src, kept, .. } if src == &old && kept == &new
        )));
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Dry run must not touch the disk but still report the work.
    #[test]
    fn dry_run_reports_without_touching_disk() {
        let source = fresh_dir("dryrun-src");
        let out = fresh_dir("dryrun-out");
        write(source.join("MM (Williams 1997).vpx"), b"stub");

        let (report, _) = run_and_collect(config(&source, &out, MergeMode::DryRun));
        assert_eq!(report.tables_processed, 1);
        assert!(
            report.assets_found >= 1,
            "the vpx placement must be reported"
        );
        assert_eq!(report.assets_applied, 0);
        assert!(!out.join("MM (Williams 1997)").exists());
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&out);
    }
}
