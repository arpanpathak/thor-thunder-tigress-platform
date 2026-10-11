"""The drawing kit for the book's system diagrams.

One visual language for every figure: paper cards with a soft shadow and a
coloured edge, cylinders for stores, sheets for text, and wires that only run
horizontally and vertically. A figure is a list of parts handed to `save`.

Two rules are checked while drawing, so a figure that breaks one does not get
written: text has to fit the box it is in, and a wire may not run at an angle.

Brand marks in `icons/` are from Simple Icons (CC0). The trademarks are their
owners'; they are used here only to say which product a box stands for.
"""
import os
import re
from html import escape
from pathlib import Path

ICONS = Path(__file__).resolve().parent / "icons"

PAPER = "#FBFAF6"
CARD = "#FFFFFF"
INK = "#1B2430"
MUTED = "#5D6B7A"
EDGE = "#D5DBE3"
WIRE = "#3F5873"

# kind: (accent, tint)
KINDS = {
    "client": ("#2F6FDB", "#EAF1FD"),
    "service": ("#1F8A70", "#E4F4EF"),
    "model": ("#5E9400", "#EEF6DC"),
    "store": ("#B7791F", "#FBF1DC"),
    "outside": ("#7A4FD0", "#F0EAFB"),
    "danger": ("#CF4526", "#FCEBE6"),
    "idle": ("#7F8EA0", "#F1F4F7"),
    "good": ("#2E8B3D", "#E7F5E9"),
}

BRAND = {
    "nvidia": "#76B900", "tailscale": "#242424", "rust": "#A8431A", "huggingface": "#E8A100",
    "python": "#3776AB", "firefoxbrowser": "#E66000", "github": "#181717", "anthropic": "#191919",
    "claude": "#D97757", "openai": "#412991", "linux": "#1B2430", "sqlite": "#003B57",
    "searxng": "#3050FF", "ollama": "#1B2430", "googlechrome": "#4285F4", "gnubash": "#3E474A",
    "git": "#F03C2E", "letsencrypt": "#003A70", "namecheap": "#DE3723", "alibabacloud": "#FF6A00",
}

