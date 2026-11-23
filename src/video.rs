//! Table preview videos for the launcher: the selected tile and the backglass
//! screen play the table's clip in a loop.
//!
//! Each clip has its own decoder thread (FFmpeg through `ffmpeg-next`, linked
//! statically). Frames are scaled to RGBA to fit the box they are shown in and
//! sent through a small bounded channel, which paces the decoder; the UI
//! thread only uploads the frame that is due and schedules the next repaint.
//! Without the `video` feature the player never opens and the launcher keeps
//! showing the still pictures.

use std::path::{Path, PathBuf};

/// Video extensions looked up in `medias/`.
const VIDEO_EXTS: &[&str] = &["mp4", "webm", "mkv", "mov", "m4v", "avi"];

/// Where a clip is shown: it decides which file is preferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSlot {
    /// The selected tile: `medias/launcher.<ext>`, else the backglass video,
    /// else the playfield video (`table.mp4`, VPinMediaDB).
    Tile,
    /// The backglass screen: the frontend's `(Backglass) <name>.<ext>`, else
    /// the launcher video.
    Backglass,
    /// The DMD screen: `medias/dmd.mp4` (VPinMediaDB), else the frontend's
    /// `(DMD) <name>.<ext>`.
    Dmd,
}

pub fn is_video(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn launcher_video(medias: &Path) -> Option<PathBuf> {
    VIDEO_EXTS
        .iter()
        .map(|ext| medias.join(format!("launcher.{ext}")))
        .find(|p| p.is_file())
}

/// A clip with a fixed name (`table.mp4`, `dmd.mp4`, as VPinMediaDB names them).
fn named_video(medias: &Path, stem: &str) -> Option<PathBuf> {
    VIDEO_EXTS
        .iter()
        .map(|ext| medias.join(format!("{stem}.{ext}")))
        .find(|p| p.is_file())
}

/// Frontend media naming (PinUP Popper / VPX layout): `(Backglass) Table Name.mp4`.
fn backglass_video(medias: &Path) -> Option<PathBuf> {
    labelled_video(medias, "(backglass)")
}

fn labelled_video(medias: &Path, label: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(medias)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            is_video(p)
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_ascii_lowercase().starts_with(label))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The clip to play for a table in a slot, if its `medias/` folder has one.
pub fn find_video(table_dir: &Path, slot: VideoSlot) -> Option<PathBuf> {
    let medias = table_dir.join("medias");
    match slot {
        VideoSlot::Tile => launcher_video(&medias)
            .or_else(|| backglass_video(&medias))
            .or_else(|| named_video(&medias, "table")),
        VideoSlot::Backglass => backglass_video(&medias).or_else(|| launcher_video(&medias)),
        VideoSlot::Dmd => named_video(&medias, "dmd").or_else(|| labelled_video(&medias, "(dmd)")),
    }
}

/// The table's playfield clip: VPinMediaDB's `table.<ext>`, else a
/// frontend's `(Playfield) <name>.<ext>`.
pub fn playfield_video(table_dir: &Path) -> Option<PathBuf> {
    let medias = table_dir.join("medias");
    named_video(&medias, "table").or_else(|| labelled_video(&medias, "(playfield)"))
}

/// One picture of a clip, `at` seconds in (the first frames are often
/// black), fitted in `max_w`×`max_h` and turned upright like the player
/// does. `None` without video support, or when nothing decodes in a few
/// seconds.
pub fn still_frame(path: &Path, max_w: u32, max_h: u32, at: f64) -> Option<image::RgbaImage> {
    let (tx, rx) = crossbeam_channel::bounded(2);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if !decoder::spawn(
        path.to_path_buf(),
        max_w,
        max_h,
        is_cabinet_playfield(path),
        tx,
        stop.clone(),
    ) {
        return None;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut last: Option<Frame> = None;
    while let Ok(f) = rx.recv_deadline(deadline) {
        let done = f.pts >= at;
        last = Some(f);
        if done {
            break;
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let f = last?;
    image::RgbaImage::from_raw(f.size[0] as u32, f.size[1] as u32, f.rgba)
}

/// Largest size with the clip's aspect ratio that fits `max_w`×`max_h`, never
/// enlarged, even dimensions (what the scaler expects for YUV sources).
#[cfg_attr(not(feature = "video"), allow(dead_code))]
pub fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (0, 0);
    }
    let scale = (max_w as f64 / w as f64)
        .min(max_h as f64 / h as f64)
        .min(1.0);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    (even(w as f64 * scale), even(h as f64 * scale))
}

/// VPinMediaDB's playfield clip (`table.<ext>`) is recorded for a cabinet's
/// playfield screen: landscape, the top of the table on the left. Such a clip
/// is turned a quarter clockwise to stand upright in a tile; a portrait one is
/// already upright and left alone (decided on its size, in the decoder).
pub fn is_cabinet_playfield(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("table"))
}

/// `rgba` (`w`×`h`) turned a quarter clockwise: the result is `h`×`w`.
#[cfg_attr(not(feature = "video"), allow(dead_code))]
fn turn_clockwise(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; rgba.len()];
    for y in 0..h {
        for x in 0..w {
            // source (x, y) lands at column h-1-y, row x
            let (nx, ny) = (h - 1 - y, x);
            let s = (y * w + x) * 4;
            let d = (ny * h + nx) * 4;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    out
}

/// One decoded frame, `pts` in seconds from the start of the clip (growing
/// across loops).
struct Frame {
    pts: f64,
    size: [usize; 2],
    rgba: Vec<u8>,
}

/// A clip playing in a loop into an egui texture.
pub struct VideoPlayer {
    rx: crossbeam_channel::Receiver<Frame>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    texture: Option<egui::TextureHandle>,
    pending: Option<Frame>,
    started: Option<std::time::Instant>,
    name: String,
    /// Blurred copy and edge colour for the tile bands, made again every
    /// second of the clip (`backdrop_due` is the next clip time).
    backdrop: Option<crate::app::tile_fit::Backdrop>,
    backdrop_due: f64,
    /// With [`VideoPlayer::keep_frames`]: the frame shown last, and how many
    /// were shown, for a reader that is not the GPU (a ZeDMD).
    kept: Option<([usize; 2], Vec<u8>)>,
    keep: bool,
    shown: u64,
}

impl VideoPlayer {
    /// Start decoding `path`, frames fitted in `max_w`×`max_h`. `None` when
    /// the build has no video support. A playfield clip made for a cabinet
    /// (see [`is_cabinet_playfield`]) is turned upright.
    pub fn open(path: &Path, max_w: u32, max_h: u32, name: &str) -> Option<Self> {
        let (tx, rx) = crossbeam_channel::bounded(4);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let upright = is_cabinet_playfield(path);
        if !decoder::spawn(path.to_path_buf(), max_w, max_h, upright, tx, stop.clone()) {
            return None;
        }
        Some(Self {
            rx,
            stop,
            texture: None,
            pending: None,
            started: None,
            name: name.to_string(),
            backdrop: None,
            backdrop_due: 0.0,
            kept: None,
            keep: false,
            shown: 0,
        })
    }

    /// Keep a copy of each frame shown, for [`VideoPlayer::kept_frame`].
    pub fn keep_frames(mut self) -> Self {
        self.keep = true;
        self
    }

    /// The frame shown last (size, RGBA) and its number, counting from 1.
    pub fn kept_frame(&self) -> Option<(u64, [usize; 2], &[u8])> {
        let (size, rgba) = self.kept.as_ref()?;
        Some((self.shown, *size, rgba))
    }

    /// The last frame shown, once the first one arrived.
    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    /// The clip's blurred copy and edge colour, once a frame arrived.
    pub fn backdrop(&self) -> Option<&crate::app::tile_fit::Backdrop> {
        self.backdrop.as_ref()
    }

    /// Show the frame that is due, if any, and schedule the next repaint.
    /// Returns the texture once the first frame arrived.
    pub fn update(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        let mut due: Option<Frame> = None;
        loop {
            let frame = match self.pending.take() {
                Some(f) => f,
                None => match self.rx.try_recv() {
                    Ok(f) => f,
                    Err(_) => break,
                },
            };
            let started = *self.started.get_or_insert_with(|| {
                std::time::Instant::now() - std::time::Duration::from_secs_f64(frame.pts.max(0.0))
            });
            let now = started.elapsed().as_secs_f64();
            if frame.pts <= now {
                // late frames are skipped, only the latest due one is shown
                due = Some(frame);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(frame.pts - now));
                self.pending = Some(frame);
                break;
            }
        }
        if self.pending.is_none() {
            // the decoder has not caught up yet: look again shortly
            ctx.request_repaint_after(std::time::Duration::from_millis(10));
        }
        if let Some(f) = due {
            let image = egui::ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
            self.shown += 1;
            if self.keep {
                self.kept = Some((f.size, f.rgba));
            }
            // a loop starts over at 0: refresh then too
            if f.pts >= self.backdrop_due || f.pts + 1.0 < self.backdrop_due - 1.0 {
                self.backdrop_due = f.pts + 1.0;
                if let Some(px) = crate::app::tile_fit::Backdrop::compute(&image) {
                    match &mut self.backdrop {
                        Some(b) => {
                            b.blur.set(px.blur, egui::TextureOptions::LINEAR);
                            b.edge = px.edge;
                        }
                        None => {
                            self.backdrop = Some(crate::app::tile_fit::Backdrop::upload(
                                ctx,
                                format!("{}/blur", self.name),
                                px,
                            ))
                        }
                    }
                }
            }
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.texture =
                        Some(ctx.load_texture(&self.name, image, egui::TextureOptions::LINEAR))
                }
            }
        }
        self.texture.as_ref()
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(feature = "video")]
mod decoder {
    use super::{Frame, fit};
    use ffmpeg_next as ff;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub(super) fn spawn(
        path: PathBuf,
        max_w: u32,
        max_h: u32,
        upright: bool,
        tx: crossbeam_channel::Sender<Frame>,
        stop: Arc<AtomicBool>,
    ) -> bool {
        std::thread::Builder::new()
            .name("video-decoder".into())
            .spawn(move || {
                if let Err(e) = run(&path, max_w, max_h, upright, &tx, &stop) {
                    log::warn!("video {}: {e}", path.display());
                }
            })
            .is_ok()
    }

