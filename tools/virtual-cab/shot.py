#!/usr/bin/env python3
"""Save one frame of a monitor through org.gnome.Mutter.ScreenCast. Unlike
X11 tools, this sees what the compositor shows: Wayland and XWayland windows,
OpenGL or Vulkan alike.

usage: shot.py CONNECTOR OUT.png   (connectors: Meta-0, Meta-1... left to right)
"""
import subprocess
import sys

import dbus
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

if len(sys.argv) != 3:
    sys.exit(__doc__)
DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
NAME = "org.gnome.Mutter.ScreenCast"
cast = dbus.Interface(bus.get_object(NAME, "/org/gnome/Mutter/ScreenCast"), NAME)
session = dbus.Interface(
    bus.get_object(NAME, cast.CreateSession(dbus.Dictionary({}, signature="sv"))),
    NAME + ".Session",
)
stream_path = session.RecordMonitor(sys.argv[1], dbus.Dictionary({"cursor-mode": dbus.UInt32(1)}, signature="sv"))
stream = bus.get_object(NAME, stream_path)
loop = GLib.MainLoop()
node = {}


def added(node_id):
    node["id"] = int(node_id)
    loop.quit()


stream.connect_to_signal("PipeWireStreamAdded", added, dbus_interface=NAME + ".Stream")
session.Start()
GLib.timeout_add_seconds(10, loop.quit)
loop.run()
if "id" not in node:
    sys.exit("no PipeWire stream (is pipewire running?)")
subprocess.run(
    ["gst-launch-1.0", "-q", "pipewiresrc", f"path={node['id']}", "num-buffers=3", "!",
     "videoconvert", "!", "pngenc", "snapshot=true", "!", "filesink", f"location={sys.argv[2]}"],
    timeout=30, check=True,
)
session.Stop()