# Glyphs drawn for this book, on the same 24 x 24 grid as the brand marks.
GLYPHS = {
    "terminal": "M3 4h18a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zm3.3 4.3-1.4 1.4L7.2 12l-2.3 2.3 1.4 1.4L10 12zM11 14v2h7v-2z",
    "browser": "M3 4h18a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zm1 5v9h16V9zm1.5-3a1 1 0 1 0 0 2 1 1 0 0 0 0-2zm3 0a1 1 0 1 0 0 2 1 1 0 0 0 0-2zm3 0a1 1 0 1 0 0 2 1 1 0 0 0 0-2z",
    "gear": "M10.3 2h3.4l.5 2.6 1.7.7 2.2-1.5 2.4 2.4-1.5 2.2.7 1.7 2.6.5v3.4l-2.6.5-.7 1.7 1.5 2.2-2.4 2.4-2.2-1.5-1.7.7-.5 2.6h-3.4l-.5-2.6-1.7-.7-2.2 1.5-2.4-2.4 1.5-2.2-.7-1.7L2 13.7v-3.4l2.6-.5.7-1.7-1.5-2.2 2.4-2.4 2.2 1.5 1.7-.7zM12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z",
    "chip": "M8 2h2v3h4V2h2v3h2a1 1 0 0 1 1 1v2h3v2h-3v4h3v2h-3v2a1 1 0 0 1-1 1h-2v3h-2v-3h-4v3H8v-3H6a1 1 0 0 1-1-1v-2H2v-2h3v-4H2V8h3V6a1 1 0 0 1 1-1h2zM7 7v10h10V7zm2.5 2.5h5v5h-5z",
    "doc": "M6 2h8l5 5v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1zm7 1.5V8h4.5zM8 11v1.6h8V11zm0 3.2v1.6h8v-1.6zm0 3.2V19h5v-1.6z",
    "key": "M14.5 2a7.5 7.5 0 0 0-7.2 9.6L2 16.9V22h5.1v-2.5h2.5V17h2.4l1-1a7.5 7.5 0 1 0 1.5-14zm2.2 4.3a2 2 0 1 1 0 4 2 2 0 0 1 0-4z",
    "search": "M10.5 3a7.5 7.5 0 0 1 5.9 12.1l4.7 4.8-1.8 1.8-4.8-4.7A7.5 7.5 0 1 1 10.5 3zm0 2.5a5 5 0 1 0 0 10 5 5 0 0 0 0-10z",
    "clock": "M12 2a10 10 0 1 1 0 20 10 10 0 0 1 0-20zm0 2.4a7.6 7.6 0 1 0 0 15.2 7.6 7.6 0 0 0 0-15.2zM11 7h2v4.6l3.3 2-1 1.7-4.3-2.6z",
    "stack": "M12 2 2 7l10 5 10-5zm-7.8 8.6L2 11.7l10 5 10-5-2.2-1.1L12 14.5zm0 4.7L2 16.4l10 5 10-5-2.2-1.1L12 19.2z",
    "swap": "M7 4 2 9l5 5v-3.5h9V8H7zm10 6v3.5H8V16h9v3.5l5-5z",
    "mask": "M4 3h16v9a8 8 0 0 1-16 0zM7.5 7h3v2.2h-3zM13.5 7h3v2.2h-3zM8.2 12.6h7.6a3.8 3.8 0 0 1-7.6 0z",
    "listen": "M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-8.5L6 21.5V17H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM12 14.3l-3.4-3.2a2.1 2.1 0 0 1 3.4-2.4 2.1 2.1 0 0 1 3.4 2.4z",
    "passport": "M6 2h12a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1zM12 6a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 8.2a1.8 1.8 0 1 1 0 3.6 1.8 1.8 0 0 1 0-3.6zM8.5 16.5h7v1.6h-7z",
    "pencil": "M3 21l1.5-5.5L15.2 4.8l4 4L8.5 19.5zM16.6 3.4l1.3-1.3a1 1 0 0 1 1.4 0l2.6 2.6a1 1 0 0 1 0 1.4l-1.3 1.3z",
    "note": "M10.5 3H19v4h-6v10.5a3.5 3.5 0 1 1-2.5-3.35z",
    "curve": "M10 2h4v4h-4zM2 15h5v5H2zM17 15h5v5h-5zM3.6 15a8.4 8.4 0 0 1 16.8 0h-1.8a6.6 6.6 0 0 0-13.2 0z",
    "laptop": "M5 4h14a1 1 0 0 1 1 1v10H4V5a1 1 0 0 1 1-1zm1 2v7h12V6zM2 16.5h20V18a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z",
    "lock": "M7 10V7a5 5 0 0 1 10 0v3h1a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1zm2.2 0h5.6V7a2.8 2.8 0 0 0-5.6 0zM12 13.5a1.6 1.6 0 0 0-.8 3v2h1.6v-2a1.6 1.6 0 0 0-.8-3z",
    "eye": "M12 5c5 0 8.5 3.5 10 7-1.5 3.5-5 7-10 7S3.5 15.5 2 12c1.5-3.5 5-7 10-7zm0 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8zm0 2.2a1.8 1.8 0 1 1 0 3.6 1.8 1.8 0 0 1 0-3.6z",
    "cup": "M4 8h13v2h1.5a3.5 3.5 0 0 1 0 7H17a5 5 0 0 1-5 4H9a5 5 0 0 1-5-5zm13 4v3h1.5a1.5 1.5 0 0 0 0-3zM7 2h1.6v4H7zm3.2 0h1.6v4h-1.6zm3.2 0H15v4h-1.6z",
}

