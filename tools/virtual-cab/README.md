# Virtual cabinet

A pinball cabinet without the cabinet: a headless GNOME session in Docker,
with one virtual monitor per screen (playfield, backglass, DMD, topper), to
run PinReady and Visual Pinball in cabinet mode, drive them with real input
and capture what each screen shows.

It runs on Ubuntu 24.04 (GNOME 46) or 26.04 (GNOME 50), the two LTS releases
PinReady targets, so a change can be checked on both before it reaches a real
cabinet.

## What you need

- Docker, and a Linux host (the container runs as you, on the host network)
- A PinReady binary (`cargo build`; add `--features inspection` to drive the
  GUI with [egui_mcp](https://github.com/rerun-io/kittest_inspector))
- Optionally a Visual Pinball build (the extracted `VPinballX_BGFX` folder) and
  a tables folder

No GPU is used: Mesa's software renderer (llvmpipe) draws everything. A 4K
playfield is slow but plays.

## Start

```bash
cd tools/virtual-cab
PINREADY_BIN=../../target/debug/pinready \
VPX_DIR=~/vpx/VPinballX_BGFX-10.8.1-6501 \
TABLES_DIR=~/my-test-tables \
CAB_UBUNTU=26.04 ./run.sh
```

The first run builds the image. The session's `$HOME` is `home-<ubuntu>/` next
to the scripts: PinReady's database, the `VPinballX.ini`, and the logs
(`pinready-stdout.log`, `compositor.log`) are there.

In the wizard, the Visual Pinball executable is `/opt/vpx/VPinballX_BGFX` and
the tables folder is `/tables`. Use a copy of your tables: Visual Pinball
writes nvram and caches into each table's folder.

The default screens are a 4K playfield, a 1440p backglass, a 1080p DMD and a
1920x480 topper, laid out left to right. Change them with
`CAB_MONITORS="3840x2160 1920x1080 1920x1080"`.

## Drive it

`cab.sh` runs a command inside the session:

```bash
./cab.sh input.py key ff1b           # Escape: leave the overview GNOME opens on
./cab.sh input.py click              # a left click where the pointer is
./cab.sh input.py move -20 0 10      # move the pointer, 10 steps of (-20, 0)
./cab.sh input.py key ff0d           # Return: launch the selected table
./cab.sh shot.py Meta-2 /home/cab/dmd.png   # capture the DMD screen
```

The input goes through the compositor (`org.gnome.Mutter.RemoteDesktop`), as a
keyboard, a cabinet button or a mouse would: it reaches the window that has
the focus, and it gives xdg-activation the input it ties a token to. Events
injected inside egui by egui_mcp do neither.

`shot.py` captures a monitor through `org.gnome.Mutter.ScreenCast`, which sees
OpenGL and Vulkan windows; X11 tools such as `xwd` cannot read them under
XWayland. Monitors are `Meta-0`, `Meta-1`... in layout order.

With `--features inspection`, egui_mcp reaches PinReady's GUI on
`127.0.0.1:5719`. When the wizard finishes and PinReady reopens as the
launcher in the same process, the inspection port stays with the first
window: restart the container to inspect the launcher.

## Things worth knowing

- **GNOME opens on the overview**, as at a desktop login. Nothing has the
  keyboard focus until it is left: send Escape first.
- **Visual Pinball's display ids.** From rev 5515 VPX names a screen by its SDL
  name and position, `Meta-1 [3840, 0]`; a bare name matches nothing and every
  window opens on the playfield. Its log says so: `The selected display
  "Meta-1" is not available`. The virtual monitors are all called
  `MetaVendor` under the Wayland driver, which makes the position the only
  thing that tells them apart, as with two identical real screens.
- **The driver PinReady picks for VPX** follows the session: native Wayland
  where GNOME offers `wp_fifo_v1` (GNOME 48 and later, so 26.04), XWayland
  otherwise. The log line `VPX display driver: ...` says which.
- **Visual Pinball aborts** when `LANG` names a locale that is not generated;
  the image generates `en_US.UTF-8` and `fr_FR.UTF-8`.
- **Focus is not a real seat's.** Keyboard focus in a headless session does
  not always follow what a cabinet would do: confirm focus behaviour on real
  hardware before drawing conclusions from it.

## Windows 11

The Docker session is Linux only. PinReady 0.23 was also checked on a
Windows 11 virtual machine with four virtual screens; the recipe:

- **A GPU in the VM** (PCI passthrough here). The VM's own display adapter
  (VirtIO, QXL...) drives a single screen and has no OpenGL, which PinReady
  needs: disable it once the screens below are up.
- **Four screens** from [Virtual Display Driver](https://github.com/VirtualDrivers/Virtual-Display-Driver):
  in `C:\VirtualDisplayDriver\vdd_settings.xml` set the count to 4, the GPU's
  name as `friendlyname` (so that GPU renders them) and the cabinet's
  resolutions (3840x2160, 2560x1440, 1920x1080, 1920x480), then pick each
  screen's mode in Windows' display settings.
- **Real input.** In cabinet mode PinReady draws its own cursor and moves it
  by relative mouse motion: tools that set an absolute pointer position
  land nowhere, move the mouse instead.

Things seen there: Visual Pinball decodes a big table's images for up to a
minute without logging anything, and the game opens paused until its
playfield window gets a real focus change.
