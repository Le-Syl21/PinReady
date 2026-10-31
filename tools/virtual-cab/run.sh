#!/bin/bash
# Start a virtual cabinet: run.sh [pinready arguments]
#
#   CAB_UBUNTU    24.04 (default) or 26.04 — the LTS to emulate
#   PINREADY_BIN  the PinReady binary to run (required)
#   CAB_HOME      host folder used as the session's $HOME (default ./home-<ubuntu>)
#   VPX_DIR       a Visual Pinball build, mounted at /opt/vpx (optional)
#   TABLES_DIR    a tables folder, mounted at /tables (optional, writable:
#                 Visual Pinball writes nvram and caches next to the table)
#   CAB_MONITORS  the screens, e.g. "3840x2160 2560x1440 1920x1080 1920x480"
#   COMPOSITOR    gnome-shell (default) or mutter
#
# The container runs as you, on the host network: egui_mcp (with PinReady
# built with --features inspection) reaches the GUI on 127.0.0.1:5719.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
ubuntu=${CAB_UBUNTU:-24.04}
image=pinready-virtual-cab:$ubuntu
home=${CAB_HOME:-$here/home-$ubuntu}
: "${PINREADY_BIN:?set PINREADY_BIN to the PinReady binary to run}"
mkdir -p "$home"
docker image inspect "$image" >/dev/null 2>&1 \
  || docker build -t "$image" -f "$here/Dockerfile.ubuntu-$ubuntu" "$here"
mounts=(-v "$home:/home/cab" -v "$(realpath "$PINREADY_BIN"):/app/pinready:ro")
[ -n "${VPX_DIR:-}" ] && mounts+=(-v "$(realpath "$VPX_DIR"):/opt/vpx:ro")
[ -n "${TABLES_DIR:-}" ] && mounts+=(-v "$(realpath "$TABLES_DIR"):/tables")
docker rm -f pinready-virtual-cab >/dev/null 2>&1 || true
exec docker run -d --name pinready-virtual-cab --network host \
  --user "$(id -u):$(id -g)" --tmpfs /tmp/.X11-unix:mode=1777 \
  -e HOME=/home/cab -e LANG="${LANG:-en_US.UTF-8}" -e LIBGL_ALWAYS_SOFTWARE=1 \
  -e SDL_AUDIODRIVER=dummy -e EGUI_INSPECTION=1 -e PINREADY_CURSOR_DIAG=1 \
  -e RUST_LOG="${RUST_LOG:-info}" -e CAB_MONITORS -e COMPOSITOR \
  "${mounts[@]}" "$image" /entry.sh "$@"
