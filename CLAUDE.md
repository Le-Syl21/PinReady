# PinReady — CLAUDE.md

## Project overview

PinReady is a cross-platform configurator and launcher for VPinballX (Visual Pinball X standalone 10.8.1).
It handles screen assignment, input mapping, tilt/nudge sensitivity, audio routing, table browsing and asset management.
It replaces the non-existent native configuration tools for VPX standalone (SDL3/bgfx builds).

**Target: VPinballX 10.8.1** — uses the new folder-per-table layout.
**Cross-platform** — Linux, macOS, Windows. No platform-specific APIs (no Win32, no xrandr). SDL3 only.

**First run** (detected by `wizard_completed` flag in SQLite DB) → configuration wizard.
**Subsequent runs** → table selector / launcher. Wizard can be re-launched with `--config` flag or from the launcher header button; all fields pre-filled from current ini values.

**Current version:** see `Cargo.toml`.

---

## Stack

| Layer | Crate | Role |
|---|---|---|
| UI | `eframe 0.34` + `egui 0.34` (Le-Syl21 fork) | Immediate mode GUI |
| Rotation | `egui-rotate 1.1` (feature: `software-cursor`) | Cabinet viewport rotation + software cursor (as an egui Plugin) |
| Wayland caps | `wayland-client 0.31` (Linux) | Detect `wp_fifo_v1` to pick VPX's SDL driver |
| Images | `egui_extras 0.34` (feature: `image`) | Thumbnail display |
| Display/Input | `sdl3-sys 0.6` (feature: `build-from-source-static`) | Screen enumeration + input capture |
| Config | `serde 1` + `ini-preserve` | Read/write VPinballX.ini (preserves comments) |
| Database | `rusqlite 0.39` (feature: `bundled`) | Local catalog + PinReady config |
| Backglass | `directb2s 0.1` | Extract backglass image from `.directb2s` |
| Display info | `display-info 0.5` | Cross-platform physical size (mm → inches) |
| EDID / hash | `sha2 0.11` + `/sys/class/drm/*/edid` | Driver-independent monitor identity (`display_id.rs`) |
| File scan | `walkdir 2.5` | Recursive .vpx discovery |
| Image codec | `image 0.25` (features: `png`, `jpeg`) | Media pack thumbnails |
| Audio decode | `symphonia 0.5` (features: `ogg`, `vorbis`, `mp3`, `pcm`, `wav`) | Decode OGG/MP3 → PCM for SDL3 |
| i18n | `rust-i18n 4` (fallback = en) | 26 locale files in `locales/` |
| HTTP | `ureq 3` + `zip 8` | GitHub release downloader for VPX fork |
| Threading | `crossbeam-channel 0.5` | SDL3 thread ↔ egui communication |
| Logging | `log 0.4` + `env_logger 0.11` | Debug output |

**No system dependencies required at runtime** — SDL3 and SQLite are statically linked via bundled features. Build-time deps are the standard xcb/xkb/ssl headers for winit.

### Rotation crate + slim egui fork

Cabinet rotation and the software cursor now live in the standalone
**`egui-rotate`** crate (an `egui::Plugin`, published on crates.io) — not the
fork. `egui-rotate 1.1` also owns the OS pointer grab, the soft/hard edge lock,
the keyboard/gamepad auto-hide and the dissolve/reform fade; PinReady registers
one `RotationPlugin` in `main.rs` and no longer wires rotation by hand.

The egui fork is now **slim** — pinned in `[patch.crates-io]` on the
`activation-token-event` branch — and only carries what isn't upstream yet:

