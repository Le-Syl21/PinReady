# PinReady — CLAUDE.md

## Project overview

PinReady is a cross-platform configurator and launcher for VPinballX (Visual Pinball X standalone 10.8.1).
It handles screen assignment, input mapping, tilt/nudge sensitivity, audio routing, table browsing and asset management.
It replaces the non-existent native configuration tools for VPX standalone (SDL3/bgfx builds).

**Target: VPinballX 10.8.1** — uses the new folder-per-table layout.
**Cross-platform** — Linux, macOS, Windows. No platform-specific APIs (no Win32, no xrandr). SDL3 only.

**First run** (detected by uninitialized SQLite DB) → configuration wizard.
**Subsequent runs** → table selector / launcher. Wizard can be re-launched; all fields pre-filled from current ini values.

---

## Stack

| Layer | Crate | Role |
|---|---|---|
| UI | `eframe 0.33` + `egui 0.33` | Immediate mode GUI |
| Images | `egui_extras 0.33` (feature: `image`) | Thumbnail display |
| Display/Input | `sdl3-sys 0.6` (feature: `build-from-source-static`) | Screen enumeration + input capture |
| Config | `serde 1` + `ini 1.3` | Read/write VPinballX.ini (Windows-style .ini format) |
| Database | `rusqlite 0.38` (feature: `bundled`) | Local table catalog |
| File scan | `walkdir 2.5` | Recursive .vpx discovery |
| Images | `image 0.25` (features: `png`, `jpeg`) | Media pack thumbnails |
| Audio decode | `symphonia 0.5` (features: `ogg`, `vorbis`, `mp3`, `pcm`, `wav`) | Decode OGG/MP3 → PCM for SDL3 |
| Threading | `crossbeam-channel 0.5` | SDL3 thread ↔ egui communication |
| Logging | `log 0.4` + `env_logger 0.11` | Debug output |

**No system dependencies required** — SDL3 and SQLite are statically linked via bundled features.

---

## Architecture

```
src/
  main.rs         # Entry point, first-run detection, eframe launch
  app.rs          # Main App struct implementing eframe::App, page routing
  screens.rs      # SDL3 display enumeration + heuristic auto-assignment
  inputs.rs       # Input mapping: SDL3 event loop on dedicated thread
  tilt.rs         # Tilt/nudge sensitivity configuration
  audio.rs        # Audio device detection + routing configuration
  launcher.rs     # Table selector: scan, list, launch VPX subprocess
  assets.rs       # Asset detection per table
  config.rs       # VPinballX.ini read/write (ini crate)
  db.rs           # SQLite catalog (table path, name, year, media paths)
```

---

## Pages / UI flow (configuration wizard)

### Page 1 — Screen assignment

**Step 1: Screen count selection**
- Radio buttons: 1 screen / 2 screens / 3 screens / 4 screens
- More screens = more likely a pincab setup

**Step 2: Screen identification and role assignment**
- Enumerate displays via `SDL_GetDisplays` + `SDL_GetDisplayName` (sdl3-sys)
- `SDL_GetDisplayName` returns EDID monitor name **with size in inches** (e.g. "Samsung U28E590 43\"")
  - SDL3 reads EDID data and computes diagonal from physical mm: `√(w_mm² + h_mm²) / 25.4`
  - This works cross-platform (X11 via XRandR EDID, Wayland via wl_output, Windows via HMONITOR)
- Display connector port names: rename for readability (DP → DisplayPort, DVI-D → DVI, HDMI → HDMI)
- For each display show: **name + size in inches**, **port**, **resolution**, **refresh rate**
- **Heuristic auto-assignment (by total pixel count, descending):**
  - Largest display → Playfield
  - 2nd largest → Backglass
  - 3rd largest (smallest) → DMD
  - 4th display (if any) → Topper
- Each display gets a dropdown to override its role
- Number of available roles matches the screen count selected in step 1

**INI output — screen placement:**
VPX uses absolute desktop coordinates for window placement. Screens are placed left-to-right:
- Playfield: X=0, Y=0, W×H = native resolution
- Backglass: X=playfield_width, Y=0, W×H = native resolution
- DMD: X=playfield_width + backglass_width, Y=0, W×H = native resolution