# A person's look: (skin, hair colour, hair style, top colour, extra).
PEOPLE = {
    "long": ("#C68B5E", "#2B1D16", "long", "#1F8A70", None),
    "beard": ("#F0C8A0", "#3A2A1E", "short", "#2F6FDB", "beard"),
    "curly": ("#8A5A3C", "#1B1512", "curly", "#D9A21B", None),
    "glasses": ("#F4D3B5", "#B5532A", "short", "#5E9400", "glasses"),
    "bun": ("#E0AC84", "#5A3A22", "bun", "#7A4FD0", None),
    "cap": ("#A9744B", "#1B1512", "short", "#CF4526", "cap"),
}

STYLE = f"""  <style>
    text {{ font-family: Inter, "Segoe UI", "Helvetica Neue", Helvetica, Arial, sans-serif; fill: {INK}; }}
    .title {{ font-size: 18px; font-weight: 650; }}
    .lede {{ font-size: 12.5px; fill: {MUTED}; }}
    .name {{ font-size: 14px; font-weight: 650; }}
    .sub {{ font-size: 12px; fill: {MUTED}; }}
    .note {{ font-size: 12px; fill: {MUTED}; }}
    .tab {{ font-size: 11.5px; font-weight: 650; letter-spacing: 0.04em; fill: {MUTED}; }}
    .mono {{ font-family: "JetBrains Mono", "SF Mono", Menlo, Consolas, monospace; font-size: 12px; }}
    .dim {{ fill: #8794A3; }}
    .strong {{ font-weight: 700; }}
    .trap {{ fill: {KINDS['danger'][0]}; font-weight: 700; }}
    .pill {{ font-size: 11px; font-weight: 600; }}
    .wire {{ fill: none; stroke: {WIRE}; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }}
    .wire.danger {{ stroke: {KINDS['danger'][0]}; stroke-dasharray: 6 5; }}
    .wire.idle {{ stroke: {KINDS['idle'][0]}; stroke-dasharray: 2 5; }}
  </style>
  <defs>
    <filter id="lift" x="-10%" y="-10%" width="120%" height="135%">
      <feDropShadow dx="0" dy="2" stdDeviation="3" flood-color="#1B2430" flood-opacity="0.13"/>
    </filter>
    <marker id="head" viewBox="0 0 10 10" refX="8.5" refY="5" markerUnits="userSpaceOnUse" markerWidth="11" markerHeight="11" orient="auto">
      <path d="M0.5,1 L9,5 L0.5,9 Z" fill="{WIRE}"/>
    </marker>
    <marker id="head-danger" viewBox="0 0 10 10" refX="8.5" refY="5" markerUnits="userSpaceOnUse" markerWidth="11" markerHeight="11" orient="auto">
      <path d="M0.5,1 L9,5 L0.5,9 Z" fill="{KINDS['danger'][0]}"/>
    </marker>
    <marker id="head-idle" viewBox="0 0 10 10" refX="8.5" refY="5" markerUnits="userSpaceOnUse" markerWidth="11" markerHeight="11" orient="auto">
      <path d="M0.5,1 L9,5 L0.5,9 Z" fill="{KINDS['idle'][0]}"/>
    </marker>
  </defs>"""


class DoesNotFit(Exception):
    """Text that is wider than the room it was given, or a wire at an angle."""


def width_of(body, size=12.0, mono=False, bold=False):
    """A cautious estimate of how wide `body` renders, in pixels."""
    per_char = 0.61 if mono else (0.6 if bold else 0.55)
    return len(body) * size * per_char


def must_fit(body, room, size=12.0, mono=False, bold=False):
    """Raises when `body` is wider than `room`. With FIGKIT_REPORT set it prints instead,
    so one run lists every label that needs more room."""
    if width_of(body, size, mono, bold) <= room:
        return
    problem = f"{body!r} needs {width_of(body, size, mono, bold):.0f} px, has {room:.0f}"
    if not os.environ.get("FIGKIT_REPORT"):
        raise DoesNotFit(problem)
    print("does not fit:", problem)


def text(x, y, body, cls="note", anchor="start", fill=None):
    colour = f' style="fill:{fill}"' if fill else ""
    return (f'  <text x="{x:.1f}" y="{y:.1f}" class="{cls}" text-anchor="{anchor}"{colour}'
            f' xml:space="preserve">{escape(body)}</text>')


