//! Small per-table pictures the launcher layouts load on demand: the wheels
//! (round or shaped logos with transparency) and the backglasses.
//!
//! A wheel is `medias/wheel.png` (VPinMediaDB's name) or a frontend's
//! `medias/(Wheel) <name>.png`. When neither exists and downloads are allowed,
//! it is fetched from VPinMediaDB the first time the table is shown, then kept
//! in the table's folder.
//!
//! A backglass is `medias/bg.png` (VPinMediaDB) or the `.directb2s` image,
//! never the launcher picture: the arcade's backboxes show the real
//! backglass even when the tiles use the user's own pictures.
//!
//! Decoding (and the download) happens on one worker thread per cache; the
//! UI thread uploads the result, a few textures per frame. The resized
//! picture is kept on disk (`<data dir>/pinready/art/<kind>/`, JPEG, or
//! lossless WebP when it has transparency), named after the source file, its
//! size and its date: the next sessions read that small file instead of
//! decoding a large PNG or a `.directb2s` again, and a changed source gets a
//! new name. "Rebuild" empties it.

use crossbeam_channel::{Receiver, Sender};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Largest side of a wheel texture.
const WHEEL_SIZE: u32 = 512;
/// Largest side of a backglass texture.
const BACKGLASS_SIZE: u32 = 1024;
/// Largest side of a playfield still.
const PLAYFIELD_SIZE: u32 = 768;
/// Largest side of a DMD picture.
const DMD_SIZE: u32 = 512;
/// Pictures uploaded per frame.
const UPLOADS_PER_FRAME: usize = 8;

/// What a cache holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtKind {
    Wheel,
    Backglass,
    /// The playfield picture (`medias/table.png`, else a still of the
    /// playfield clip), for the arcade's unselected cabinets.
    Playfield,
    /// The DMD picture (`medias/dmd.png`, else a still of the DMD clip), for
    /// the arcade's unselected cabinets.
    Dmd,
}

impl ArtKind {
    /// Largest side of the texture.
    fn max_size(self) -> u32 {
        match self {
            ArtKind::Wheel => WHEEL_SIZE,
            ArtKind::Backglass => BACKGLASS_SIZE,
            ArtKind::Playfield => PLAYFIELD_SIZE,
            ArtKind::Dmd => DMD_SIZE,
        }
    }

    fn name(self) -> &'static str {
        match self {
            ArtKind::Wheel => "wheel",
            ArtKind::Backglass => "backglass",
            ArtKind::Playfield => "playfield",
            ArtKind::Dmd => "dmd",
        }
    }
}

struct Request {
    generation: u64,
    idx: usize,
    /// The table's `.vpx`.
    table: PathBuf,
    /// VPinMediaDB id, when the wheel may be downloaded.
    download: Option<String>,
}

struct Done {
    generation: u64,
    idx: usize,
    image: Option<ColorImage>,
}

/// The wheel file of a table folder, if there is one.
pub fn find_wheel(table_dir: &Path) -> Option<PathBuf> {
    let medias = table_dir.join("medias");
    let fixed = medias.join("wheel.png");
    if fixed.is_file() {
        return Some(fixed);
    }
    std::fs::read_dir(&medias)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let lower = name.to_ascii_lowercase();
            lower.starts_with("(wheel)") && (lower.ends_with(".png") || lower.ends_with(".webp"))
        })
}

/// Where the resized pictures of `kind` are kept between sessions.
fn cache_dir(kind: ArtKind) -> PathBuf {
    crate::db::default_db_path()
        .with_file_name("art")
        .join(kind.name())
}

/// Empty the on-disk cache of both kinds ("Rebuild").
pub fn clear_disk_cache() {
    let dir = cache_dir(ArtKind::Wheel);
    if let Some(parent) = dir.parent()
        && parent.is_dir()
        && let Err(e) = std::fs::remove_dir_all(parent)
    {
        log::warn!("Art cache: cannot empty {}: {e}", parent.display());
    }
}