```ini
[Player]
PlayfieldDisplay = <display_name>
PlayfieldFullScreen = 1  ; Required for correct multi-screen positioning
PlayfieldWndX = 0
PlayfieldWndY = 0
PlayfieldWidth = 3840
PlayfieldHeight = 2160
BGSet = 2  ; View mode: 0=Desktop/FSS, 1=Cabinet, 2=FSS — auto-set to 1 (Cabinet) when 2+ screens, but always editable by user

[Backglass]
BackglassOutput = 1  ; Floating
BackglassDisplay = <display_name>
BackglassWndX = 0
BackglassWndY = 0
BackglassWidth = 2560
BackglassHeight = 1440

[ScoreView]
ScoreViewOutput = 1  ; Floating
ScoreViewDisplay = <display_name>
ScoreViewWndX = 0
ScoreViewWndY = 0
ScoreViewWidth = 1920
ScoreViewHeight = 1080
```

### Page 2 — Input mapping

**Input binding format** (stored in `[Input]` section):
- Devices are declared with `Device.<deviceId>.Type` and `Device.<deviceId>.Name`
  - Device IDs: `Key` (keyboard), `Mouse`, `SDLJoy_<unique_id>` (joystick/gamepad)
  - `NoAutoLayout = 1` disables auto-layout for that device
- Digital: `Mapping.<ActionId> = <deviceId>;<buttonId>` (e.g. `Key;42` or `SDLJoy_PSC0041701862884E45J009;7`)
  - Multiple alternatives separated by `|`, combos by `&`
- Analog: `Mapping.<SensorId> = <deviceId>;<axisId>;<type>;<deadZone>;<scale>;<limit>`
  - Type: P=Position, V=Velocity, A=Acceleration

**Real example** (Pinscape controller + keyboard):
```ini
[Input]
Devices = 
Device.Key.Type = 
Device.Key.NoAutoLayout = 
Device.Mouse.Type = 
Device.Mouse.NoAutoLayout = 
Device.SDLJoy_PSC0041701862884E45J009.Type = 
Device.SDLJoy_PSC0041701862884E45J009.NoAutoLayout = 1
Mapping.LeftFlipper = SDLJoy_PSC0041701862884E45J009;7
Mapping.RightFlipper = SDLJoy_PSC0041701862884E45J009;8
Mapping.LaunchBall = SDLJoy_PSC0041701862884E45J009;4
Mapping.Start = SDLJoy_PSC0041701862884E45J009;0
Mapping.Credit1 = Key;34
Mapping.LeftNudge = Key;29
Mapping.RightNudge = Key;56
```

**SDL3 event loop** runs on a **dedicated thread**, communicates via `crossbeam-channel`.

**At startup**, detect connected devices via SDL3 joystick/gamepad API.
- If dedicated pinball controller detected (Pinscape KL25Z, Pinscape Pico, DudesCab) → VPX auto-manages plunger axes and accelerometer nudge. PinReady does NOT map PlungerPos/PlungerVel/NudgeX/NudgeY — those are handled by VPX directly.
- Detection: name contains "Pinscape" or ID contains "PSC" → Pinscape; name contains "DudesCab" → DudesCab.
- Three controller profiles with default button mappings, selectable in the wizard:

**Profile 0 — KL25Z (KL Shield V5.1 / Brain / Rig Master)**
Verified via jstest on physical hardware (Arnoz default firmware config).

| SDL Btn | KL Shield label | VPX Action |
|---|---|---|
| 0 | START | Start |
| 1 | EXTRA-B | ExtraBall |
| 2 | COIN1 | Credit1 |
| 3 | COIN2 | Credit2 |
| 4 | L BALL | LaunchBall |
| 5 | EXIT | ExitGame |
| 6 | QUIT | *(VP editor, not mapped)* |
| 7 | L FLIPP | LeftFlipper + LeftStagedFlipper |
| 8 | R FLIPP | RightFlipper + RightStagedFlipper |
| 9 | L MAGNA | LeftMagna |
| 10 | R MAGNA | RightMagna |
| 11 | FIRE | Lockbar |
| 12 | TILT | Tilt |
| 13 | DOOR | CoinDoor |
| 14 | SERVICE EXIT | Service1 (Cancel) |
| 15 | SERVICE - | Service2 (Down) |
| 16 | SERVICE + | Service3 (Up) |
| 17 | ENTER | Service4 (Enter) |
| 18 | N.M. | *(Night Mode, Pinscape only)* |
| 19 | VOL- | VolumeDown |
| 20 | VOL+ | VolumeUp |

