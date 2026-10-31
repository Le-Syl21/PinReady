#!/bin/bash
# Run a command inside the running virtual cabinet, with its session set up:
#   cab.sh input.py key ff0d        cab.sh shot.py Meta-2 /home/cab/dmd.png
exec docker exec -e XDG_RUNTIME_DIR=/home/cab/.xdg \
  -e DBUS_SESSION_BUS_ADDRESS=unix:path=/home/cab/.xdg/bus \
  -e WAYLAND_DISPLAY=wayland-0 pinready-virtual-cab "$@"
