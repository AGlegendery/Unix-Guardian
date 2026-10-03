#!/usr/bin/env python3
"""A stand-in for KDE's GlobalShortcuts portal and kglobalaccel, for
scripts/shortcuts-e2e.sh. Run it on a private test bus only.

Behaves the way KDE does where the agent depends on it: binding keeps keys the
store already has (the preferred trigger only counts the first time),
setForeignShortcutKeys changes the store and the portal reports the change
(ShortcutsChanged), and globalShortcutsByKey says who holds a key. Meta+E is
held by "KWin" from the start. org.unixguardian.Test.Press(id) presses a bound
shortcut (emits Activated).

Only letters and F1-F24 with Ctrl/Alt/Shift/Meta are understood.
"""
from gi.repository import Gio, GLib

PORTAL = "/org/freedesktop/portal/desktop"
MODS = [("CTRL", "Ctrl", 0x04000000), ("ALT", "Alt", 0x08000000),
        ("SHIFT", "Shift", 0x02000000), ("LOGO", "Meta", 0x10000000)]
LABEL_ORDER = ["Meta", "Ctrl", "Alt", "Shift"]

# (component, action) -> [qt key code, friendly component, friendly action]
store = {("kwin", "Window Close"): [0x10000000 | ord("E"), "KWin", "Close Window"]}
sessions = {}  # session path -> app id
app_ids = {}   # sender -> registered app id


def xdg_to_qt(t):
    *mods, key = t.split("+")
    code = 0x01000030 + int(key[1:]) - 1 if key[0] == "F" and key[1:].isdigit() else ord(key.upper())
    for name, _, bit in MODS:
        if name in mods:
            code |= bit
    return code


def label(code):
    names = {lab: bit for _, lab, bit in MODS}
    key = code & 0x01FFFFFF
    k = f"F{key - 0x01000030 + 1}" if key >= 0x01000030 else chr(key)
    return "+".join([m for m in LABEL_ORDER if code & names[m]] + [k])


XML = """
<node>
  <interface name="org.freedesktop.portal.GlobalShortcuts">
    <method name="CreateSession"><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
    <method name="BindShortcuts"><arg type="o" direction="in"/><arg type="a(sa{sv})" direction="in"/>
      <arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
    <property name="version" type="u" access="read"/>
    <signal name="Activated"><arg type="o"/><arg type="s"/><arg type="t"/><arg type="a{sv}"/></signal>
    <signal name="ShortcutsChanged"><arg type="o"/><arg type="a(sa{sv})"/></signal>
  </interface>
  <interface name="org.freedesktop.host.portal.Registry">
    <method name="Register"><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/></method>
  </interface>
  <interface name="org.unixguardian.Test">
    <method name="Press"><arg type="s" direction="in"/></method>
  </interface>
</node>"""

SESSION_XML = """<node><interface name="org.freedesktop.portal.Session">
  <method name="Close"/></interface></node>"""

KDE_XML = """<node><interface name="org.kde.KGlobalAccel">
  <method name="shortcutKeys"><arg type="as" direction="in"/><arg type="a(ai)" direction="out"/></method>
  <method name="setForeignShortcutKeys"><arg type="as" direction="in"/><arg type="a(ai)" direction="in"/></method>
  <method name="globalShortcutsByKey"><arg type="(ai)" direction="in"/><arg type="(i)" direction="in"/>
    <arg type="a(ssssssaiai)" direction="out"/></method>
</interface></node>"""


def shortcut_list(app):
    return [(a, {"description": GLib.Variant("s", v[2]),
                 "trigger_description": GLib.Variant("s", label(v[0]))})
            for (c, a), v in store.items() if c == app]


def respond(conn, sender, token, results):
    path = f"{PORTAL}/request/{sender[1:].replace('.', '_')}/{token}"
    GLib.idle_add(lambda: conn.emit_signal(None, path, "org.freedesktop.portal.Request", "Response",
                                           GLib.Variant("(ua{sv})", (0, results))) and False)
    return path


def portal_call(conn, sender, path, iface, method, params, inv):
    p = params.unpack()
    if method == "Register":
        app_ids[sender] = p[0]
        return inv.return_value(None)
    if method == "Press":
        for s, app in sessions.items():
            conn.emit_signal(None, PORTAL, "org.freedesktop.portal.GlobalShortcuts", "Activated",
                             GLib.Variant("(osta{sv})", (s, p[0], 0, {})))
        return inv.return_value(None)
    if method == "CreateSession":
        opts = p[0]
        spath = f"{PORTAL}/session/{sender[1:].replace('.', '_')}/{opts['session_handle_token']}"
        sessions[spath] = app_ids.get(sender, "")
        node = Gio.DBusNodeInfo.new_for_xml(SESSION_XML)
        reg = []

        def close(c, *_a):
            sessions.pop(spath, None)
            c.unregister_object(reg[0])
            _a[-1].return_value(None)
        reg.append(conn.register_object(spath, node.interfaces[0], close, None, None))
        rpath = respond(conn, sender, opts["handle_token"], {"session_handle": GLib.Variant("s", spath)})
        return inv.return_value(GLib.Variant("(o)", (rpath,)))
    if method == "BindShortcuts":
        spath, wanted, _parent, opts = p
        app = sessions.get(spath, "")
        for sid, props in wanted:
            # Like KDE: keys already in the store win over the suggestion.
            if (app, sid) not in store:
                store[(app, sid)] = [xdg_to_qt(props["preferred_trigger"]), app, props["description"]]
        rpath = respond(conn, sender, opts["handle_token"],
                        {"shortcuts": GLib.Variant("a(sa{sv})", shortcut_list(app))})
        return inv.return_value(GLib.Variant("(o)", (rpath,)))


def portal_prop(conn, sender, path, iface, prop):
    return GLib.Variant("u", 1)


def kde_call(conn, sender, path, iface, method, params, inv):
    p = params.unpack()
    if method == "shortcutKeys":
        v = store.get((p[0][0], p[0][1]))
        return inv.return_value(GLib.Variant("(a(ai))", ([([v[0]],)] if v else [],)))
    if method == "setForeignShortcutKeys":
        comp, action = p[0][0], p[0][1]
        if (comp, action) in store and p[1]:
            store[(comp, action)][0] = p[1][0][0][0]
            for s, app in sessions.items():
                if app == comp:
                    conn.emit_signal(None, PORTAL, "org.freedesktop.portal.GlobalShortcuts", "ShortcutsChanged",
                                     GLib.Variant("(oa(sa{sv}))", (s, shortcut_list(comp))))
        return inv.return_value(None)
    if method == "globalShortcutsByKey":
        code = p[0][0][0]
        users = [("default", "Default", c, v[1], a, v[2], [v[0]], [v[0]])
                 for (c, a), v in store.items() if v[0] == code]
        return inv.return_value(GLib.Variant("(a(ssssssaiai))", (users,)))


def on_bus(conn, _name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    for i in node.interfaces:
        conn.register_object(PORTAL, i, portal_call, portal_prop, None)
    kde = Gio.DBusNodeInfo.new_for_xml(KDE_XML)
    conn.register_object("/kglobalaccel", kde.interfaces[0], kde_call, None, None)
    Gio.bus_own_name_on_connection(conn, "org.kde.kglobalaccel", 0, None, None)


Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.portal.Desktop", 0, on_bus, None, None)
GLib.MainLoop().run()