**Profile 1 — Pinscape Pico (OpenPinballDevice)**
From `OpenPinballDeviceReport.h` standard.

| SDL Btn | Function | VPX Action |
|---|---|---|
| 0 | Start | Start |
| 1 | Exit | ExitGame |
| 2 | Extra Ball | ExtraBall |
| 3–6 | Coin 1–4 | Credit1–Credit4 |
| 7 | Launch Ball | LaunchBall |
| 8 | Fire | Lockbar |
| 9 | Left Flipper | LeftFlipper |
| 10 | Right Flipper | RightFlipper |
| 11 | Upper Left Flipper | LeftStagedFlipper |
| 12 | Upper Right Flipper | RightStagedFlipper |
| 13 | MagnaSave Left | LeftMagna |
| 14 | MagnaSave Right | RightMagna |
| 15 | Tilt Bob | Tilt |
| 16 | Slam Tilt | SlamTilt |
| 17 | Coin Door | CoinDoor |
| 18–21 | Service Cancel/Down/Up/Enter | Service1–Service4 |
| 22 | Left Nudge | LeftNudge |
| 23 | Forward Nudge | CenterNudge |
| 24 | Right Nudge | RightNudge |
| 25 | Volume Up | VolumeUp |
| 26 | Volume Down | VolumeDown |

**Profile 2 — DudesCab (Arnoz)**
From official DudesCab mapping table. Buttons numbered from 1 in docs, SDL from 0.

| SDL Btn | DudesCab label | VPX Action |
|---|---|---|
| 0 | Start | Start |
| 1 | ExtraBall | ExtraBall |
| 2 | Coin1 | Credit1 |
| 3 | Coin2 | Credit2 |
| 4 | LaunchBall | LaunchBall |
| 5 | Return | ExitGame |
| 6 | Exit | *(Quit to editor, not mapped)* |
| 7 | Flipper Left | LeftFlipper + LeftStagedFlipper |
| 8 | Flipper Right | RightFlipper + RightStagedFlipper |
| 9 | Magna Left | LeftMagna |
| 10 | Magna Right | RightMagna |
| 11 | Tilt | Tilt |
| 12 | Fire | Lockbar |
| 13 | Door | CoinDoor |
| 14 | ROM Exit | Service1 |
| 15 | ROM - | Service2 |
| 16 | ROM + | Service3 |
| 17 | ROM Enter | Service4 |
| 18 | VOL - | VolumeDown |
| 19 | VOL + | VolumeUp |
| 20–23 | DPAD | *(Hat, not buttons)* |
| 24 | NightMode | *(DO NOT REMAP)* |
| 25–30 | Spare 1–6 | *(User-defined)* |
| 31 | Calib | *(DO NOT REMAP)* |

- Keyboard and joypad can coexist.

**Actions to map — presented all at once:**

Essential (always shown):
| Action | Setting ID | Type | Default Key |
|---|---|---|---|
| Left Flipper | LeftFlipper | Digital | Left Shift |
| Right Flipper | RightFlipper | Digital | Right Shift |
| Left Magna | LeftMagna | Digital | Left Ctrl |
| Right Magna | RightMagna | Digital | Right Ctrl |
| Lockbar | Lockbar | Digital | Left Alt |
| Extra Ball | ExtraBall | Digital | B |
| Launch Ball (digital plunger) | LaunchBall | Digital | Enter |
| Start Game | Start | Digital | 1 |
| Add Credit | Credit1 | Digital | 5 |
| Exit Game | ExitGame | Digital | Escape |

Advanced (optional, collapsed by default):
| Action | Setting ID | Type | Default Key |
|---|---|---|---|
| Add Credit 2-4 | Credit2-4 | Digital | 4, 3, 6 |
| Coin Door | CoinDoor | Digital | End |
| Service 1-2 | Service1-2 | Digital | 7, 8 |
| Custom 1-4 | Custom1-4 | Digital | unmapped |
| Left/Right Staged Flipper | L/RStagedFlipper | Digital | unmapped |

Hidden/auto-managed (not shown in wizard):
- Nudge L/R/Center, Tilt → auto if accelerometer detected, else mappable
- VR actions → not relevant for cab
- Debug/Perf actions → not relevant for wizard
- Service 3-8, SlamTilt → rarely used