def icon(name, x, y, size=20, fill=None):
    """A brand mark from icons/, or one of the kit's own glyphs, drawn `size` wide."""
    if name in GLYPHS:
        path, colour = GLYPHS[name], fill or INK
    else:
        source = (ICONS / f"{name}.svg").read_text()
        path, colour = re.search(r'<path d="([^"]+)"', source).group(1), fill or BRAND.get(name, INK)
    return (f'  <g transform="translate({x:.1f},{y:.1f}) scale({size / 24:.4f})">'
            f'<path d="{path}" fill="{colour}" fill-rule="evenodd"/></g>')


def person(x, y, look, size=56):
    """A friendly head-and-shoulders portrait in a round badge, `size` wide, with its
    top-left corner at (x, y). `look` is a key of PEOPLE."""
    skin, hair, style, top, extra = PEOPLE[look]
    clip = f"person-{look}-{int(x)}-{int(y)}"
    fringe = "M21.5 26 C20 15 26 12 32 12 C38 12 44 15 42.5 26 C41 20.5 38 18.5 32 18.5 C26 18.5 23 20.5 21.5 26 Z"
    parts = [
        f'  <g transform="translate({x:.1f},{y:.1f}) scale({size / 64:.4f})">',
        f'<clipPath id="{clip}"><circle cx="32" cy="32" r="30"/></clipPath>',
        f'<circle cx="32" cy="32" r="30" fill="#FFFFFF" stroke="{EDGE}" stroke-width="1.6" filter="url(#lift)"/>',
        f'<g clip-path="url(#{clip})">',
        f'<circle cx="32" cy="32" r="30" fill="{top}" opacity="0.14"/>',
    ]
    if style == "long":
        parts.append(f'<path d="M18 50 C15 28 19 11 32 11 C45 11 49 28 46 50 C42 50 41 44 41 36 L23 36 C23 44 22 50 18 50 Z" fill="{hair}"/>')
    parts += [
        f'<path d="M8 66 C8 49 19 43 32 43 C45 43 56 49 56 66 Z" fill="{top}"/>',
        f'<path d="M27.5 34 h9 v8.5 a4.5 4.5 0 0 1 -9 0 Z" fill="{skin}"/>',
        f'<path d="M27.5 37.5 a8 8 0 0 0 9 0 v3 a6 6 0 0 1 -9 0 Z" fill="{INK}" opacity="0.13"/>',
        f'<ellipse cx="32" cy="26.5" rx="10.2" ry="11.2" fill="{skin}"/>',
    ]
    if extra == "beard":
        parts.append(f'<path d="M22 27 C22 38.5 27.5 40 32 40 C36.5 40 42 38.5 42 27 C40 32.5 36.5 33.2 32 33.2 C27.5 33.2 24 32.5 22 27 Z" fill="{hair}"/>')
    if style == "curly":
        parts += [f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{hair}"/>'
                  for cx, cy, r in ((21.5, 22, 5.2), (24.5, 16, 5.6), (30, 12.5, 5.8), (36, 13, 5.6), (41, 17, 5.4), (43, 23, 4.8))]
    else:
        parts.append(f'<path d="{fringe}" fill="{hair}"/>')
    if style == "bun":
        parts.append(f'<circle cx="32" cy="9.5" r="5.2" fill="{hair}"/>')
    if extra == "cap":
        parts.append(f'<path d="M21 21 C21 12 26 9.5 32 9.5 C38 9.5 43 12 43 21 Z M43 18.5 h7.5 a1.6 1.6 0 0 1 0 3.2 H42 Z" fill="{top}"/>')
    parts += [
        f'<circle cx="28" cy="27" r="1.25" fill="{INK}"/><circle cx="36" cy="27" r="1.25" fill="{INK}"/>',
        f'<path d="M28.3 31.4 Q32 34.6 35.7 31.4" fill="none" stroke="{"#FFFFFF" if extra == "beard" else INK}" stroke-width="1.3" stroke-linecap="round"/>',
    ]
    if extra == "glasses":
        parts.append(f'<path d="M24.6 27 a3.4 3.4 0 1 0 6.8 0 a3.4 3.4 0 1 0 -6.8 0 M32.6 27 a3.4 3.4 0 1 0 6.8 0 a3.4 3.4 0 1 0 -6.8 0 M31.4 26.6 h1.2"'
                     f' fill="none" stroke="{INK}" stroke-width="1.1"/>')
    parts += ["</g>", "</g>"]
    return "\n".join(parts)


