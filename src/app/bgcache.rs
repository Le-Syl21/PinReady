//! Where the backglass pictures live while the launcher runs.
//!
//! Three levels, so that moving through the grid never waits for a decode:
//!
//! 1. **Thumbnails** (`assets::THUMB_WIDTH`×`THUMB_HEIGHT`, made once and
//!    stored in the database): one texture in the GPU for *every* table.
//!    ~0.74 MB each.
//! 2. **The page on screen**: the full-size picture of every visible tile,
//!    as a GPU texture, so the backglass cover follows the selection with
//!    no delay whichever way it moves (a column, a row, a page).
//! 3. **The previous and next pages**: the full-size pictures already
//!    decoded, in RAM; they go to the GPU the moment the page changes.
//!
//! Everything is decoded by a small pool of threads (the database read of a
//! full picture too), the UI thread only uploads a few textures per frame.
//! Before this, the grid asked egui for the full 1280×1024 picture of every
//! card it drew (all of them, visible or not): 5 MB of GPU memory each, so a
//! thousand tables filled an 8 GB card in seven seconds and glutin answered
//! `out of memory`.

use crate::db::Database;
use crossbeam_channel::{Receiver, Sender};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Thumbnails sent to the GPU per frame (0.74 MB each).
const THUMB_UPLOADS_PER_FRAME: usize = 48;
/// Full-size pictures sent to the GPU per frame (5 MB each).
const FULL_UPLOADS_PER_FRAME: usize = 6;
/// Decoding threads.
const WORKERS: usize = 4;

enum Request {
    /// Decode a thumbnail whose bytes are at hand.
    Thumb {
        generation: u64,
        idx: usize,
        bytes: Arc<[u8]>,
    },
    /// Read a full picture from the database and decode it.
    Full {
        generation: u64,
        idx: usize,
        rel_path: String,
    },
}

struct Decoded {
    generation: u64,
    idx: usize,
    full: bool,
    image: ColorImage,
}

/// The decoded pixels of an encoded picture (JPEG, PNG or WebP).
fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

pub struct BgCache {
    generation: u64,
    req_tx: Sender<Request>,
    res_rx: Receiver<Decoded>,
    thumbs: Vec<Option<TextureHandle>>,
    /// Decoded thumbnails waiting for their upload.
    thumb_ready: Vec<(usize, ColorImage)>,
    /// Full pictures of the page on screen, in the GPU.
    gpu: HashMap<usize, TextureHandle>,
    /// Full pictures of the neighbouring pages, decoded, in RAM.
    ram: HashMap<usize, ColorImage>,
    /// Decoded full pictures not wanted yet by anyone, kept for the frame.
    full_ready: Vec<(usize, ColorImage)>,
    inflight_full: HashSet<usize>,
    inflight_thumbs: usize,
    /// Everything was freed for a running VPX: nothing is decoded or
    /// uploaded until [`restore`](Self::restore).
    released: bool,
}