**Capture flow**: for each action, display current binding (default or previously configured) → "Press a key / button..." → capture SDL3 event → show human-readable name. Escape = skip, keeps current/default value.

**Default key bindings (hardcoded in VPX):**

Essential actions:
| Setting ID | Label | Default Key | SDL Scancode |
|---|---|---|---|
| LeftFlipper | Left Flipper | Left Shift | LSHIFT |
| RightFlipper | Right Flipper | Right Shift | RSHIFT |
| LeftMagna | Left Magna | Left Ctrl | LCTRL |
| RightMagna | Right Magna | Right Ctrl | RCTRL |
| Lockbar | Lockbar | Left Alt | LALT |
| ExtraBall | Extra Ball | B | B |
| LaunchBall | Launch Ball | Enter | RETURN |
| Start | Start Game | 1 | 1 |
| Credit1 | Add Credit (1) | 5 | 5 |
| ExitGame | Exit Game | Escape | ESCAPE |

Advanced actions:
| Setting ID | Label | Default Key | SDL Scancode |
|---|---|---|---|
| Credit2 | Add Credit (2) | 4 | 4 |
| Credit3 | Add Credit (3) | 3 | 3 |
| Credit4 | Add Credit (4) | 6 | 6 |
| CoinDoor | Coin Door | End | END |
| Service1 | Service #1 | 7 | 7 |
| Service2 | Service #2 | 8 | 8 |
| Custom1-4 | Custom Buttons | (unmapped) | — |
| LeftStagedFlipper | Left Staged Flipper | Left Shift | LSHIFT |
| RightStagedFlipper | Right Staged Flipper | Right Shift | RSHIFT |

Hidden/auto (not in wizard):
| Setting ID | Label | Default Key |
|---|---|---|
| LeftNudge | Left Nudge | Z |
| RightNudge | Right Nudge | / |
| CenterNudge | Center Nudge | Space |
| Tilt | Tilt | T |
| SlamTilt | Slam Tilt | Home |
| Reset | Reset | F3 |
| Pause | Pause | P |
| InGameUI | Toggle InGame UI | F12 |
| PerfOverlay | Perf Overlay | F11 |
| VolumeDown | Volume Down | - |
| VolumeUp | Volume Up | = |
| DebugBalls | Debug Balls | O |
| Debugger | Debugger | D |
| ToggleStereo | Stereo Mode | F10 |
| ShowRules | Show Rules | (unmapped) |
| Service3 | Service #3 | 9 |
| Service4 | Service #4 | 0 |
| Service5 | Service #5 | 6 |
| Service6 | Service #6 | Page Up |
| Service7 | Service #7 | - |
| Service8 | Service #8 | (unmapped) |
| GenTournament | Tournament File | Left Alt + 1 |
| VRCenter/Up/Down/Front/Back/Left/Right | VR navigation | Numpad or unmapped |

**⚠️ Known default key conflicts:**
| Key | Actions sharing it | Note |
|---|---|---|
| Left Shift | LeftFlipper + LeftStagedFlipper | Intentional (staged is modifier) |
| Right Shift | RightFlipper + RightStagedFlipper | Intentional (staged is modifier) |
| - (Minus) | VolumeDown + Service7 | Conflict |
| 6 | Credit4 + Service5 | Conflict |

PinReady must **warn the user** if they assign a key already used by another action (display which action conflicts).

### Page 3 — Tilt / Nudge sensitivity

VPX auto-detects accelerometer devices (KL25Z, Pinscape Pico, etc.).
PinReady configures **sensitivity tuning only**:

**Simplified mode** (default): single slider "Sensitivity" that adjusts multiple parameters together.
**Advanced mode** (expandable): individual control over each parameter.

| INI Key (section [Player]) | Role | Default | Range |
|---|---|---|---|
| NudgeStrength | Visual nudge intensity | 0.02 | 0.0–0.25 |
| PlumbInertia | Tilt plumb simulation inertia | 0.35 | 0.001–1.0 |
| PlumbThresholdAngle | Angle that triggers Tilt | 35.0 | 5.0–60.0 |
| NudgeFilter0 / NudgeFilter1 | Anti-noise filter on sensors | 0 | 0/1 |
| NudgeOrientation0 / NudgeOrientation1 | Sensor orientation | 0.0 | 0.0–360.0 |

### Page 4 — Audio configuration