def card(x, y, w, h, kind, name, sub=(), glyph=None, glyph_fill=None, dashed=False, pill=None, tile=None, big=False):
    """A paper card: a coloured edge on the left, an icon tile, a name and up to three lines.

    `tile` is the colour behind the icon; the card's tint when it is not given. `big` draws
    the icon half as large again, for a card whose picture is the point."""
    accent, tint = KINDS[kind]
    lines = [sub] if isinstance(sub, str) else list(sub)
    side = 48 if big else 32
    left = x + (26 + side if glyph else 20)
    room = x + w - left - 12 - (width_of(pill, 11, bold=True) + 22 if pill else 0)
    must_fit(name, room, 14, bold=True)
    for line in lines:
        must_fit(line, x + w - left - 12)

    outline = f'stroke="{accent}" stroke-dasharray="6 5"' if dashed else f'stroke="{EDGE}"'
    shadow = "" if dashed else ' filter="url(#lift)"'
    fill = tint if dashed else CARD
    parts = [
        f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="{fill}" {outline} stroke-width="1.3"{shadow}/>',
        f'  <path d="M{x + 12},{y + 0.7} H{x + 6} V{y + h - 0.7} H{x + 12} A11.3,11.3 0 0 1 {x + 0.7},{y + h - 12}'
        f' V{y + 12} A11.3,11.3 0 0 1 {x + 12},{y + 0.7} Z" fill="{accent}"/>',
    ]
    if glyph:
        mark = side - 12 if not big else 30
        parts += [f'  <rect x="{x + 16}" y="{y + h / 2 - side / 2}" width="{side}" height="{side}" rx="{side / 4:.0f}" fill="{tile or tint}"/>',
                  icon(glyph, x + 16 + (side - mark) / 2, y + h / 2 - mark / 2, mark,
                       glyph_fill or (None if glyph not in GLYPHS else accent))]
    top = y + h / 2 - 9 * len(lines) + 5
    parts.append(text(left, top, name, "name"))
    parts += [text(left, top + 18 * (row + 1), line, "sub") for row, line in enumerate(lines)]
    if pill:
        parts.append(tag(x + w - 12, y + 12, pill, kind, anchor="end"))
    return "\n".join(parts)


def tag(x, y, body, kind="idle", anchor="start"):
    """A small rounded label, such as a port number."""
    accent, tint = KINDS[kind]
    wide = width_of(body, 11, bold=True) + 14
    left = x - wide if anchor == "end" else x
    return (f'  <rect x="{left:.1f}" y="{y}" width="{wide:.1f}" height="18" rx="9" fill="{tint}"/>\n'
            + text(left + wide / 2, y + 12.6, body, "pill", "middle", accent))


def cylinder(x, y, w, h, kind, name, sub=()):
    """A store: a database, a file on disk, a model's weights."""
    accent, tint = KINDS[kind]
    lines = [sub] if isinstance(sub, str) else list(sub)
    must_fit(name, w - 20, 14, bold=True)
    for line in lines:
        must_fit(line, w - 16)
    ry, cx = 9, x + w / 2
    body = (f'M{x},{y + ry} A{w / 2},{ry} 0 0 1 {x + w},{y + ry} V{y + h - ry}'
            f' A{w / 2},{ry} 0 0 1 {x},{y + h - ry} Z')
    top = y + 2 * ry + (h - 2 * ry - 4) / 2 - 9 * len(lines) + 5
    parts = [
        f'  <path d="{body}" fill="{tint}" stroke="{accent}" stroke-width="1.4" filter="url(#lift)"/>',
        f'  <ellipse cx="{cx}" cy="{y + ry}" rx="{w / 2}" ry="{ry}" fill="{CARD}" stroke="{accent}" stroke-width="1.4"/>',
        text(cx, top, name, "name", "middle"),
    ]
    parts += [text(cx, top + 18 * (row + 1), line, "sub", "middle") for row, line in enumerate(lines)]
    return "\n".join(parts)