    fn run(
        path: &std::path::Path,
        max_w: u32,
        max_h: u32,
        upright: bool,
        tx: &crossbeam_channel::Sender<Frame>,
        stop: &AtomicBool,
    ) -> Result<(), ff::Error> {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let _ = ff::init();
            ff::util::log::set_level(ff::util::log::Level::Error);
        });
        let mut input = ff::format::input(path)?;
        let stream = input
            .streams()
            .best(ff::media::Type::Video)
            .ok_or(ff::Error::StreamNotFound)?;
        let index = stream.index();
        let time_base = f64::from(stream.time_base());
        let ctx = ff::codec::context::Context::from_parameters(stream.parameters())?;
        let mut decoder = ctx.decoder().video()?;
        // a landscape cabinet playfield is shown turned: fit its turned size,
        // scale to the unturned one, turn each frame
        let turn = upright && decoder.width() > decoder.height();
        let (w, h) = if turn {
            let (tw, th) = fit(decoder.height(), decoder.width(), max_w, max_h);
            (th, tw)
        } else {
            fit(decoder.width(), decoder.height(), max_w, max_h)
        };
        if w == 0 {
            return Err(ff::Error::InvalidData);
        }
        let mut scaler = ff::software::scaling::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ff::format::Pixel::RGBA,
            w,
            h,
            ff::software::scaling::Flags::BILINEAR,
        )?;
        log::info!(
            "video {}: {}x{} -> {w}x{h}{}",
            path.display(),
            decoder.width(),
            decoder.height(),
            if turn { ", turned upright" } else { "" }
        );
        let mut decoded = ff::frame::Video::empty();
        let mut rgba = ff::frame::Video::empty();
        // pts offset of the current loop, and the end of the clip seen so far
        let (mut offset, mut last, mut step) = (0.0f64, 0.0f64, 1.0f64 / 30.0);
        let mut sent_any = false;
        loop {
            let mut got_packet = false;
            for (s, packet) in input.packets() {
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if s.index() != index {
                    continue;
                }
                got_packet = true;
                decoder.send_packet(&packet)?;
                while decoder.receive_frame(&mut decoded).is_ok() {
                    let pts = decoded.timestamp().unwrap_or(0) as f64 * time_base;
                    if pts > last {
                        step = (pts - last).clamp(1.0 / 120.0, 0.5);
                    }
                    last = last.max(pts);
                    scaler.run(&decoded, &mut rgba)?;
                    // the scaler pads rows: copy them out tightly
                    let stride = rgba.stride(0);
                    let data = rgba.data(0);
                    let row = w as usize * 4;
                    let mut out = Vec::with_capacity(row * h as usize);
                    for y in 0..h as usize {
                        out.extend_from_slice(&data[y * stride..y * stride + row]);
                    }
                    let frame = if turn {
                        Frame {
                            pts: offset + pts,
                            size: [h as usize, w as usize],
                            rgba: super::turn_clockwise(&out, w as usize, h as usize),
                        }
                    } else {
                        Frame {
                            pts: offset + pts,
                            size: [w as usize, h as usize],
                            rgba: out,
                        }
                    };
                    if tx.send(frame).is_err() {
                        return Ok(()); // the player was dropped
                    }
                    sent_any = true;
                }
            }
            if !got_packet || !sent_any {
                return Err(ff::Error::Eof);
            }
            // end of the clip: start over, after the last frame
            offset += last + step;
            last = 0.0;
            decoder.flush();
            input.seek(0, ..)?;
        }
    }
}

