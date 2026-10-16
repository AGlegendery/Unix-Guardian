#!/usr/bin/env python3
# Checks that every animated icon really moves, in the window's own engine
# (WebKitGTK 4.1, what Tauri uses on Linux): loads app/dev/icons.html from a
# running dev server, hovers each icon in turn and compares frames taken
# during its run with the still icon. An icon whose pixels barely change
# fails. No window is shown (an offscreen GTK window).
#
#   cd app && npm run dev &      # serves http://localhost:5173
#   python3 scripts/icon-motion-check.py [--motion classic|lively]
#                                        [--only a,b] [--strip out.png]
#
# --strip writes every icon's frames as one image, a row per icon.
#
# Needs python-gobject and webkit2gtk-4.1, and a desktop session (GTK needs
# a display even offscreen).

import argparse
import json
import os
import sys

os.environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
os.environ.setdefault("WEBKIT_DISABLE_COMPOSITING_MODE", "1")

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf, GLib, Gtk, WebKit2  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--url", default="http://localhost:5173/dev/icons.html")
ap.add_argument("--motion", default="classic")
ap.add_argument("--only", default="")
ap.add_argument("--strip", default="")
# Share of the icon's ink pixels that must change in the most-moved frame.
ap.add_argument("--min-change", type=float, default=0.1)
args = ap.parse_args()

SHOTS = [80, 160, 240, 330, 430, 550, 700, 900]
SCALE = 3

win = Gtk.OffscreenWindow()
view = WebKit2.WebView()
win.add(view)
win.set_default_size(1100, 800)
win.show_all()

st = {"i": 0, "names": [], "rect": None, "frames": [], "still": None}
results = []
rows = []


def js(code, cb=None):
    def done(v, res, _data=None):
        cb(v.evaluate_javascript_finish(res).to_string())

    view.evaluate_javascript(code, -1, None, None, None, done if cb else None, None)


def grab():
    x, y, w, h = st["rect"]
    return win.get_pixbuf().new_subpixbuf(x, y, w, h).copy()


def ink(pb):
    """Pixel brightness as a flat list (the icons are light on dark)."""
    data = pb.get_pixels()
    n, stride, w, h = pb.get_n_channels(), pb.get_rowstride(), pb.get_width(), pb.get_height()
    return [data[r * stride + c * n] + data[r * stride + c * n + 1] + data[r * stride + c * n + 2]
            for r in range(h) for c in range(w)]


def change(a, b):
    lit = sum(1 for v in a if v > 200) or 1
    moved = sum(1 for p, q in zip(a, b) if abs(p - q) > 120)
    return moved / lit


def begin():
    js("JSON.stringify([...document.querySelectorAll('.gi')].map(b => b.title))", names)
    return False


def names(s):
    st["names"] = json.loads(s)
    if not st["names"]:
        print("no icons on the page", file=sys.stderr)
        Gtk.main_quit()
        return
    GLib.timeout_add(300, nxt)


def nxt():
    i = st["i"]
    if i >= len(st["names"]):
        return finish()
    js(f"""(() => {{ const b = document.querySelectorAll('.gi')[{i}];
        b.scrollIntoView({{ block: 'center' }});
        const r = b.querySelector('svg').getBoundingClientRect();
        return JSON.stringify([r.x - 6, r.y - 6, r.width + 12, r.height + 12]); }})()""", rect)
    return False


def rect(s):
    st["rect"] = [max(0, int(v)) for v in json.loads(s)]
    st["frames"] = []
    GLib.timeout_add(120, hover)


def hover():
    st["still"] = grab()
    js(f"document.querySelectorAll('.gi')[{st['i']}].dispatchEvent(new PointerEvent('pointerenter'))")
    for t in SHOTS:
        GLib.timeout_add(t, shot)
    GLib.timeout_add(SHOTS[-1] + 200, score)
    return False


def shot():
    st["frames"].append(grab())
    return False


def score():
    name = st["names"][st["i"]]
    js(f"document.querySelectorAll('.gi')[{st['i']}].dispatchEvent(new PointerEvent('pointerleave'))")
    still = ink(st["still"])
    best = max(change(still, ink(f)) for f in st["frames"])
    results.append((name, best))
    rows.append([st["still"]] + st["frames"])
    st["i"] += 1
    # Let a looping icon settle before the next one is measured.
    GLib.timeout_add(500, nxt)
    return False


def finish():
    if args.strip and rows:
        fw = max(f.get_width() for r in rows for f in r) * SCALE
        fh = max(f.get_height() for r in rows for f in r) * SCALE
        out = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, True, 8, fw * len(rows[0]), fh * len(rows))
        out.fill(0x202020FF)
        for k, r in enumerate(rows):
            for j, f in enumerate(r):
                big = f.scale_simple(f.get_width() * SCALE, f.get_height() * SCALE, GdkPixbuf.InterpType.BILINEAR)
                big.copy_area(0, 0, big.get_width(), big.get_height(), out, j * fw, k * fh)
        out.savev(args.strip, "png", [], [])
    bad = [(n, c) for n, c in results if c < args.min_change]
    for n, c in sorted(results, key=lambda r: r[1]):
        print(f"{'FAIL' if c < args.min_change else 'ok  '} {c:5.2f} {n}")
    print(f"{len(results) - len(bad)} move, {len(bad)} still (motion={args.motion})")
    st["exit"] = 1 if bad or not results else 0
    Gtk.main_quit()
    return False


def loaded(_v, ev):
    if ev == WebKit2.LoadEvent.FINISHED:
        GLib.timeout_add(2500, begin)


q = f"?motion={args.motion}" + (f"&only={args.only}" if args.only else "")
view.connect("load-changed", loaded)
view.load_uri(args.url + q)
GLib.timeout_add(600000, Gtk.main_quit)
st["exit"] = 2
Gtk.main()
sys.exit(st["exit"])