def sheet(x, y, w, h, kind, heading, lines, footer=()):
    """A sheet of paper with a folded corner: a heading, lines of code, then plain lines.

    `lines` are (class, text) pairs set in the code face; `footer` are plain strings."""
    accent, tint = KINDS[kind]
    fold = 18
    must_fit(heading, w - 36 - fold, 12, bold=True)
    for _, body in lines:
        must_fit(body, w - 32, 12, mono=True)
    for body in footer:
        must_fit(body, w - 32)
    outline = (f'M{x + 10},{y} H{x + w - fold} L{x + w},{y + fold} V{y + h - 10} A10,10 0 0 1 {x + w - 10},{y + h}'
               f' H{x + 10} A10,10 0 0 1 {x},{y + h - 10} V{y + 10} A10,10 0 0 1 {x + 10},{y} Z')
    parts = [
        f'  <path d="{outline}" fill="{CARD}" stroke="{EDGE}" stroke-width="1.3" filter="url(#lift)"/>',
        f'  <path d="M{x + w - fold},{y} V{y + fold} H{x + w} Z" fill="{tint}" stroke="{EDGE}" stroke-width="1.3" stroke-linejoin="round"/>',
        f'  <rect x="{x + 16}" y="{y + 16}" width="4" height="14" rx="2" fill="{accent}"/>',
        text(x + 28, y + 27.5, heading, "tab", fill=accent),
    ]
    parts += [text(x + 16, y + 56 + 18 * row, body, f"mono {cls}".strip()) for row, (cls, body) in enumerate(lines)]
    base = y + h - 16 - 17 * (len(footer) - 1)
    parts += [text(x + 16, base + 17 * row, body, "note", fill=accent) for row, body in enumerate(footer)]
    return "\n".join(parts)


def zone(x, y, w, h, label):
    """A tinted area that groups cards, with its label on a tab."""
    must_fit(label, w - 40, 11.5, bold=True)
    wide = width_of(label.upper(), 11.5, bold=True) * 1.14 + 28
    return "\n".join([
        f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="16" fill="#F3F1EA" stroke="#E4E0D4" stroke-width="1.2"/>',
        f'  <rect x="{x + 18}" y="{y - 11}" width="{wide:.1f}" height="22" rx="11" fill="{INK}"/>',
        text(x + 18 + wide / 2, y + 4.2, label.upper(), "tab", "middle", "#FFFFFF"),
    ])


