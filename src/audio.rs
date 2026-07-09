use crossbeam_channel::Sender;
use sdl3_sys::everything::*;
use std::ffi::CStr;
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound3DMode {
    FrontStereo = 0,
    RearStereo = 1,
    SurroundRearLockbar = 2,
    SurroundFrontLockbar = 3,
    SsfLegacy = 4,
    SsfNew = 5,
}

impl Sound3DMode {
    /// i18n key for this mode's radio label. Translate at the call site with
    /// `t!(mode.label())` — kept UI-agnostic so this module stays free of
    /// the locale layer.
    pub fn label(&self) -> &'static str {
        match self {
            Self::FrontStereo => "audio_mode_front_stereo",
            Self::RearStereo => "audio_mode_rear_stereo",
            Self::SurroundRearLockbar => "audio_mode_surround_rear",
            Self::SurroundFrontLockbar => "audio_mode_surround_front",
            Self::SsfLegacy => "audio_mode_ssf_legacy",
            Self::SsfNew => "audio_mode_ssf_new",
        }
    }

    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::FrontStereo,
            1 => Self::RearStereo,
            2 => Self::SurroundRearLockbar,
            3 => Self::SurroundFrontLockbar,
            4 => Self::SsfLegacy,
            5 => Self::SsfNew,
            _ => Self::FrontStereo,
        }
    }

    pub fn all() -> &'static [Sound3DMode] {
        &[
            Self::FrontStereo,
            Self::RearStereo,
            Self::SurroundRearLockbar,
            Self::SurroundFrontLockbar,
            Self::SsfLegacy,
            Self::SsfNew,
        ]
    }

    /// `true` for the 2-channel modes (`FrontStereo`, `RearStereo`) where
    /// the wizard's front↔lockbar and top/bottom-speaker tests aren't
    /// meaningful — there's only a single stereo pair to validate.
    pub fn is_stereo(&self) -> bool {
        matches!(self, Self::FrontStereo | Self::RearStereo)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTestPhase {
    Idle,
}

/// Which speaker(s) to play on in 7.1 layout
/// SDL3 7.1: FL(0), FR(1), FC(2), LFE(3), BL(4), BR(5), SL(6), SR(7)
/// In SSF pincab: BL/BR = top playfield (near backglass), SL/SR = bottom (lockbar)
#[derive(Clone, Copy)]
pub enum SpeakerTarget {
    /// Front L+R (backglass speakers)
    FrontBoth,
    /// BL only — top-left exciter (near backglass, left side)
    TopLeft,
    /// BR only — top-right exciter
    TopRight,
    /// SL only — bottom-left exciter (lockbar, left side)
    BottomLeft,
    /// SR only — bottom-right exciter (lockbar, right side)
    BottomRight,
    /// All top (BL+BR)
    TopBoth,
    /// All bottom (SL+SR)
    BottomBoth,
    /// All left (BL+SL)
    LeftBoth,
    /// All right (BR+SR)
    RightBoth,
}

pub enum AudioCommand {
    /// Play on specific speaker target
    PlayOnSpeaker {
        path: String,
        target: SpeakerTarget,
    },
    /// Play an arbitrary audio file (mp3/ogg) on the front (backglass)
    /// stereo pair as a table preview. Replaces any previous preview.
    /// `volume` is 0.0..=1.0.
    PreviewStart {
        path: std::path::PathBuf,
        volume: f32,
    },
    /// Stop the current preview (clears the audio stream).
    PreviewStop,
    /// Play with hold at source, fade, hold at destination
    /// hold_start_ms: time on 'from' before fading
    /// fade_ms: crossfade duration
    /// hold_end_ms: time on 'to' after fading
    PlayBallSequence {
        path: String,
        from: SpeakerTarget,
        to: SpeakerTarget,
        hold_start_ms: u32,
        fade_ms: u32,
        hold_end_ms: u32,
    },
    /// Play music on front (backglass) with L/R pan
    StartMusic {
        path: String,
    },
    SetMusicPan {
        pan: f32,
    },
    StopMusic,
    #[allow(dead_code)]
    StopAll,
    #[allow(dead_code)]
    Quit,
}

#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub available_devices: Vec<String>,
    pub device_bg: String,
    pub device_pf: String,
    /// Snapshot of `device_pf` from the previous frame. The audio
    /// wizard page compares against it: when the user changes the
    /// playfield device dropdown, we re-detect the device's native
    /// channel count and pre-select the matching Sound3D mode.
    pub prev_device_pf: String,
    pub sound_3d_mode: Sound3DMode,
    /// Was `Sound3D` actually present in `VPinballX.ini`? `false` on
    /// a first-install (no ini) or a config that's been reset; the
    /// audio wizard page uses this to run a one-shot auto-detection
    /// on the first render so the user lands on a sensible mode
    /// instead of the unconditional `FrontStereo` default.
    pub sound_3d_from_ini: bool,
    pub music_volume: i32,
    pub sound_volume: i32,
    #[allow(dead_code)]
    pub test_phase: AudioTestPhase,
    pub music_looping: bool,
    pub music_pan: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            available_devices: Vec::new(),
            device_bg: String::new(),
            device_pf: String::new(),
            prev_device_pf: String::new(),
            sound_3d_mode: Sound3DMode::FrontStereo,
            sound_3d_from_ini: false,
            music_volume: 100,
            sound_volume: 100,
            test_phase: AudioTestPhase::Idle,
            music_looping: false,
            music_pan: 0.0,
        }
    }
}