impl BgCache {
    pub fn new() -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<Request>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<Decoded>();
        for w in 0..WORKERS {
            let req_rx = req_rx.clone();
            let res_tx = res_tx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("pinready-bgdecode-{w}"))
                .spawn(move || {
                    // Opened on the first full picture only.
                    let mut db: Option<Database> = None;
                    while let Ok(req) = req_rx.recv() {
                        let done = match req {
                            Request::Thumb {
                                generation,
                                idx,
                                bytes,
                            } => decode(&bytes).map(|image| Decoded {
                                generation,
                                idx,
                                full: false,
                                image,
                            }),
                            Request::Full {
                                generation,
                                idx,
                                rel_path,
                            } => {
                                if db.is_none() {
                                    db = Database::open(None).ok();
                                }
                                db.as_ref()
                                    .and_then(|d| d.get_backglass(&rel_path))
                                    .and_then(|(bytes, _)| decode(&bytes))
                                    .map(|image| Decoded {
                                        generation,
                                        idx,
                                        full: true,
                                        image,
                                    })
                            }
                        };
                        // A picture that does not decode is reported as an
                        // empty one so the request does not stay in flight.
                        let msg = done.unwrap_or_else(|| Decoded {
                            generation: u64::MAX,
                            idx: usize::MAX,
                            full: false,
                            image: ColorImage::default(),
                        });
                        if res_tx.send(msg).is_err() {
                            break;
                        }
                    }
                });
        }
        Self {
            generation: 0,
            req_tx,
            res_rx,
            thumbs: Vec::new(),
            thumb_ready: Vec::new(),
            gpu: HashMap::new(),
            ram: HashMap::new(),
            full_ready: Vec::new(),
            inflight_full: HashSet::new(),
            inflight_thumbs: 0,
            released: false,
        }
    }

    /// A new scan: forget everything (the textures are freed with their
    /// handles) and start numbering the tables again.
    pub fn reset(&mut self, generation: u64, tables: usize) {
        self.generation = generation;
        self.thumbs = vec![None; tables];
        self.thumb_ready.clear();
        self.gpu.clear();
        self.ram.clear();
        self.full_ready.clear();
        self.inflight_full.clear();
        self.inflight_thumbs = 0;
    }

    /// Frees every picture held in the GPU and in RAM, to leave the room to a
    /// VPX that is starting, and stops loading more until [`restore`].
    /// `keep` is the full picture of one table that stays in the GPU (the
    /// cover shown while VPX loads); release it with a second call.
    ///
    /// The numbering moves on, so the decodes still in flight are dropped when
    /// they come back.
    ///
    /// [`restore`]: Self::restore
    pub fn release(&mut self, keep: Option<usize>) {
        self.generation += 1;
        let kept = keep.and_then(|idx| self.gpu.remove(&idx).map(|tex| (idx, tex)));
        self.thumbs.iter_mut().for_each(|t| *t = None);
        self.thumb_ready.clear();
        self.gpu.clear();
        self.ram.clear();
        self.full_ready.clear();
        self.inflight_full.clear();
        self.inflight_thumbs = 0;
        self.released = true;
        if let Some((idx, tex)) = kept {
            self.gpu.insert(idx, tex);
        }
    }

    /// VPX is gone: pictures are loaded again. The thumbnails have to be
    /// queued again by the caller (they come from the database), the full
    /// pictures follow the page on screen as usual.
    pub fn restore(&mut self) {
        self.released = false;
    }

    pub fn is_released(&self) -> bool {
        self.released
    }

    /// Ask for the thumbnail of table `idx` to be decoded and uploaded.
    pub fn queue_thumb(&mut self, idx: usize, bytes: Arc<[u8]>) {
        self.inflight_thumbs += 1;
        let _ = self.req_tx.send(Request::Thumb {
            generation: self.generation,
            idx,
            bytes,
        });
    }

    pub fn thumb(&self, idx: usize) -> Option<&TextureHandle> {
        self.thumbs.get(idx).and_then(|t| t.as_ref())
    }

    /// The full-size picture of `idx` if it is in the GPU.
    pub fn full(&self, idx: usize) -> Option<TextureHandle> {
        self.gpu.get(&idx).cloned()
    }

    /// The best picture available for `idx`: the full one, else its thumbnail.
    pub fn best(&self, idx: usize) -> Option<TextureHandle> {
        self.full(idx).or_else(|| self.thumb(idx).cloned())
    }

    /// Every frame: `on_screen` are the tables of the page shown, `near`
    /// those of the pages before and after it, `rel_path` gives a table's
    /// path in the database. Takes what the decoders finished, uploads a few
    /// textures, asks for what is missing and frees what is not wanted any
    /// more.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        on_screen: &[usize],
        near: &[usize],
        rel_path: &dyn Fn(usize) -> Option<String>,
    ) {
        if self.released {
            return;
        }
        let mut got = 0usize;
        while let Ok(d) = self.res_rx.try_recv() {
            got += 1;
            if d.generation == u64::MAX {
                // Undecodable picture; one request less in flight.
                self.inflight_thumbs = self.inflight_thumbs.saturating_sub(1);
                continue;
            }
            if d.generation != self.generation {
                continue;
            }
            if d.full {
                self.inflight_full.remove(&d.idx);
                self.full_ready.push((d.idx, d.image));
            } else {
                self.inflight_thumbs = self.inflight_thumbs.saturating_sub(1);
                self.thumb_ready.push((d.idx, d.image));
            }
        }

        // Thumbnails: every one goes to the GPU, a few per frame.
        let n = self.thumb_ready.len().min(THUMB_UPLOADS_PER_FRAME);
        for (idx, image) in self.thumb_ready.drain(..n) {
            if let Some(slot) = self.thumbs.get_mut(idx) {
                *slot = Some(ctx.load_texture(
                    format!("bgthumb/{}/{idx}", self.generation),
                    image,
                    TextureOptions::LINEAR,
                ));
            }
        }

        let on_screen_set: HashSet<usize> = on_screen.iter().copied().collect();
        let near_set: HashSet<usize> = near.iter().copied().collect();

        // Full pictures decoded: the page's go to the GPU, the neighbours'
        // wait in RAM, anything else is dropped.
        for (idx, image) in std::mem::take(&mut self.full_ready) {
            if on_screen_set.contains(&idx) || near_set.contains(&idx) {
                self.ram.insert(idx, image);
            }
        }

        // The page on screen: from RAM to the GPU, a few per frame.
        let mut uploads = 0;
        for &idx in on_screen {
            if self.gpu.contains_key(&idx) {
                continue;
            }
            if uploads < FULL_UPLOADS_PER_FRAME
                && let Some(image) = self.ram.remove(&idx)
            {
                self.gpu.insert(
                    idx,
                    ctx.load_texture(
                        format!("bgfull/{}/{idx}", self.generation),
                        image,
                        TextureOptions::LINEAR,
                    ),
                );
                uploads += 1;
            }
        }

        // Ask for what is missing, the page first.
        for &idx in on_screen.iter().chain(near.iter()) {
            if self.gpu.contains_key(&idx)
                || self.ram.contains_key(&idx)
                || self.inflight_full.contains(&idx)
            {
                continue;
            }
            if let Some(rel) = rel_path(idx) {
                self.inflight_full.insert(idx);
                let _ = self.req_tx.send(Request::Full {
                    generation: self.generation,
                    idx,
                    rel_path: rel,
                });
            }
        }

        // Free what left the page and its neighbours.
        self.gpu.retain(|idx, _| on_screen_set.contains(idx));
        self.ram
            .retain(|idx, _| on_screen_set.contains(idx) || near_set.contains(idx));

        // Keep the frames coming while decoders or uploads are busy.
        if got > 0
            || !self.thumb_ready.is_empty()
            || !self.inflight_full.is_empty()
            || self.inflight_thumbs > 0
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

impl Default for BgCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_loaded_while_released_and_loading_resumes_after_restore() {
        let ctx = egui::Context::default();
        let mut cache = BgCache::new();
        cache.reset(1, 3);
        let rel = |idx: usize| Some(format!("table{idx}"));

        cache.release(None);
        assert!(cache.is_released());
        cache.update(&ctx, &[0, 1], &[2], &rel);
        assert!(
            cache.inflight_full.is_empty(),
            "a released cache must not ask for pictures"
        );

        cache.restore();
        cache.update(&ctx, &[0, 1], &[2], &rel);
        assert_eq!(cache.inflight_full.len(), 3);
    }

    #[test]
    fn release_forgets_what_was_in_flight() {
        let mut cache = BgCache::new();
        cache.reset(1, 2);
        cache.queue_thumb(0, Arc::from(&[][..]));
        let before = cache.generation;
        cache.release(None);
        assert_eq!(cache.inflight_thumbs, 0);
        assert!(cache.generation > before);
        assert!(cache.thumb(0).is_none());
    }
}