/// The cache file name of a source file: a hash of its path, size and date,
/// so an edited or replaced picture is never served from an old entry.
fn cache_name(source: &Path, max: u32) -> Option<String> {
    use sha2::{Digest, Sha256};
    let meta = std::fs::metadata(source).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let mut h = Sha256::new();
    h.update(source.as_os_str().as_encoded_bytes());
    h.update(meta.len().to_le_bytes());
    h.update(mtime.to_le_bytes());
    h.update(max.to_le_bytes());
    let d = h.finalize();
    Some(d[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// The source file of a table's picture: the wheel file (downloaded first
/// when allowed), else `medias/bg.png` or the `.directb2s`.
fn source_file(kind: ArtKind, req: &Request) -> Option<PathBuf> {
    let table_dir = req.table.parent()?;
    match kind {
        ArtKind::Wheel => wheel_file(table_dir, req),
        ArtKind::Backglass => {
            let bg = table_dir.join("medias").join("bg.png");
            if bg.is_file() {
                return Some(bg);
            }
            let b2s = req.table.with_extension("directb2s");
            b2s.is_file().then_some(b2s)
        }
        ArtKind::Playfield => playfield_file(table_dir, req),
        ArtKind::Dmd => dmd_file(table_dir, req),
    }
}

fn load(kind: ArtKind, req: &Request) -> Option<ColorImage> {
    let source = source_file(kind, req)?;
    let cached = cache_name(&source, kind.max_size()).map(|n| cache_dir(kind).join(n));
    let img = cached
        .as_ref()
        .and_then(|c| std::fs::read(c).ok())
        .and_then(|b| image::load_from_memory(&b).ok())
        .or_else(|| {
            let img = decode_source(kind, &source)?;
            if let Some(c) = &cached
                && let Some(bytes) = crate::assets::encode_cached(&img)
            {
                let _ = std::fs::create_dir_all(cache_dir(kind));
                if let Err(e) = std::fs::write(c, bytes) {
                    log::warn!("Art cache: cannot write {}: {e}", c.display());
                }
            }
            Some(img)
        })?;
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

/// Decode a source file and fit it in the kind's size.
fn decode_source(kind: ArtKind, source: &Path) -> Option<image::DynamicImage> {
    let max = kind.max_size();
    let img = match kind {
        ArtKind::Wheel => image::open(source).ok()?,
        ArtKind::Dmd if !crate::video::is_video(source) => image::open(source).ok()?,
        // VPinMediaDB's table.png lies on its side like the cabinet clip:
        // stood up the same way (a quarter clockwise)
        ArtKind::Playfield if !crate::video::is_video(source) => {
            let img = image::open(source).ok()?;
            if img.width() > img.height() {
                img.rotate90()
            } else {
                img
            }
        }
        ArtKind::Playfield | ArtKind::Dmd => {
            // already fitted by the decoder
            return crate::video::still_frame(source, max, max, 2.0)
                .map(image::DynamicImage::ImageRgba8);
        }
        ArtKind::Backglass => {
            let bytes = if source.extension().is_some_and(|e| e == "directb2s") {
                crate::assets::extract_backglass_from_b2s(source)?
            } else {
                std::fs::read(source).ok()?
            };
            image::load_from_memory(&bytes).ok()?
        }
    };
    Some(if img.width() > max || img.height() > max {
        img.resize(max, max, image::imageops::FilterType::Triangle)
    } else {
        img
    })
}

/// The table's playfield picture: `medias/table.png`, downloaded from
/// VPinMediaDB when missing and allowed, else its playfield clip (a still is
/// taken). The picture comes first: the clip is only fetched when the table
/// is selected, so a cabinet seen before that had nothing to show and stayed
/// black once its table was left.
fn playfield_file(table_dir: &Path, req: &Request) -> Option<PathBuf> {
    let png = table_dir.join("medias").join("table.png");
    if png.is_file() {
        return Some(png);
    }
    let downloaded = req.download.as_deref().and_then(|vps_id| {
        let (url, md5) = crate::mediadb::MediaDb::cached()?.table_image_url_md5(vps_id)?;
        match crate::mediadb::fetch_asset(url, md5)
            .and_then(|b| crate::mediadb::install_asset(table_dir, "table.png", &b))
        {
            Ok(p) => {
                log::info!("Playfield picture installed: {}", p.display());
                Some(p)
            }
            Err(e) => {
                log::warn!("Playfield picture for {}: {e}", table_dir.display());
                None
            }
        }
    });
    downloaded.or_else(|| crate::video::playfield_video(table_dir))
}

/// The table's DMD picture: `medias/dmd.png`, downloaded from VPinMediaDB
/// when missing and allowed, else its DMD clip (a still is taken).
fn dmd_file(table_dir: &Path, req: &Request) -> Option<PathBuf> {
    let png = table_dir.join("medias").join("dmd.png");
    if png.is_file() {
        return Some(png);
    }
    let downloaded = req.download.as_deref().and_then(|vps_id| {
        let (url, md5) = crate::mediadb::MediaDb::cached()?.dmd_image_url_md5(vps_id)?;
        match crate::mediadb::fetch_asset(url, md5)
            .and_then(|b| crate::mediadb::install_asset(table_dir, "dmd.png", &b))
        {
            Ok(p) => {
                log::info!("DMD picture installed: {}", p.display());
                Some(p)
            }
            Err(e) => {
                log::warn!("DMD picture for {}: {e}", table_dir.display());
                None
            }
        }
    });
    downloaded.or_else(|| crate::video::find_video(table_dir, crate::video::VideoSlot::Dmd))
}

/// The table's wheel file, downloaded first when missing and allowed.
fn wheel_file(table_dir: &Path, req: &Request) -> Option<PathBuf> {
    find_wheel(table_dir).or_else(|| {
        let vps_id = req.download.as_deref()?;
        let mdb = crate::mediadb::MediaDb::cached()?;
        let (url, md5) = mdb.wheel_url_md5(vps_id)?;
        match crate::mediadb::fetch_asset(url, md5)
            .and_then(|b| crate::mediadb::install_asset(table_dir, "wheel.png", &b))
        {
            Ok(p) => {
                log::info!("Wheel installed: {}", p.display());
                Some(p)
            }
            Err(e) => {
                log::warn!("Wheel for {}: {e}", table_dir.display());
                None
            }
        }
    })
}

pub struct ArtCache {
    kind: ArtKind,
    generation: u64,
    req_tx: Sender<Request>,
    res_rx: Receiver<Done>,
    /// `Some(None)`: looked for, the table has no wheel.
    wheels: HashMap<usize, Option<TextureHandle>>,
    ready: Vec<(usize, Option<ColorImage>)>,
    inflight: std::collections::HashSet<usize>,
    /// Frame counter, and the frame each picture was last asked for: the
    /// ones off screen for a while are let go, so scrolling a large
    /// collection does not keep every texture in memory.
    frame: u64,
    used: HashMap<usize, u64>,
}

/// Pictures kept however long they were not asked for.
const KEEP: usize = 96;
/// Frames a picture stays after it was last asked for.
const IDLE_FRAMES: u64 = 180;

impl ArtCache {
    pub fn new(kind: ArtKind) -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<Request>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<Done>();
        let _ = std::thread::Builder::new()
            .name(format!("pinready-{}s", kind.name()))
            .spawn(move || {
                while let Ok(req) = req_rx.recv() {
                    let done = Done {
                        generation: req.generation,
                        idx: req.idx,
                        image: load(kind, &req),
                    };
                    if res_tx.send(done).is_err() {
                        break;
                    }
                }
            });
        Self {
            kind,
            generation: 0,
            req_tx,
            res_rx,
            wheels: HashMap::new(),
            ready: Vec::new(),
            inflight: Default::default(),
            frame: 0,
            used: HashMap::new(),
        }
    }

    /// A new scan (the tables are numbered again), or VPX starting: forget
    /// every wheel.
    pub fn reset(&mut self) {
        self.generation += 1;
        self.wheels.clear();
        self.ready.clear();
        self.inflight.clear();
        self.used.clear();
    }

    /// The picture of table `idx` (its `.vpx` is `table`), asking for it the
    /// first time. `download` is the table's VPinMediaDB id when a missing
    /// wheel may be fetched.
    pub fn get(
        &mut self,
        idx: usize,
        table: &Path,
        download: Option<&str>,
    ) -> Option<&TextureHandle> {
        self.used.insert(idx, self.frame);
        if !self.wheels.contains_key(&idx) && self.inflight.insert(idx) {
            let _ = self.req_tx.send(Request {
                generation: self.generation,
                idx,
                table: table.to_path_buf(),
                download: download.map(str::to_string),
            });
        }
        self.wheels.get(&idx).and_then(|w| w.as_ref())
    }

    /// Whether table `idx` was looked at and has no wheel.
    #[cfg(test)]
    pub fn is_missing(&self, idx: usize) -> bool {
        matches!(self.wheels.get(&idx), Some(None))
    }

    /// Every frame: take what the worker finished and upload a few.
    pub fn update(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        if self.wheels.len() > KEEP {
            let (frame, used) = (self.frame, &self.used);
            self.wheels.retain(|idx, _| {
                used.get(idx)
                    .is_some_and(|&f| frame.saturating_sub(f) < IDLE_FRAMES)
            });
            let wheels = &self.wheels;
            self.used.retain(|idx, _| wheels.contains_key(idx));
        }
        while let Ok(d) = self.res_rx.try_recv() {
            if d.generation == self.generation {
                self.inflight.remove(&d.idx);
                self.ready.push((d.idx, d.image));
            }
        }
        let n = self.ready.len().min(UPLOADS_PER_FRAME);
        for (idx, image) in self.ready.drain(..n) {
            let tex = image.map(|i| {
                ctx.load_texture(
                    format!("{}/{}/{idx}", self.kind.name(), self.generation),
                    i,
                    TextureOptions::LINEAR,
                )
            });
            self.wheels.insert(idx, tex);
        }
        if !self.ready.is_empty() || !self.inflight.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_source_gets_a_new_cache_name() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("wheel.png");
        std::fs::write(&f, b"one").unwrap();
        let a = cache_name(&f, 512).unwrap();
        assert_eq!(a, cache_name(&f, 512).unwrap());
        assert_ne!(a, cache_name(&f, 1024).unwrap());
        std::fs::write(&f, b"longer").unwrap();
        assert_ne!(a, cache_name(&f, 512).unwrap());
        assert!(cache_name(&dir.path().join("none.png"), 512).is_none());
    }

    #[test]
    fn finds_the_mediadb_name_then_a_frontend_name() {
        let root = std::env::temp_dir().join(format!("pinready-wheel-{}", std::process::id()));
        let medias = root.join("medias");
        std::fs::create_dir_all(&medias).unwrap();
        assert!(find_wheel(&root).is_none());
        std::fs::write(medias.join("(Wheel) Attack from Mars.png"), b"x").unwrap();
        assert!(
            find_wheel(&root)
                .unwrap()
                .ends_with("(Wheel) Attack from Mars.png")
        );
        std::fs::write(medias.join("wheel.png"), b"x").unwrap();
        assert!(find_wheel(&root).unwrap().ends_with("wheel.png"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_table_without_wheel_is_reported_missing() {
        let ctx = egui::Context::default();
        let mut cache = ArtCache::new(ArtKind::Wheel);
        let dir = std::env::temp_dir()
            .join("pinready-wheel-none-does-not-exist")
            .join("t.vpx");
        assert!(cache.get(3, &dir, None).is_none());
        for _ in 0..200 {
            cache.update(&ctx);
            if cache.is_missing(3) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(cache.is_missing(3));
    }
}