**Dual audio device model** in VPX:
- `SoundDeviceBG` → Backglass speakers (music, voice, game sounds)
- `SoundDevice` → Playfield speakers (mechanical sounds: flippers, bumpers, drops)

**Step 1: Detect audio devices** via `SDL_GetAudioOutputDevices` (SDL3)

**Step 2: Assign devices** — dropdown for each role (Backglass / Playfield)

**Step 3: Output mode** (`Sound3D` key in `[Player]`):
| Mode | Description | Typical Use |
|---|---|---|
| 0 | 2 Front channels | Simple stereo, speakers in front |
| 1 | 2 Rear channels | Stereo, speakers behind (lockbar) |
| 2 | 6 channels, Rear at lockbar | 5.1 surround, rear = lockbar |
| 3 | 6 channels, Front at lockbar | 5.1 surround, front = lockbar |
| 4 | 6ch Side & Rear at lockbar, Legacy | SSF legacy mixing |
| 5 | 6ch Side & Rear at lockbar, New | SSF new mixing |

**Step 4: Volumes** — sliders for `MusicVolume` (backglass) and `SoundVolume` (playfield), range 0–100.

Visual diagrams showing speaker placement in cab for each mode (inspired by SSF setup guides).

**Step 5: Audio test sequence**

Sound assets in `assets/audio/`:
| File | Role | Size |
|---|---|---|
| `music.ogg` | Backglass music test | 576K |
| `ball_release.ogg` | Ball release sound (medium freq) | 23K |
| `ball_roll.ogg` | Ball rolling sound (continuous) | 55K |
| `ball_drop.ogg` | Ball drop impact (medium/low freq) | 7K |
| `knocker.ogg` | Knocker surprise on config validation | 7K |

Audio files are OGG Vorbis, decoded at runtime via `symphonia` crate to PCM i16 stereo 44100Hz for SDL3 playback.

**Test 1 — Backglass music** (validates `SoundDeviceBG` + L/R wiring):
- Play/Stop button — music loops on backglass device
- User can leave music playing in background while testing playfield
- Fade left→right available to validate backglass stereo wiring

**Test 2 — Playfield top→bottom** (validates SSF mode 4/5 — front-to-lockbar):
- `ball_release.wav` at 100% on both **top** (front) speakers
- `ball_roll.wav` fades from both top speakers → both bottom (lockbar/rear) speakers
- `ball_drop.wav` at 100% on both **bottom** speakers
- No left/right panning — only front/rear axis
- If sound direction is wrong → suggest switching Sound3D mode

**Test 3 — Playfield left→right** (validates L/R wiring):
- `ball_release.wav` at 100% on **left** speakers only
- `ball_roll.wav` fades from left → right (no top/bottom panning, same height)
- `ball_drop.wav` at 100% on **right** speakers only
- If direction is wrong → suggest checking cable wiring

**Validation surprise**: when user confirms all tests passed → play `knocker.wav` at full volume!

Each test has pass/fail feedback. On failure, suggest corrective action (swap Sound3D mode, check wiring, swap device assignment).

### Page 5 — Tables directory

Simple file picker: select the root directory containing all table folders (folder-per-table layout).
Stored in PinReady's own SQLite config (not in VPinballX.ini).
Default suggestion: scan common locations (`~/Documents`, `~/tables`, `~/Okay`, etc.)

**Note to user:** All settings configured in the wizard can also be modified in-game via the **F12** key (InGame UI).

### Page 6 — Table selector (main launcher)

*Phase 2 — not part of initial configuration wizard.*

- Scan configurable root directory for folder-per-table `.vpx` files
- Extract backglass thumbnails from `.directb2s` files (via `directb2s` crate, no external tool needed)
- Display: scrollable grid with backglass thumbnail or placeholder
- On table select → launch: `VPinballX_BGFX -play <table.vpx>`

---

## VPinballX 10.8.1 — Folder-per-table layout

Reference: `/home/pincab/VPinballX/docs/FileLayout.md`

