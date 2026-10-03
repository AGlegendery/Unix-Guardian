#!/usr/bin/env python3
"""Generates the G-gauge tray icons.

Writes assets/branding/tray/ug-tray-<state>[-16].svg (installed into the icon
theme and loaded by name, so the plain parts follow the panel's text colour)
and crates/ug-agent/assets/tray-<state>-<size>.argb, the pixmaps the agent
falls back to when the icons are not installed. Needs ImageMagick and Pillow.
"""
import io
import math
import pathlib
import subprocess

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
SVG_DIR = ROOT / "assets/branding/tray"
ARGB_DIR = ROOT / "crates/ug-agent/assets"
TEXT = 'class="ColorScheme-Text" stroke="currentColor"'
TEXT_FILL = 'class="ColorScheme-Text" fill="currentColor"'
# The pixmaps cannot follow the theme; a light grey reads on dark and mid panels.
PIXMAP_TEXT = "#dcdcdc"

# state: (arc, needle, angle, opacity, hollow hub)
STATES = {
    "normal": ("text", "text", 0, 1, False),
    "warning": ("text", "#f5a524", -20, 1, False),
    "critical": ("#ef4444", "#ef4444", -32, 1, False),
    "gamemode": ("#9b6bff", "#9b6bff", 0, 1, False),
    "offline": ("text", "text", 90, 0.45, True),
}


def svg(state, size, pixmap=False):
    arc, needle, ang, op, hollow = STATES[state]
    sw, r = (7, 6) if size != 16 else (8.5, 6.5)

    def stroke(c):
        if c == "text":
            return f'stroke="{PIXMAP_TEXT}"' if pixmap else TEXT
        return f'stroke="{c}"'

    def fill(c):
        if c == "text":
            return f'fill="{PIXMAP_TEXT}"' if pixmap else TEXT_FILL
        return f'fill="{c}"'

    if hollow:
        hub = f'<circle cx="50" cy="52" r="{r - 1.5}" fill="none" stroke-width="3" {stroke(needle)}/>'
    else:
        hub = f'<circle cx="50" cy="52" r="{r}" stroke="none" {fill(needle)}/>'
    style = "" if pixmap else '<style type="text/css" id="current-color-scheme">.ColorScheme-Text { color:#232629; }</style>'
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="16 18 68 68">{style}'
        f'<g fill="none" stroke-linecap="round" stroke-width="{sw}" opacity="{op}">'
        f'<path d="M69.8 32.2 A28 28 0 1 0 78 52" {stroke(arc)}/>'
        f'<path d="{needle_path(ang)}" {stroke(needle)}/>{hub}</g></svg>\n'
    )


def needle_path(ang):
    """The needle "M53 52 H78" turned by `ang` degrees (SVG sense) around the
    hub, written out rather than as a transform: some renderers ignore it."""
    a = math.radians(ang)
    pt = lambda x: (round(50 + (x - 50) * math.cos(a), 2), round(52 + (x - 50) * math.sin(a), 2))
    (x0, y0), (x1, y1) = pt(53), pt(78)
    return f"M{x0:g} {y0:g} L{x1:g} {y1:g}"


def argb(svg_text, size):
    # Render large and scale down: ImageMagick's own SVG renderer antialiases poorly.
    png = subprocess.run(
        ["convert", "-background", "none", "-density", "1200", "svg:-", "png:-"],
        input=svg_text.encode(), capture_output=True, check=True,
    ).stdout
    im = Image.open(io.BytesIO(png)).convert("RGBA").resize((size, size), Image.LANCZOS)
    return b"".join(bytes((a, r, g, b)) for r, g, b, a in im.getdata())


SVG_DIR.mkdir(parents=True, exist_ok=True)
for old in ARGB_DIR.glob("tray-*.argb"):
    old.unlink()
for state in STATES:
    (SVG_DIR / f"ug-tray-{state}.svg").write_text(svg(state, 22))
    (SVG_DIR / f"ug-tray-{state}-16.svg").write_text(svg(state, 16))
    for size in (22, 32, 48, 64):
        (ARGB_DIR / f"tray-{state}-{size}.argb").write_bytes(argb(svg(state, size, pixmap=True), size))
print("ok")