#[cfg(not(feature = "video"))]
mod decoder {
    pub(super) fn spawn(
        _path: std::path::PathBuf,
        _max_w: u32,
        _max_h: u32,
        _upright: bool,
        _tx: crossbeam_channel::Sender<super::Frame>,
        _stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_turn_puts_the_left_edge_on_top() {
        // 3x2: a b c / d e f  ->  2x3: d a / e b / f c
        let px = |v: u8| [v, v, v, 255];
        let src: Vec<u8> = [1, 2, 3, 4, 5, 6].iter().flat_map(|&v| px(v)).collect();
        let out = turn_clockwise(&src, 3, 2);
        let vals: Vec<u8> = out.chunks(4).map(|c| c[0]).collect();
        assert_eq!(vals, vec![4, 1, 5, 2, 6, 3]);
    }

    #[test]
    fn only_the_playfield_clip_is_turned() {
        assert!(is_cabinet_playfield(Path::new("/t/medias/table.mp4")));
        assert!(is_cabinet_playfield(Path::new("/t/medias/Table.webm")));
        assert!(!is_cabinet_playfield(Path::new("/t/medias/launcher.mp4")));
        assert!(!is_cabinet_playfield(Path::new("/t/medias/dmd.mp4")));
    }

    /// Decodes a real clip: `PINREADY_TEST_VIDEO=<file> cargo test -- --ignored decodes`.
    #[cfg(feature = "video")]
    #[test]
    #[ignore]
    fn decodes_a_real_clip_and_loops() {
        let path = std::path::PathBuf::from(std::env::var("PINREADY_TEST_VIDEO").unwrap());
        let (tx, rx) = crossbeam_channel::bounded(4);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        assert!(decoder::spawn(path, 480, 384, false, tx, stop.clone()));
        let mut last = -1.0;
        let mut n = 0;
        let mut looped = false;
        while let Ok(f) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            assert!(f.size[0] <= 480 && f.size[1] <= 384);
            assert_eq!(f.rgba.len(), f.size[0] * f.size[1] * 4);
            assert!(f.pts > last, "pts must keep growing across loops");
            last = f.pts;
            n += 1;
            if n > 2000 || (n > 10 && f.pts > 60.0) {
                looped = true;
                break;
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        eprintln!("decoded {n} frames up to {last:.2} s");
        assert!(n > 10 && looped, "the clip must decode and keep looping");
    }

    /// A cabinet playfield clip comes out upright (portrait):
    /// `PINREADY_TEST_VIDEO=<table.mp4> PINREADY_TEST_OUT=<x.ppm> cargo test -- --ignored upright`.
    #[cfg(feature = "video")]
    #[test]
    #[ignore]
    fn turns_a_real_playfield_clip_upright() {
        let path = std::path::PathBuf::from(std::env::var("PINREADY_TEST_VIDEO").unwrap());
        let (tx, rx) = crossbeam_channel::bounded(4);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        assert!(decoder::spawn(path, 480, 960, true, tx, stop.clone()));
        let f = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            f.size[1] > f.size[0],
            "upright means portrait: {:?}",
            f.size
        );
        assert!(f.size[0] <= 480 && f.size[1] <= 960);
        if let Ok(out) = std::env::var("PINREADY_TEST_OUT") {
            let mut ppm = format!("P6 {} {} 255\n", f.size[0], f.size[1]).into_bytes();
            ppm.extend(f.rgba.chunks(4).flat_map(|c| [c[0], c[1], c[2]]));
            std::fs::write(out, ppm).unwrap();
        }
    }

    #[test]
    fn fit_keeps_the_ratio_and_never_enlarges() {
        assert_eq!(fit(1920, 1080, 480, 384), (480, 270));
        assert_eq!(fit(1080, 1920, 480, 384), (216, 384));
        assert_eq!(fit(320, 240, 480, 384), (320, 240));
        assert_eq!(fit(0, 240, 480, 384), (0, 0));
    }

    #[test]
    fn tile_prefers_the_launcher_clip_and_backglass_the_frontend_one() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let medias = dir.join("medias");
        std::fs::create_dir_all(&medias).unwrap();
        assert_eq!(find_video(&dir, VideoSlot::Tile), None);
        std::fs::write(medias.join("table.mp4"), b"").unwrap();
        assert_eq!(
            find_video(&dir, VideoSlot::Tile),
            Some(medias.join("table.mp4"))
        );
        std::fs::write(medias.join("(Backglass) Some Table.mp4"), b"").unwrap();
        let bg = medias.join("(Backglass) Some Table.mp4");
        assert_eq!(find_video(&dir, VideoSlot::Tile), Some(bg.clone()));
        std::fs::write(medias.join("launcher.webm"), b"").unwrap();
        assert_eq!(
            find_video(&dir, VideoSlot::Tile),
            Some(medias.join("launcher.webm"))
        );
        assert_eq!(find_video(&dir, VideoSlot::Backglass), Some(bg));
        assert_eq!(find_video(&dir, VideoSlot::Dmd), None);
        std::fs::write(medias.join("dmd.mp4"), b"").unwrap();
        assert_eq!(
            find_video(&dir, VideoSlot::Dmd),
            Some(medias.join("dmd.mp4"))
        );
    }
}
