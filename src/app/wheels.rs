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
//! UI thread uploads the result, a few textures per frame.

use crossbeam_channel::{Receiver, Sender};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Largest side of a wheel texture.
const WHEEL_SIZE: u32 = 320;
/// Largest side of a backglass texture.
const BACKGLASS_SIZE: u32 = 480;
/// Pictures uploaded per frame.
const UPLOADS_PER_FRAME: usize = 8;

/// What a cache holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtKind {
    Wheel,
    Backglass,
}

impl ArtKind {
    fn name(self) -> &'static str {
        match self {
            ArtKind::Wheel => "wheel",
            ArtKind::Backglass => "backglass",
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

fn load(kind: ArtKind, req: &Request) -> Option<ColorImage> {
    let table_dir = req.table.parent()?;
    let (img, max) = match kind {
        ArtKind::Wheel => (image::open(wheel_file(table_dir, req)?).ok()?, WHEEL_SIZE),
        ArtKind::Backglass => {
            let bytes =
                crate::assets::extract_backglass_from_vpinmediadb(table_dir).or_else(|| {
                    let b2s = req.table.with_extension("directb2s");
                    b2s.is_file()
                        .then(|| crate::assets::extract_backglass_from_b2s(&b2s))
                        .flatten()
                })?;
            (image::load_from_memory(&bytes).ok()?, BACKGLASS_SIZE)
        }
    };
    let img = if img.width() > max || img.height() > max {
        img.resize(max, max, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
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
}

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
        }
    }

    /// A new scan (the tables are numbered again), or VPX starting: forget
    /// every wheel.
    pub fn reset(&mut self) {
        self.generation += 1;
        self.wheels.clear();
        self.ready.clear();
        self.inflight.clear();
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
