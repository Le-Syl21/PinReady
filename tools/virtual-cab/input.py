#!/usr/bin/env python3
"""Real input through org.gnome.Mutter.RemoteDesktop: the events go through
the compositor like a keyboard, a cabinet button or a mouse, unlike events
injected inside egui.

usage: input.py key KEYSYM_HEX       (ff0d Return, ff1b Escape, ff53 Right...)
       input.py click
       input.py move DX DY [STEPS]   (relative motion, STEPS times)
"""
import sys
import time

import dbus

bus = dbus.SessionBus()
rd = dbus.Interface(
    bus.get_object("org.gnome.Mutter.RemoteDesktop", "/org/gnome/Mutter/RemoteDesktop"),
    "org.gnome.Mutter.RemoteDesktop",
)
session = dbus.Interface(
    bus.get_object("org.gnome.Mutter.RemoteDesktop", rd.CreateSession()),
    "org.gnome.Mutter.RemoteDesktop.Session",
)
session.Start()
time.sleep(0.3)
what = sys.argv[1]
if what == "key":
    keysym = dbus.UInt32(int(sys.argv[2], 16))
    session.NotifyKeyboardKeysym(keysym, True)
    time.sleep(0.15)
    session.NotifyKeyboardKeysym(keysym, False)
elif what == "click":
    BTN_LEFT = dbus.Int32(0x110)
    session.NotifyPointerButton(BTN_LEFT, True)
    time.sleep(0.1)
    session.NotifyPointerButton(BTN_LEFT, False)
elif what == "move":
    dx, dy = float(sys.argv[2]), float(sys.argv[3])
    for _ in range(int(sys.argv[4]) if len(sys.argv) > 4 else 10):
        session.NotifyPointerMotionRelative(dbus.Double(dx), dbus.Double(dy))
        time.sleep(0.03)
else:
    sys.exit(__doc__)
time.sleep(0.3)
session.Stop()
