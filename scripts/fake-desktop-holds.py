#!/usr/bin/env python3
"""A stand-in for the desktop's screen saver and KDE's notification
inhibition, for scripts/gamemode-e2e.sh. Run it on a private test bus only.

Writes the active holds to the file given as the first argument after every
change, one line per hold: "screen <cookie>" or "notifications <cookie>".
"""
import sys
from gi.repository import Gio, GLib

OUT = sys.argv[1]
holds = {}  # cookie -> kind
next_cookie = [1]

XML = """
<node>
  <interface name="org.freedesktop.ScreenSaver">
    <method name="Inhibit"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="u" direction="out"/></method>
    <method name="UnInhibit"><arg type="u" direction="in"/></method>
  </interface>
  <interface name="org.freedesktop.Notifications">
    <method name="Inhibit"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="u" direction="out"/></method>
    <method name="UnInhibit"><arg type="u" direction="in"/></method>
  </interface>
</node>
"""


def write():
    with open(OUT, "w") as f:
        for c, k in sorted(holds.items()):
            f.write(f"{k} {c}\n")


def call(conn, sender, path, iface, method, params, inv):
    kind = "screen" if iface == "org.freedesktop.ScreenSaver" else "notifications"
    if method == "Inhibit":
        c = next_cookie[0]
        next_cookie[0] += 1
        holds[c] = kind
        write()
        inv.return_value(GLib.Variant("(u)", (c,)))
    else:
        holds.pop(params.unpack()[0], None)
        write()
        inv.return_value(None)


node = Gio.DBusNodeInfo.new_for_xml(XML)
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
for path, iface in [("/org/freedesktop/ScreenSaver", 0), ("/org/freedesktop/Notifications", 1)]:
    bus.register_object(path, node.interfaces[iface], call, None, None)
for name in ["org.freedesktop.ScreenSaver", "org.freedesktop.Notifications"]:
    Gio.bus_own_name_on_connection(bus, name, Gio.BusNameOwnerFlags.NONE, None, None)
write()
GLib.MainLoop().run()