def wire(points, kind="flow", label=None, on=None, arrow=True):
    """A connection through `points`. Every segment is horizontal or vertical; corners are rounded.

    `label` sits on segment `on` (the longest by default), on a patch of paper that hides the line under it."""
    for (ax, ay), (bx, by) in zip(points, points[1:]):
        if ax != bx and ay != by:
            raise DoesNotFit(f"wire runs at an angle from {(ax, ay)} to {(bx, by)}")
    radius, path = 9, [f"M{points[0][0]},{points[0][1]}"]
    for before, corner, after in zip(points, points[1:], points[2:]):
        path.append("L{},{} Q{},{} {},{}".format(*toward(corner, before, radius), *corner, *toward(corner, after, radius)))
    path.append(f"L{points[-1][0]},{points[-1][1]}")
    cls = "wire" if kind == "flow" else f"wire {kind}"
    marker = {"flow": "head", "danger": "head-danger", "idle": "head-idle"}[kind]
    end = f' marker-end="url(#{marker})"' if arrow else ""
    parts = [f'  <path class="{cls}" d="{" ".join(path)}"{end}/>']
    if label:
        segments = list(zip(points, points[1:]))
        (ax, ay), (bx, by) = segments[on] if on is not None else max(segments, key=lambda s: abs(s[0][0] - s[1][0]) + abs(s[0][1] - s[1][1]))
        if abs(ax - bx) + abs(ay - by) < width_of(label, 11, bold=True) + 34 and ay == by:
            raise DoesNotFit(f"label {label!r} is longer than the segment it sits on")
        colour = {"flow": WIRE, "danger": KINDS["danger"][0], "idle": KINDS["idle"][0]}[kind]
        wide, cx, cy = width_of(label, 11, bold=True) + 14, (ax + bx) / 2, (ay + by) / 2
        parts += [f'  <rect x="{cx - wide / 2:.1f}" y="{cy - 9}" width="{wide:.1f}" height="18" rx="9" fill="{PAPER}" stroke="{colour}" stroke-width="1"/>',
                  text(cx, cy + 3.8, label, "pill", "middle", colour)]
    return "\n".join(parts)


def toward(corner, other, distance):
    """The point `distance` from `corner` on the way to `other`, or halfway when they are closer than that."""
    (cx, cy), (ox, oy) = corner, other
    reach = min(distance, (abs(ox - cx) + abs(oy - cy)) / 2)
    step = lambda a, b: a + reach * ((b > a) - (b < a))
    return step(cx, ox), step(cy, oy)


def legend(x, y, items):
    """A row of (kind, text) pairs, each with a swatch."""
    parts, at = [], x
    for kind, body in items:
        accent, tint = KINDS[kind]
        parts += [f'  <rect x="{at}" y="{y - 10}" width="14" height="14" rx="4" fill="{tint}" stroke="{accent}" stroke-width="1.3"/>',
                  text(at + 21, y + 1.5, body, "note")]
        at += 21 + width_of(body) + 22
    return "\n".join(parts)


def save(path, width, height, label, title, lede, parts):
    """Writes the figure. `label` is read aloud by a screen reader in place of the picture."""
    must_fit(title, width - 64, 18, bold=True)
    must_fit(lede, width - 64, 12.5)
    body = "\n".join(parts)
    Path(path).write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}"'
        f' role="img" aria-label="{escape(label)}">\n{STYLE}\n'
        f'  <rect width="{width}" height="{height}" rx="14" fill="{PAPER}"/>\n'
        f'{text(32, 42, title, "title")}\n{text(32, 63, lede, "lede")}\n{body}\n</svg>\n')


def stacked(x, y, w, h, segments, total=None):
    """A horizontal bar of (value, colour, label) segments. A label is drawn inside its
    segment when it fits and left out when it does not, so a caller lists small ones in a legend."""
    total = total or sum(value for value, _, _ in segments)
    parts, at = [], x
    for value, colour, label in segments:
        wide = w * value / total
        parts.append(f'  <rect x="{at:.1f}" y="{y}" width="{wide:.1f}" height="{h}" fill="{colour}"/>')
        if label and width_of(label, 12, bold=True) + 16 <= wide:
            parts.append(text(at + wide / 2, y + h / 2 + 4.2, label, "pill", "middle", "#FFFFFF"))
        at += wide
    frame = f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" fill="none" stroke="{EDGE}" stroke-width="1.2"/>'
    clip = f"clip-{int(x)}-{int(y)}"
    return "\n".join([f'  <clipPath id="{clip}"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6"/></clipPath>',
                      f'  <g clip-path="url(#{clip})" filter="url(#lift)">'] + parts + ["  </g>", frame])


def swatches(x, y, items):
    """A row of (colour, text) pairs."""
    parts, at = [], x
    for colour, body in items:
        parts += [f'  <rect x="{at:.1f}" y="{y - 10}" width="14" height="14" rx="4" fill="{colour}"/>',
                  text(at + 21, y + 1.5, body, "note")]
        at += 21 + width_of(body) + 24
    return "\n".join(parts)