impl AudioConfig {
    pub fn load_from_config(&mut self, config: &crate::config::VpxConfig) {
        if let Some(v) = config.get("Player", "SoundDeviceBG") {
            self.device_bg = v;
        }
        if let Some(v) = config.get("Player", "SoundDevice") {
            self.device_pf = v;
        }
        // Sync the change-tracking snapshot so the wizard's auto-detect
        // doesn't fire on the first frame just because the field
        // transitioned from default-empty to the loaded ini value.
        self.prev_device_pf = self.device_pf.clone();
        if let Some(v) = config.get_i32("Player", "Sound3D") {
            self.sound_3d_mode = Sound3DMode::from_i32(v);
            self.sound_3d_from_ini = true;
        } else {
            self.sound_3d_from_ini = false;
        }
        if let Some(v) = config.get_i32("Player", "MusicVolume") {
            self.music_volume = v;
        }
        if let Some(v) = config.get_i32("Player", "SoundVolume") {
            self.sound_volume = v;
        }
    }

    /// Query the OS-reported native channel count for `device_name`
    /// via SDL3. Empty / "Default" name → the system default device.
    /// Returns the channel count as reported by PulseAudio/PipeWire on
    /// Linux, WASAPI on Windows, CoreAudio on macOS — i.e. whatever the
    /// user has configured in their OS sound settings (Stereo / 5.1 /
    /// 7.1). `None` if the device can't be queried (disconnected,
    /// SDL3 audio subsystem not init, etc.).
    pub fn detect_native_channels(device_name: &str) -> Option<u8> {
        unsafe {
            // The audio subsystem must be init for the format query to
            // work. The audio thread normally owns this, but the wizard
            // can run before the user has triggered any audio path —
            // bump the refcount here and quit it on return.
            let needs_init = (SDL_WasInit(SDL_INIT_AUDIO) & SDL_INIT_AUDIO) == 0;
            if needs_init && !SDL_InitSubSystem(SDL_INIT_AUDIO) {
                return None;
            }

            let target_id = if device_name.is_empty() {
                SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK
            } else {
                let mut count: i32 = 0;
                let device_ids = SDL_GetAudioPlaybackDevices(&mut count);
                let mut found = SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK;
                let mut matched = false;
                if !device_ids.is_null() && count > 0 {
                    for i in 0..count as usize {
                        let id = *device_ids.add(i);
                        let name_ptr = SDL_GetAudioDeviceName(id);
                        if name_ptr.is_null() {
                            continue;
                        }
                        if CStr::from_ptr(name_ptr).to_string_lossy() == device_name {
                            found = id;
                            matched = true;
                            break;
                        }
                    }
                    SDL_free(device_ids as *mut _);
                }
                if !matched {
                    if needs_init {
                        SDL_QuitSubSystem(SDL_INIT_AUDIO);
                    }
                    return None;
                }
                found
            };

            let mut spec = std::mem::zeroed::<SDL_AudioSpec>();
            let ok = SDL_GetAudioDeviceFormat(target_id, &mut spec, std::ptr::null_mut());
            if needs_init {
                SDL_QuitSubSystem(SDL_INIT_AUDIO);
            }
            if ok && spec.channels > 0 {
                Some(spec.channels.min(255) as u8)
            } else {
                None
            }
        }
    }

    /// Map an OS-reported channel count to the Sound3D mode that best
    /// matches a typical pincab wiring. Stereo systems → mode 1
    /// (rear stereo / lockbar); 5.1 → mode 3 (surround front at
    /// lockbar); 7.1 → mode 5 (SSF new). Anything else falls back to
    /// rear stereo.
    pub fn recommended_sound_3d_mode(channels: u8) -> Sound3DMode {
        match channels {
            6 => Sound3DMode::SurroundFrontLockbar,
            8 => Sound3DMode::SsfNew,
            _ => Sound3DMode::RearStereo,
        }
    }

    pub fn save_to_config(&self, config: &mut crate::config::VpxConfig) {
        config.set_sound_device_bg(&self.device_bg);
        config.set_sound_device_pf(&self.device_pf);
        config.set_sound_3d_mode(self.sound_3d_mode as i32);
        config.set_music_volume(self.music_volume);
        config.set_sound_volume(self.sound_volume);
    }