- **`eframe::App::transform_primitives` + `post_platform_output` hooks** (pending as [emilk/egui#8138])
- **`ViewportBuilder::with_monitor` / `ViewportCommand::SetMonitor`** — target a specific monitor (used for the PinReady window + the BG/DMD/Topper cover viewports)
- **`Key::ShiftLeft/Right` + `IntlBackslash` physical-key variants** (pending as [emilk/egui#8127])

Rev pinned in `Cargo.toml`.

---

## Architecture

```
src/
  main.rs            # Entry point: CLI, logging, SDL init, cabinet-mode launch, plugin registration
  screens.rs         # SDL3 display enumeration + role assignment
  session.rs         # Display-server session type (wayland / x11), Linux only
  wayland_caps.rs    # wp_fifo_v1 detection + preferred_vpx_driver() decision
  display_id.rs      # EDID parse + MonitorId fingerprint + geometry↔EDID correlation
  display_reconcile.rs # Launch-time: pick VPX driver, reconcile *Display= names + FullScreen mode
  inputs.rs          # Joystick thread (SDL3), controller profile detection, input actions
  tilt.rs            # Tilt/nudge sensitivity settings
  audio.rs           # Audio device detection, routing, ogg/mp3 playback thread
  assets.rs          # Backglass extraction + cache
  config.rs          # VPinballX.ini read/write (preserves comments via ini-preserve)
  db.rs              # SQLite: tables catalog + PinReady config (wizard_completed, display_anchor_*, etc.)
  updater.rs         # VPX-fork auto-install from GitHub releases
  system_info.rs     # OS / session one-liner for crash reports
  i18n.rs            # Language detection + locale loading (26 languages)
  app/
    mod.rs            # App struct, eframe::App impl, kiosk cursor loop, launch-time reconcile
    launcher.rs       # VPX subprocess mgmt, status polling, joystick nav, driver pinning
    launcher_ui.rs    # Grid view, secondary viewports (BG/DMD/Topper covers)
    screens_page.rs   # Wizard: screens + cabinet dimensions + VPX install
    rendering_page.rs # Wizard: sync mode / max framerate / AA / FPS overlay / round ball
    inputs_page.rs    # Wizard: controller profile + keyboard & joystick capture (two columns)
    outputs_page.rs   # Wizard: DOF / output-controller discovery
    tilt_page.rs      # Wizard: tilt/nudge sliders
    audio_page.rs     # Wizard: device routing + SSF test sequence
    tables_dir_page.rs# Wizard: tables directory picker
    system_page.rs    # Wizard: autostart / desktop integration
    save.rs           # Orchestrates writing wizard state to VPinballX.ini (+ persist_anchors)
    autostart.rs      # ~/.config/autostart/pinready.desktop toggle
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

**Two independent bindings per action.** VPX runs `Mapping.X = joy;btn | Key;sc`
as alternatives (either input fires the action), so `InputAction` carries a
separate `keyboard` slot and `joystick` slot. The page shows them as two
columns; the single **Map** button routes the captured event by device type
(a key fills only the keyboard slot, a joystick button only the joystick slot),
and each column has an **× unmap** button (keyboard → back to the VPX default,
joystick → unassigned). Controller detection (Pinscape / DudesCab / PinOne)
fills the joystick slot with its profile defaults, leaving the keyboard binding
intact; a **None** profile opts out. `Escape` is a normal mappable key
(VPX's ExitGame) — auto-map is skipped only via the **Skip** button, never a key.

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
- If dedicated pinball controller detected (Pinscape KL25Z, Pinscape Pico, DudesCab, PinOne) → VPX auto-manages plunger axes and accelerometer nudge. PinReady does NOT map PlungerPos/PlungerVel/NudgeX/NudgeY — those are handled by VPX directly.
- Detection: name contains "Pinscape" or ID contains "PSC" → Pinscape; name contains "DudesCab" → DudesCab; name contains "PinOne" or ID contains "CSD" → PinOne.
- **Four controller profiles** with default button mappings, selectable in the wizard (KL25Z, Pico, DudesCab, PinOne). The ecosystem is well-covered — all mainstream VP-dedicated HID boards are here. LedWiz/PacLed are output-only and out of scope.

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

**Profile 3 — PinOne (Cleveland Software Design)**
Detected by name containing "PinOne" or ID prefix `CSD`. Nudge axes use Acceleration type (not Position) — the board exposes raw accelerometer data, not pre-integrated position.

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

**Never touch `Plugin.PinMAME.*`** — PinReady must never disable PinMAME audio (or any PinMAME setting). Only settings PinReady writes:
- `[Player]` — PlayfieldDisplay/Width/Height/FullScreen, BGSet, NudgeStrength, MusicVolume, SoundVolume, rendering options
- `[Backglass]` / `[ScoreView]` / `[Topper]` — Output, Display, Width, Height, **FullScreen** (0 windowed on real Xorg, 1 borderless FS otherwise — see *Display driver + window placement*)
- `[Input]` — Device.* + Mapping.*
- `[Plugin.B2SLegacy]` — backglass overlay flags (B2SHideGrill, ScoreViewDMDOverlay, etc.) only when no DMD screen is present
Only write back keys that already exist in the file, or that are explicitly documented by VPX.

**Rendering options the wizard exposes** (`[Player]`):
- `SyncMode` — the four VPX values surfaced in the combo: `0` No sync, `1` Vertical sync, `2` Adaptive sync, `3` Frame pacing (VPX's own default). Each has an explanatory tooltip.
- `MaxFramerate` — auto-set to the playfield's detected refresh rate.
- `ShowFPS` — FPS overlay, pre-checked (`1`).
- `BallAntiStretch` — "Round ball", pre-checked (`1` — VPX defaults it off): compensates the render stretch that makes the ball look egg-shaped in fast motion. Cosmetic only.
- AA / MSAA / FXAA / Sharpen / PFReflection / MaxTexDimension.

**First run**: if `VPinballX.ini` is absent, the wizard runs `VPX -h` once (writes the full documented default ini in ~0.1 s without opening a window) and adopts it before layering the user's choices — so the ini stays the canonical annotated file rather than a sparse PinReady-only one.

---

## Cabinet / kiosk mode (v0.6.0)

When VPX config has `BGSet = 1` (Cabinet) **and** the wizard is already completed, PinReady launches in kiosk mode:

- **Main PF viewport**: created via `ViewportBuilder::with_monitor(idx)` + `with_decorations(false)`, borderless fullscreen directly on the Playfield monitor — no position loop, no WM races. Rotation is CW90 (pincab playfields are laid flat but the monitor reports landscape) applied by the `egui-rotate` `RotationPlugin` registered on the root viewport, not by the ViewportBuilder.
- **Secondary viewports** (BG / DMD / Topper): created via `show_viewport_deferred` with `with_monitor(idx)` and `with_active(false)` so they don't steal keyboard focus from the PF. The plugin only rotates the root viewport, so these render upright. They show:
  - BG → backglass image (from `.directb2s` extraction via `directb2s` crate)
  - DMD + Topper → grey cover with tinted VPX logo
- **Kiosk cursor**: the `egui-rotate` `RotationPlugin` owns the software cursor (scale 3× for far-viewing playfields, hard lock, and the OS pointer grab it applies itself). The `App::ui` loop (gated on `kiosk_cursor && !vpx_running`) only handles the compositor workarounds: force a repaint each frame (Mutter Wayland skips presenting unfocused surfaces), reclaim focus when unfocused (X11 only), keep the OS pointer hidden, and seed `SoftwareCursor::set_virtual_pos` once the viewport rect is known.

### Display driver + window placement across X11 / XWayland / Wayland

This is the load-bearing part of cabinet mode. The same physical monitor is
**named differently** by SDL depending on the video driver, and VPX places its
windows by the `*Display=` name in the ini — so the names must match the driver
VPX actually runs under, and that driver must be chosen for framerate.

**The driver decision** (`wayland_caps::preferred_vpx_driver`):
- Wayland session **with** `wp_fifo_v1` (Mutter ≥ 48) → native `wayland` (proper FIFO, full refresh).
- Wayland session **without** it (Mutter ≤ 46) → `x11` (XWayland). Native Wayland there frame-callback-throttles VPX to ¼ refresh (120→30); XWayland lifts it to ~60.
- Real Xorg session → `x11`, which reaches the **full** refresh (120).

Detection is a plain Wayland registry enumeration (`wayland-client`, already in
tree via winit) looking for `wp_fifo_manager_v1`.

**The name bridge** (`display_id.rs` + `display_reconcile.rs`). SDL names diverge
badly: e.g. the same 42″ playfield is `Iiyama North America 42"` (Wayland),
`DP-1 42"` (XWayland) and `PL4380UH 42"` (real Xorg). So a role is anchored on
**geometry (layout position + size)** plus, as a fallback, its **EDID
fingerprint** — both driver-independent:
- The wizard persists `role → DisplayAnchor { x, y, w, h, fingerprint }` in the DB (`save.rs::persist_anchors`). The fingerprint is a SHA-256 of the 128-byte EDID base block, read from `/sys/class/drm/*/edid` (kernel layer, identical across drivers). Junk-EDID panels (no serial, native mode ≠ driven mode — e.g. a no-name DMD) get `fingerprint: None` and are tracked by position alone.
- At **launcher startup** (`App::new` → `choose_driver_and_reconcile`): pick the driver, enumerate SDL displays under it (via a `--enumerate-displays <driver>` subprocess so SDL's driver is isolated from PinReady's own winit stack), correlate them to the DRM EDID, resolve each role's anchor → rewrite only the `*Display=` names (positions/refresh untouched), atomically (all roles resolve or nothing changes). VPX is then launched pinned to that driver so it finds its screens.
- The startup "should we re-run the wizard?" check (`unresolvable_assigned_displays`) is **anchor-aware**: a role is fine if its anchor still resolves to a connected monitor, regardless of the ini's current name. So an X11↔Wayland session switch is transparent — the old session-change wizard trigger was removed.

**FullScreen mode is also per-session** (`reconcile_fullscreen`): under a **real
Xorg** session, gnome-shell does not map a borderless-fullscreen window on a
non-primary output (VPX logs the BG/DMD windows as created but they show the
desktop), so secondary displays are set to windowed (`FullScreen = 0`); under
Wayland/XWayland they stay borderless fullscreen (`1`). The playfield (primary)
maps fine at borderless fullscreen everywhere.

`with_monitor(idx)` (used for PinReady's *own* PF window + cover viewports) still
relies on `self.displays` matching winit's `available_monitors()` order — fine
on the Wayland session PinReady's UI runs under. VPX window placement no longer
depends on that index; it goes through the name bridge above.

### VPX launch lifecycle

When the user selects a table:
1. `launch_table` spawns `VPinballX_BGFX -play <vpx>` as a subprocess.
2. `vpx_running` flips to `true`. Stdout is read in a helper thread and parsed for `SetProgress <pct>%`, `Startup done`, `PluginLog`, etc. Messages stream into the launcher's loading overlay.
3. On `Startup done`:
   - `vpx_hide_covers = true` → secondary cover viewports stop rendering, so VPX's own BG/DMD windows become visible
   - `SoftwareCursor::release()` → the plugin drops its OS grab and stops hiding the pointer, so VPX can read the mouse
   - Kiosk focus-reclaim stops (gated on `!vpx_running`). VPX windows naturally z-order above PinReady
   - The `*Display=` names + FullScreen mode were already reconciled to VPX's driver at launcher startup (see *Display driver + window placement*), and `SDL_VIDEODRIVER` is pinned to that driver when spawning VPX
4. On VPX exit (`ExitOk` / `ExitError` / `LaunchError`):
   - `vpx_running = false` → kiosk loop resumes, reclaiming focus + re-warping the cursor
   - `vpx_hide_covers = false` → secondary covers render again
   - `kiosk_cursor_warped = false` → triggers a fresh warp + focus on next frame

No `Minimized(true)` on the PF. Attempted once; Mutter does not reliably restore borderless-fullscreen windows with `Minimized(false)`.

### Why a forked egui?

Rotation and the software cursor left the fork and live in the published
`egui-rotate` crate (see *Rotation crate + slim egui fork* above). The remaining
fork (`Le-Syl21/egui`, branch `activation-token-event`) only holds three things
not yet upstream: the `transform_primitives` / `post_platform_output` eframe
hooks ([#8138]), `ViewportBuilder::with_monitor` / `ViewportCommand::SetMonitor`,
and the `ShiftLeft/Right` + `IntlBackslash` physical-key variants ([#8127]).
When those land upstream, drop the patch.

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
- **No system dependencies at runtime** — use bundled features for SDL3 and SQLite. Zero external tools in PATH (ever).
- **Subprocess for VPX only** — use `std::process::Command` to launch tables, no FFI linking
- **Unsafe SDL3 calls** — wrap in safe Rust functions in `screens.rs` and `inputs.rs`, never expose raw pointers to other modules
- **Error handling** — use `anyhow` or explicit `Result` types, no `unwrap()` in production paths
- **Config writes preserve comments** — via `ini-preserve` crate, read full ini → modify → write back
- **SQLite catalog** — initialize schema on first run, upsert on rescan
- **First run detection** — `wizard_completed` flag in SQLite DB (not ini existence, not filesystem heuristics). `--config` CLI flag forces wizard re-entry
- **Display enumeration order must match winit** — `screens.rs` does not reorder `self.displays` after enumeration; roles are assigned via parallel index sort. `ViewportBuilder::with_monitor(idx)` depends on this invariant
- **Translation keys** — most `t!()` calls use string literals (static grep catches those). `input_*` action labels are looked up dynamically via `t!(action.label)` where `label` is a string field in `inputs.rs`. When auditing "dead" keys, a grep-only audit will miss these — verify via runtime usage before deleting
- **Before every `git push`** — run `cargo fmt` and `cargo clippy --all-targets -- -D warnings`. Both must be clean. The CI enforces `cargo fmt --check` on Linux x86_64 and fails the whole release pipeline otherwise. Pushing unformatted or clippy-dirty code breaks the release build

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

**For iteration / pincab redeploy, use `--profile=fast` instead of `--release`.**
The `release` profile bundles full LTO + single codegen unit (~3 min, 40 MB
stripped binary, only worth paying when shipping a tagged release). The
`debug` default produces a runnable binary in ~30 s but it weighs 200+ MB
which makes scp-to-pincab painfully slow over Wi-Fi. The `fast` profile
(defined in `Cargo.toml`) inherits `release` but disables LTO and uses 16
codegen units — same `opt-level=3`, same stripped output, ~60 s build,
~80 MB binary. That's the right default for the develop → scp-pincab →
test loop. Output lands in `target/fast/pinready`. Keep `--release` for
the actual `git tag vX.Y.Z` build.

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