Each table lives in its own directory with all companion files:
```
Table Name (Manufacturer Year)/
├── table.vpx                        # Main table file
├── table.ini                        # Per-table settings override
├── table.directb2s                  # Backglass
├── table.info                       # Frontend metadata (JSON)
├── altsound/<rom_name>/             # AltSound plugin files
├── cache/                           # VPX runtime cache
├── medias/                          # Frontend media (thumbnails, videos)
│   ├── (Playfield) Table Name.mp4
│   ├── (Backglass) Table Name.mp4
│   ├── (Wheel) Table Name.apng
│   └── ...
├── music/                           # Music (PlayMusic script command)
├── pinmame/                         # PinMAME plugin
│   ├── roms/<rom_name>.zip
│   ├── nvram/<rom_name>.nv
│   └── alias.txt
├── pupvideos/<rom_name>/            # PinUp Player videos
├── scripts/                         # Additional table scripts
├── serum/<rom_name>/<rom_name>.crz  # Serum colorization
├── user/VPReg.stg                   # Script-saved values
└── vni/<rom_name>/                  # VNI colorization
    ├── <rom_name>.pal
    └── <rom_name>.vni
```

File search order: table filename → folder name → legacy global folder.

**Legacy layout** (Batocera-style with `roms/`, `nvram/`, `altcolor/` at table root) is also supported by VPX but discouraged.

---

## VPinballX.ini format

**Global ini location:** `~/.local/share/VPinballX/10.8/VPinballX.ini`

VPX supports a global ini and a **per-table ini** (same name as `.vpx`, same directory).
Per-table ini overrides global.

Do not hardcode section names or key names — the ini format evolves.
**Read the actual ini file at startup** to discover available keys and sections dynamically.
Only write back keys that already exist in the file, or that are explicitly documented by VPX.

---

## SDL3 threading model

`eframe` uses `winit` internally for its event loop — SDL3 must run on a **separate thread**.

```
Main thread: eframe/egui event loop
SDL3 thread: SDL_Init → event poll loop → send events via crossbeam-channel → recv in egui update()
```

Do NOT call SDL3 functions from the egui update() thread.
Do NOT call egui functions from the SDL3 thread.
Use `crossbeam_channel::unbounded()` for communication.

---

## Key conventions

- **Cross-platform only** — no Win32, no xrandr, no platform-specific APIs. SDL3 for everything.
- **No system dependencies** — use bundled features for SDL3 and SQLite
- **Subprocess for VPX only** — use `std::process::Command` to launch tables, no FFI linking, no external tools required
- **Unsafe SDL3 calls** — wrap in safe Rust functions in `screens.rs` and `inputs.rs`, never expose raw pointers to other modules
- **Error handling** — use `anyhow` or explicit `Result` types, no `unwrap()` in production paths
- **Config writes are atomic** — read full ini → modify → write back, never partial writes
- **SQLite catalog** — initialize schema on first run, upsert on rescan
- **First run detection** — uninitialized SQLite database triggers wizard mode

---

## Build

```bash
# Ubuntu/Debian dependencies (for eframe/winit)
sudo apt install build-essential cmake pkg-config \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev

# SDL3 is compiled from source automatically via build-from-source-static feature

cargo build --release
```

---

## External tools

- **VPinballX** — executable at `~/Visual_Pinball/VPinballX_BGFX` (auto-installed or user-configured). Used for: `VPinballX_BGFX -play <table.vpx>`
- **vpxtool** — **NOT used**. Was previously considered for `vpxtool info <table.vpx>` to extract table metadata, but dropped: PinReady now scans table folders directly and extracts backglass thumbnails from `.directb2s` files using the `directb2s` crate. This keeps the project dependency-free (no external tools required in PATH).

---

## Reference sources

- **VPX source code**: `~/vpinball/` — authoritative for input system, ini format, SDL3 usage
  - Input mapping: `src/input/InputAction.cpp`, `InputManager.cpp`, `PhysicsSensor.cpp`
  - Display config: `src/renderer/Window.cpp`, `Window.h`
  - Settings properties: `src/core/Settings_properties.inl`
- **VPX file layout**: `/home/pincab/VPinballX/docs/FileLayout.md`
- **SDL3 source**: `~/SDL/` — for API details not in headers
- **VPX ini**: `~/.local/share/VPinballX/10.8/VPinballX.ini`
- **Note**: VPX docs/ folder contains outdated documentation (pre-standalone era). Always prefer source code.

---

## Running

Claude Code is operated **locally**. You can build and run the application (`cargo build`, `cargo run`).
Do NOT launch VPinballX directly.

---

## Out of scope (phase 1)

- Video playback in table selector (future: `video-rs` or ffmpeg subprocess)
- 3D table preview (future: `vpin` crate + wgpu)
- WASM/VBScript transpilation (separate project)
- Multi-instance VPX management