    pub fn enumerate_devices() -> Vec<String> {
        let mut devices = Vec::new();
        unsafe {
            let mut count: i32 = 0;
            let device_ids = SDL_GetAudioPlaybackDevices(&mut count);
            if !device_ids.is_null() && count > 0 {
                for i in 0..count as usize {
                    let id = *device_ids.add(i);
                    let name_ptr = SDL_GetAudioDeviceName(id);
                    if !name_ptr.is_null() {
                        devices.push(CStr::from_ptr(name_ptr).to_string_lossy().into_owned());
                    }
                }
                SDL_free(device_ids as *mut _);
            }
            log::info!("Found {} audio playback devices", count);
        }
        devices
    }
}

const SAMPLE_RATE: usize = 44100;

/// Decode an asset and return its exact playback duration. Used by the
/// wizard's finalize path to schedule the eframe close precisely at the
/// end of the knocker sound instead of hardcoding a timeout.
pub fn asset_duration(name: &str) -> Option<std::time::Duration> {
    decode_to_mono_pcm(name)
        .map(|pcm| std::time::Duration::from_secs_f64(pcm.len() as f64 / SAMPLE_RATE as f64))
}

// Embedded audio assets
const KNOCKER_OGG: &[u8] = include_bytes!("../assets/audio/knocker.ogg");
const BALL_ROLL_OGG: &[u8] = include_bytes!("../assets/audio/ball_roll.ogg");
const MUSIC_OGG: &[u8] = include_bytes!("../assets/audio/music.ogg");

fn get_embedded_audio(name: &str) -> Option<&'static [u8]> {
    match name {
        "knocker.ogg" => Some(KNOCKER_OGG),
        "ball_roll.ogg" => Some(BALL_ROLL_OGG),
        "music.ogg" => Some(MUSIC_OGG),
        _ => None,
    }
}

/// Decode a symphonia media source stream to interleaved i16 samples,
/// returning the interleaved buffer alongside the source's channel count
/// and sample rate.
///
/// Shared refactor after the symphonia 0.5 → 0.6 API split: the three
/// PinReady decode paths (embedded mono, embedded stereo, on-disk stereo)
/// now differ only in what they do to the returned interleaved buffer.
fn decode_stream_to_interleaved_i16(
    mss: symphonia::core::io::MediaSourceStream<'static>,
    ext: &str,
) -> Option<(Vec<i16>, usize, u32)> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::meta::MetadataOptions;

    let mut hint = Hint::new();
    hint.with_extension(ext);

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()?;

    let track = format.default_track(TrackType::Audio)?;
    let track_id = track.id;
    let audio_params = track.codec_params.as_ref()?.audio()?.clone();
    let channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2)
        .max(1);
    let sample_rate = audio_params.sample_rate.unwrap_or(44100);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .ok()?;

    let mut interleaved: Vec<i16> = Vec::new();
    let mut chunk: Vec<i16> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(_) => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let n = decoded.samples_interleaved();
        if chunk.len() < n {
            chunk.resize(n, 0);
        }
        decoded.copy_to_slice_interleaved(&mut chunk[..n]);
        interleaved.extend_from_slice(&chunk[..n]);
    }
    Some((interleaved, channels, sample_rate))
}

/// Decode OGG to mono i16 PCM 44100Hz (single channel for multi-channel routing)
fn decode_to_mono_pcm(name: &str) -> Option<Vec<i16>> {
    use symphonia::core::io::MediaSourceStream;

    let data = get_embedded_audio(name)?;
    let cursor = std::io::Cursor::new(data);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let (interleaved, channels, _rate) = decode_stream_to_interleaved_i16(mss, "ogg")?;

    // Downmix to mono if the source is not already mono.
    let samples: Vec<i16> = if channels >= 2 {
        interleaved
            .chunks_exact(channels)
            .map(|c| ((c[0] as i32 + c[1] as i32) / 2) as i16)
            .collect()
    } else {
        interleaved
    };

    log::info!(
        "Decoded {} (mono): {} samples ({:.1}s)",
        name,
        samples.len(),
        samples.len() as f32 / SAMPLE_RATE as f32
    );
    if samples.is_empty() {
        None
    } else {
        Some(samples)
    }
}

/// Decode to stereo i16 PCM (for music on front speakers)
fn decode_to_stereo_pcm(name: &str) -> Option<Vec<i16>> {
    use symphonia::core::io::MediaSourceStream;

    let data = get_embedded_audio(name)?;
    let cursor = std::io::Cursor::new(data);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let (samples, _channels, _rate) = decode_stream_to_interleaved_i16(mss, "ogg")?;
    if samples.is_empty() {
        None
    } else {
        Some(samples)
    }
}

