#!/bin/bash
# Inside the container: the session a GNOME desktop login would set up, a
# headless gnome-shell with one virtual monitor per screen of the cabinet,
# then PinReady. Logs land in $HOME (mounted from the host).
set -u
export XDG_RUNTIME_DIR=$HOME/.xdg
rm -rf "$XDG_RUNTIME_DIR"; mkdir -p -m 700 "$XDG_RUNTIME_DIR"
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=GNOME

# Session bus, and a second user bus standing in for the system bus that
# gnome-shell opens at start-up (there is none in a container)
dbus-daemon --session --address="unix:path=$XDG_RUNTIME_DIR/bus" --fork --nopidfile
dbus-daemon --session --address="unix:path=$XDG_RUNTIME_DIR/system_bus" --fork --nopidfile
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/system_bus"

# PipeWire carries the frames of the ScreenCast captures (shot.py)
(pipewire > "$HOME/pipewire.log" 2>&1 &)
sleep 0.5
(wireplumber > "$HOME/wireplumber.log" 2>&1 &)

# One --virtual-monitor per screen, laid out left to right in this order
MONITORS=()
for m in ${CAB_MONITORS:-3840x2160 2560x1440 1920x1080 1920x480}; do
  MONITORS+=(--virtual-monitor "$m")
done
${COMPOSITOR:-gnome-shell} --headless --wayland "${MONITORS[@]}" > "$HOME/compositor.log" 2>&1 &
for _ in $(seq 100); do [ -S "$XDG_RUNTIME_DIR/wayland-0" ] && break; sleep 0.2; done
export WAYLAND_DISPLAY=wayland-0

# What gnome-session exports for X clients: XWayland's display and its auth
for _ in $(seq 100); do ls "$XDG_RUNTIME_DIR"/.mutter-Xwaylandauth.* >/dev/null 2>&1 && break; sleep 0.2; done
export DISPLAY=:0 XAUTHORITY="$(ls "$XDG_RUNTIME_DIR"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)"

exec /app/pinready "$@" > "$HOME/pinready-stdout.log" 2>&1