/// Decode an arbitrary file path (mp3/ogg) to stereo i16 PCM and resample
/// to the audio thread's 44100Hz target. Used for table preview audio
/// (`medias/audio.mp3`). Returns None on probe/decode failure.
fn decode_file_to_stereo_pcm_44100(path: &std::path::Path) -> Option<Vec<i16>> {
    use symphonia::core::io::MediaSourceStream;

    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let (interleaved, channels, src_rate) = decode_stream_to_interleaved_i16(mss, ext)?;
    if interleaved.is_empty() {
        return None;
    }
    if interleaved.is_empty() {
        return None;
    }

    // Downmix / upmix to stereo.
    let stereo: Vec<i16> = if channels == 1 {
        let mut out = Vec::with_capacity(interleaved.len() * 2);
        for s in &interleaved {
            out.push(*s);
            out.push(*s);
        }
        out
    } else if channels == 2 {
        interleaved
    } else {
        let frames = interleaved.len() / channels;
        let mut out = Vec::with_capacity(frames * 2);
        for f in 0..frames {
            let base = f * channels;
            out.push(interleaved[base]);
            out.push(interleaved[base + 1]);
        }
        out
    };

    // Linear-interpolation resample to 44100Hz if needed. Pitch-quality
    // is fine for short table jingles; avoids touching SDL stream format.
    if src_rate == 44100 {
        return Some(stereo);
    }
    let in_frames = stereo.len() / 2;
    let out_frames = (in_frames as u64 * 44100 / src_rate as u64) as usize;
    if out_frames == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(out_frames * 2);
    let ratio = src_rate as f64 / 44100.0;
    for i in 0..out_frames {
        let pos = i as f64 * ratio;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(in_frames - 1);
        let t = (pos - i0 as f64) as f32;
        let l0 = stereo[i0 * 2] as f32;
        let r0 = stereo[i0 * 2 + 1] as f32;
        let l1 = stereo[i1 * 2] as f32;
        let r1 = stereo[i1 * 2 + 1] as f32;
        out.push((l0 + (l1 - l0) * t) as i16);
        out.push((r0 + (r1 - r0) * t) as i16);
    }
    Some(out)
}

/// Route mono PCM to 8-channel (7.1) output on specific speakers
/// Returns 8-channel interleaved i16 data
pub(crate) fn mono_to_71(mono: &[i16], target: SpeakerTarget) -> Vec<i16> {
    // 7.1 layout: FL(0), FR(1), FC(2), LFE(3), BL(4), BR(5), SL(6), SR(7)
    // SSF pincab: BL/BR(4,5) = top playfield, SL/SR(6,7) = bottom/lockbar
    let mut out = vec![0i16; mono.len() * 8];
    for (i, &sample) in mono.iter().enumerate() {
        let base = i * 8;
        match target {
            SpeakerTarget::FrontBoth => {
                out[base] = sample;
                out[base + 1] = sample;
            }
            SpeakerTarget::TopLeft => {
                out[base + 4] = sample;
            }
            SpeakerTarget::TopRight => {
                out[base + 5] = sample;
            }
            SpeakerTarget::BottomLeft => {
                out[base + 6] = sample;
            }
            SpeakerTarget::BottomRight => {
                out[base + 7] = sample;
            }
            SpeakerTarget::TopBoth => {
                out[base + 4] = sample;
                out[base + 5] = sample;
            }
            SpeakerTarget::BottomBoth => {
                out[base + 6] = sample;
                out[base + 7] = sample;
            }
            SpeakerTarget::LeftBoth => {
                out[base + 4] = sample;
                out[base + 6] = sample;
            }
            SpeakerTarget::RightBoth => {
                out[base + 5] = sample;
                out[base + 7] = sample;
            }
        }
    }
    out
}

/// Route stereo PCM to 8-channel with L/R pan on front speakers (for music)
pub(crate) fn stereo_to_71_front(stereo: &[i16], pan: f32) -> Vec<i16> {
    let lg = (1.0 - pan.max(0.0)).min(1.0);
    let rg = (1.0 + pan.min(0.0)).min(1.0);
    let stereo_frames = stereo.len() / 2;
    let mut out = vec![0i16; stereo_frames * 8];
    for i in 0..stereo_frames {
        let base = i * 8;
        let l = stereo[i * 2];
        let r = stereo[i * 2 + 1];
        out[base] = (l as f32 * lg) as i16; // FL
        out[base + 1] = (r as f32 * rg) as i16; // FR
    }
    out
}

/// Spawn audio thread with 8-channel (7.1) output. Returns the command
/// sender plus the JoinHandle so the caller can shut the thread down:
/// drop the sender → recv Err → loop break → thread returns. No
/// per-thread SDL3 teardown — `App::shutdown_sdl_threads` calls
/// `SDL_Quit()` once both worker threads have joined, which nukes
/// every subsystem + open device in one shot.
pub fn spawn_audio_thread() -> (Sender<AudioCommand>, thread::JoinHandle<()>) {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();

    let handle = thread::spawn(move || {
        unsafe {
            if !SDL_InitSubSystem(SDL_INIT_AUDIO) {
                log::error!(
                    "Audio: SDL_InitSubSystem failed: {:?}",
                    CStr::from_ptr(SDL_GetError())
                );
                return;
            }

            // 8 channels (7.1) for SSF speaker routing, i16, 44100Hz
            let spec = SDL_AudioSpec {
                format: SDL_AUDIO_S16,
                channels: 8,
                freq: 44100,
            };

            let stream = SDL_OpenAudioDeviceStream(
                SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                &spec,
                None,
                std::ptr::null_mut(),
            );
            if stream.is_null() {
                log::error!(
                    "Audio: OpenAudioDeviceStream 7.1 failed: {:?}",
                    CStr::from_ptr(SDL_GetError())
                );
                return;
            }
            SDL_ResumeAudioStreamDevice(stream);
            log::info!("Audio thread: 7.1 stream opened and resumed");

            // Music-loop state: the stereo PCM is decoded once at StartMusic
            // then fed to the SDL audio stream in short chunks. The pan is
            // recomputed at every chunk (smoothed toward `target_pan` so a
            // fast slider drag doesn't click), and the cursor wraps around
            // so the music loops indefinitely until StopMusic.
            struct MusicState {
                stereo: Vec<i16>, // interleaved L,R,L,R,... at 44100Hz
                cursor: usize,    // sample index (in i16 units, not frames)
                pan: f32,         // applied at last push, smoothed
                target_pan: f32,  // last value received from the UI
            }
            let mut music: Option<MusicState> = None;

            // Chunk we push each idle tick when music is running. 100ms is
            // small enough that a slider drag feels live (pan changes are
            // audible within ~150ms) and large enough that the SDL queue
            // never underruns between two `recv_timeout` wake-ups.
            const MUSIC_CHUNK_MS: usize = 100;
            // Keep at least this many bytes queued so we never gap out
            // between two push chunks. `7.1 * i16 * 44100` = 705_600 B/s,
            // so 200ms ≈ 141_120 bytes of headroom.
            const MUSIC_QUEUE_LOW_BYTES: u32 = 141_120;
            // Exponential smoothing: `pan += (target - pan) * ALPHA` per
            // 100ms chunk. 0.15 crosses ±1 in ~600ms — snappy but no click.
            const PAN_ALPHA: f32 = 0.15;

            loop {
                let idle_wait = if music.is_some() {
                    // Short wake-up so we can top the queue and pick up
                    // slider changes with low latency.
                    std::time::Duration::from_millis(20)
                } else {
                    // Idle: 1s tick is plenty — real work arrives as
                    // commands anyway.
                    std::time::Duration::from_millis(1_000)
                };
                let recv_res = cmd_rx.recv_timeout(idle_wait);
                if let Err(crossbeam_channel::RecvTimeoutError::Timeout) = recv_res {
                    // No command this tick — top up the music buffer if
                    // playing and low on queued bytes.
                    if let Some(ms) = &mut music {
                        let queued = SDL_GetAudioStreamQueued(stream);
                        if queued >= 0 && (queued as u32) < MUSIC_QUEUE_LOW_BYTES {
                            let samples_per_ms = SAMPLE_RATE / 1000; // 44 f/ms
                            let frames = MUSIC_CHUNK_MS * samples_per_ms;
                            let mut chunk: Vec<i16> = Vec::with_capacity(frames * 2);
                            for _ in 0..frames {
                                chunk.push(ms.stereo[ms.cursor]);
                                chunk.push(ms.stereo[ms.cursor + 1]);
                                ms.cursor = (ms.cursor + 2) % ms.stereo.len();
                            }
                            ms.pan += (ms.target_pan - ms.pan) * PAN_ALPHA;
                            let data = stereo_to_71_front(&chunk, ms.pan);
                            SDL_PutAudioStreamData(
                                stream,
                                data.as_ptr() as *const _,
                                (data.len() * 2) as i32,
                            );
                            SDL_FlushAudioStream(stream);
                        }
                    }
                    continue;
                }
                match recv_res {
                    Ok(AudioCommand::PlayOnSpeaker { path, target }) => {
                        log::info!("Audio: PlayOnSpeaker {}", path);
                        if let Some(mono) = decode_to_mono_pcm(&path) {
                            let data = mono_to_71(&mono, target);
                            SDL_ClearAudioStream(stream);
                            SDL_PutAudioStreamData(
                                stream,
                                data.as_ptr() as *const _,
                                (data.len() * 2) as i32,
                            );
                            SDL_FlushAudioStream(stream);
                        }
                    }
                    Ok(AudioCommand::PlayBallSequence {
                        path,
                        from,
                        to,
                        hold_start_ms,
                        fade_ms,
                        hold_end_ms,
                    }) => {
                        log::info!(
                            "Audio: PlayBallSequence {} hold_start={}ms fade={}ms hold_end={}ms",
                            path,
                            hold_start_ms,
                            fade_ms,
                            hold_end_ms
                        );
                        if let Some(mono) = decode_to_mono_pcm(&path) {
                            SDL_ClearAudioStream(stream);

                            let samples_per_ms = SAMPLE_RATE / 1000;
                            let hold_start_samples = hold_start_ms as usize * samples_per_ms;
                            let fade_samples = fade_ms as usize * samples_per_ms;
                            let hold_end_samples = hold_end_ms as usize * samples_per_ms;

                            let mut offset = 0;

                            // Phase 1: hold on 'from'
                            let end1 = (offset + hold_start_samples).min(mono.len());
                            if offset < end1 {
                                let data = mono_to_71(&mono[offset..end1], from);
                                SDL_PutAudioStreamData(
                                    stream,
                                    data.as_ptr() as *const _,
                                    (data.len() * 2) as i32,
                                );
                                offset = end1;
                            }

                            // Phase 2: crossfade from -> to
                            let chunk_ms = 50u32;
                            let chunk_samples = chunk_ms as usize * samples_per_ms;
                            let fade_end = (offset + fade_samples).min(mono.len());
                            let fade_total = fade_end - offset;
                            let mut fade_pos = 0;
                            while offset < fade_end {
                                let end = (offset + chunk_samples).min(fade_end);
                                let chunk = &mono[offset..end];
                                let t = if fade_total > 0 {
                                    fade_pos as f32 / fade_total as f32
                                } else {
                                    1.0
                                };

                                let from_data = mono_to_71(chunk, from);
                                let to_data = mono_to_71(chunk, to);
                                let mixed: Vec<i16> = from_data
                                    .iter()
                                    .zip(to_data.iter())
                                    .map(|(&a, &b)| {
                                        ((a as f32 * (1.0 - t)) + (b as f32 * t)) as i16
                                    })
                                    .collect();
                                SDL_PutAudioStreamData(
                                    stream,
                                    mixed.as_ptr() as *const _,
                                    (mixed.len() * 2) as i32,
                                );

                                fade_pos += end - offset;
                                offset = end;
                            }

                            // Phase 3: hold on 'to'
                            let end3 = (offset + hold_end_samples).min(mono.len());
                            if offset < end3 {
                                let data = mono_to_71(&mono[offset..end3], to);
                                SDL_PutAudioStreamData(
                                    stream,
                                    data.as_ptr() as *const _,
                                    (data.len() * 2) as i32,
                                );
                            }

                            SDL_FlushAudioStream(stream);
                            let total_ms = hold_start_ms + fade_ms + hold_end_ms;
                            std::thread::sleep(std::time::Duration::from_millis(total_ms as u64));
                        }
                    }
                    Ok(AudioCommand::StartMusic { path }) => {
                        log::info!("Audio: StartMusic {}", path);
                        if let Some(stereo) = decode_to_stereo_pcm(&path) {
                            // Seed the loop with a first chunk so the SDL
                            // queue has audio to play before the next
                            // recv_timeout wake-up.
                            SDL_ClearAudioStream(stream);
                            let samples_per_ms = SAMPLE_RATE / 1000;
                            let seed_frames =
                                (MUSIC_CHUNK_MS * 2 * samples_per_ms).min(stereo.len() / 2);
                            let seed = &stereo[..seed_frames * 2];
                            let data = stereo_to_71_front(seed, 0.0);
                            SDL_PutAudioStreamData(
                                stream,
                                data.as_ptr() as *const _,
                                (data.len() * 2) as i32,
                            );
                            SDL_FlushAudioStream(stream);
                            music = Some(MusicState {
                                cursor: (seed_frames * 2) % stereo.len(),
                                pan: 0.0,
                                target_pan: 0.0,
                                stereo,
                            });
                        }
                    }
                    Ok(AudioCommand::SetMusicPan { pan }) => {
                        // Just note the new target; the music-loop tick
                        // above smooths toward it at the next chunk push.
                        // No SDL_Clear / re-push here — the currently
                        // queued audio keeps playing seamlessly.
                        if let Some(ms) = &mut music {
                            ms.target_pan = pan.clamp(-1.0, 1.0);
                        }
                    }
                    Ok(AudioCommand::StopMusic) | Ok(AudioCommand::StopAll) => {
                        SDL_ClearAudioStream(stream);
                        music = None;
                    }
                    Ok(AudioCommand::PreviewStart { path, volume }) => {
                        log::info!("Audio: PreviewStart {} vol={:.2}", path.display(), volume);
                        SDL_ClearAudioStream(stream);
                        music = None;
                        if let Some(stereo) = decode_file_to_stereo_pcm_44100(&path) {
                            let v = volume.clamp(0.0, 1.0);
                            let scaled: Vec<i16> = if (v - 1.0).abs() > 0.01 {
                                stereo.iter().map(|s| (*s as f32 * v) as i16).collect()
                            } else {
                                stereo
                            };
                            let data = stereo_to_71_front(&scaled, 0.0);
                            SDL_PutAudioStreamData(
                                stream,
                                data.as_ptr() as *const _,
                                (data.len() * 2) as i32,
                            );
                            SDL_FlushAudioStream(stream);
                        } else {
                            log::warn!("Audio: PreviewStart decode failed for {}", path.display());
                        }
                    }
                    Ok(AudioCommand::PreviewStop) => {
                        SDL_ClearAudioStream(stream);
                    }
                    Ok(AudioCommand::Quit) | Err(_) => break,
                }
            }
            // Just exit. No SDL_DestroyAudioStream / SDL_QuitSubSystem
            // here: `App::shutdown_sdl_threads` calls `SDL_Quit()` on
            // the main thread once both worker threads have joined,
            // which nukes every subsystem + open device in one shot.
            let _ = stream; // kept alive for the loop, dropped on exit
            log::info!("Audio thread: exited command loop");
        }
    });

    (cmd_tx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VpxConfig;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn config_from_str(content: &str) -> VpxConfig {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(content.as_bytes()).unwrap();
        VpxConfig::load(Some(tmp.path())).unwrap()
    }

    // --- Sound3DMode ---

    #[test]
    fn sound_3d_mode_from_i32_valid() {
        assert_eq!(Sound3DMode::from_i32(0), Sound3DMode::FrontStereo);
        assert_eq!(Sound3DMode::from_i32(1), Sound3DMode::RearStereo);
        assert_eq!(Sound3DMode::from_i32(2), Sound3DMode::SurroundRearLockbar);
        assert_eq!(Sound3DMode::from_i32(3), Sound3DMode::SurroundFrontLockbar);
        assert_eq!(Sound3DMode::from_i32(4), Sound3DMode::SsfLegacy);
        assert_eq!(Sound3DMode::from_i32(5), Sound3DMode::SsfNew);
    }

    #[test]
    fn sound_3d_mode_from_i32_invalid_defaults() {
        assert_eq!(Sound3DMode::from_i32(-1), Sound3DMode::FrontStereo);
        assert_eq!(Sound3DMode::from_i32(6), Sound3DMode::FrontStereo);
        assert_eq!(Sound3DMode::from_i32(999), Sound3DMode::FrontStereo);
    }

    #[test]
    fn sound_3d_mode_all_has_6_entries() {
        assert_eq!(Sound3DMode::all().len(), 6);
    }

    #[test]
    fn sound_3d_mode_roundtrip_i32() {
        for mode in Sound3DMode::all() {
            assert_eq!(Sound3DMode::from_i32(*mode as i32), *mode);
        }
    }

    #[test]
    fn sound_3d_mode_labels_not_empty() {
        for mode in Sound3DMode::all() {
            assert!(!mode.label().is_empty());
        }
    }

    // --- AudioConfig default ---

    #[test]
    fn audio_config_default() {
        let cfg = AudioConfig::default();
        assert!(cfg.available_devices.is_empty());
        assert!(cfg.device_bg.is_empty());
        assert!(cfg.device_pf.is_empty());
        assert_eq!(cfg.sound_3d_mode, Sound3DMode::FrontStereo);
        assert_eq!(cfg.music_volume, 100);
        assert_eq!(cfg.sound_volume, 100);
        assert!(!cfg.music_looping);
        assert!((cfg.music_pan - 0.0).abs() < f32::EPSILON);
    }

    // --- AudioConfig load/save ---

    #[test]
    fn audio_config_load_from_config() {
        let cfg = config_from_str(
            "[Player]\nSoundDeviceBG = HD Audio\nSoundDevice = USB\n\
             Sound3D = 4\nMusicVolume = 75\nSoundVolume = 50\n",
        );
        let mut audio = AudioConfig::default();
        audio.load_from_config(&cfg);
        assert_eq!(audio.device_bg, "HD Audio");
        assert_eq!(audio.device_pf, "USB");
        assert_eq!(audio.sound_3d_mode, Sound3DMode::SsfLegacy);
        assert_eq!(audio.music_volume, 75);
        assert_eq!(audio.sound_volume, 50);
    }

    #[test]
    fn audio_config_load_empty_keeps_defaults() {
        let cfg = config_from_str("");
        let mut audio = AudioConfig::default();
        audio.load_from_config(&cfg);
        assert_eq!(audio.music_volume, 100);
        assert_eq!(audio.sound_3d_mode, Sound3DMode::FrontStereo);
    }

    #[test]
    fn audio_config_save_to_config() {
        let mut cfg = config_from_str("");
        let audio = AudioConfig {
            device_bg: "Speaker A".to_string(),
            device_pf: "Speaker B".to_string(),
            sound_3d_mode: Sound3DMode::SsfNew,
            music_volume: 80,
            sound_volume: 60,
            ..Default::default()
        };
        audio.save_to_config(&mut cfg);
        assert_eq!(
            cfg.get("Player", "SoundDeviceBG"),
            Some("Speaker A".to_string())
        );
        assert_eq!(
            cfg.get("Player", "SoundDevice"),
            Some("Speaker B".to_string())
        );
        assert_eq!(cfg.get_i32("Player", "Sound3D"), Some(5));
        assert_eq!(cfg.get_i32("Player", "MusicVolume"), Some(80));
        assert_eq!(cfg.get_i32("Player", "SoundVolume"), Some(60));
    }

    // --- mono_to_71 ---

    #[test]
    fn mono_to_71_front_both() {
        let mono = vec![1000i16, 2000];
        let out = mono_to_71(&mono, SpeakerTarget::FrontBoth);
        assert_eq!(out.len(), 16); // 2 samples × 8 channels
                                   // Frame 0: FL=1000, FR=1000, rest=0
        assert_eq!(out[0], 1000);
        assert_eq!(out[1], 1000);
        assert_eq!(out[2], 0);
        // Frame 1: FL=2000, FR=2000
        assert_eq!(out[8], 2000);
        assert_eq!(out[9], 2000);
    }

    #[test]
    fn mono_to_71_top_left() {
        let mono = vec![500i16];
        let out = mono_to_71(&mono, SpeakerTarget::TopLeft);
        // BL is channel 4
        assert_eq!(out[4], 500);
        assert_eq!(out[0], 0); // FL silent
        assert_eq!(out[5], 0); // BR silent
    }

    #[test]
    fn mono_to_71_bottom_both() {
        let mono = vec![300i16];
        let out = mono_to_71(&mono, SpeakerTarget::BottomBoth);
        // SL(6) and SR(7)
        assert_eq!(out[6], 300);
        assert_eq!(out[7], 300);
        assert_eq!(out[4], 0); // BL silent
    }

    #[test]
    fn mono_to_71_left_both() {
        let mono = vec![400i16];
        let out = mono_to_71(&mono, SpeakerTarget::LeftBoth);
        // BL(4) and SL(6)
        assert_eq!(out[4], 400);
        assert_eq!(out[6], 400);
        assert_eq!(out[5], 0); // BR silent
        assert_eq!(out[7], 0); // SR silent
    }

    #[test]
    fn mono_to_71_empty_input() {
        let out = mono_to_71(&[], SpeakerTarget::FrontBoth);
        assert!(out.is_empty());
    }

    // --- stereo_to_71_front ---

    #[test]
    fn stereo_to_71_center_pan() {
        let stereo = vec![1000i16, 2000]; // L=1000, R=2000
        let out = stereo_to_71_front(&stereo, 0.0);
        assert_eq!(out.len(), 8); // 1 frame × 8 channels
        assert_eq!(out[0], 1000); // FL
        assert_eq!(out[1], 2000); // FR
        assert_eq!(out[2], 0); // FC
    }

    #[test]
    fn stereo_to_71_full_left_pan() {
        let stereo = vec![1000i16, 1000];
        let out = stereo_to_71_front(&stereo, -1.0);
        assert_eq!(out[0], 1000); // FL at full
        assert_eq!(out[1], 0); // FR muted
    }

    #[test]
    fn stereo_to_71_full_right_pan() {
        let stereo = vec![1000i16, 1000];
        let out = stereo_to_71_front(&stereo, 1.0);
        assert_eq!(out[0], 0); // FL muted
        assert_eq!(out[1], 1000); // FR at full
    }

    #[test]
    fn stereo_to_71_empty() {
        let out = stereo_to_71_front(&[], 0.0);
        assert!(out.is_empty());
    }

    // --- Embedded audio ---

    #[test]
    fn embedded_audio_knocker_exists() {
        assert!(get_embedded_audio("knocker.ogg").is_some());
    }

    #[test]
    fn embedded_audio_ball_roll_exists() {
        assert!(get_embedded_audio("ball_roll.ogg").is_some());
    }

    #[test]
    fn embedded_audio_music_exists() {
        assert!(get_embedded_audio("music.ogg").is_some());
    }

    #[test]
    fn embedded_audio_unknown_returns_none() {
        assert!(get_embedded_audio("nonexistent.ogg").is_none());
    }

    // --- Audio decoding ---

    #[test]
    fn decode_knocker_to_mono() {
        let pcm = decode_to_mono_pcm("knocker.ogg");
        assert!(pcm.is_some(), "knocker.ogg should decode to mono PCM");
        let samples = pcm.unwrap();
        assert!(!samples.is_empty());
    }

    #[test]
    fn decode_music_to_stereo() {
        let pcm = decode_to_stereo_pcm("music.ogg");
        assert!(pcm.is_some(), "music.ogg should decode to stereo PCM");
        let samples = pcm.unwrap();
        assert!(!samples.is_empty());
        // Stereo = even number of samples
        assert_eq!(samples.len() % 2, 0);
    }

    #[test]
    fn decode_nonexistent_returns_none() {
        assert!(decode_to_mono_pcm("does_not_exist.ogg").is_none());
    }
}
