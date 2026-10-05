"""Draws the book's logo, cover and chapter plates.

    python3 book/tools/draw_art.py

Writes free-form SVG files to book/src/art/ and renders cover.png, logo.png and
logo.gif with cairosvg and Pillow. No picture carries an opaque background, so
each one can sit on any page colour. Every picture is built from one full-body
Thor tigress, a Bengal tigress in a winged silver helm and storm cape, holding
the tool of her crate:

    Hammer  thor-hammer-trainer     data generation and training
    Lasso   thor-lasso-distiller    distillation from a teacher model
    Spark   thor-spark-safety-eval  evaluation and safety

A pale halo is drawn behind the art so the dark outlines stay readable on the
book's dark theme.
"""

import io
import re
from pathlib import Path
from xml.sax.saxutils import escape

import cairosvg
from PIL import Image

ART = Path(__file__).resolve().parent.parent / "src" / "art"

HALO = "#F7FAFF"
INK = "#141C28"
FUR = "#9AA9C4"
FUR_DARK = "#4A5A7A"
FUR_LIGHT = "#E2E9F5"
CREAM = "#F1F5FC"
STRIPE = "#37476A"
NOSE = "#C79AA0"
EYE = "#F0C46A"
SILVER = "#D9E1EC"
SILVER_LIGHT = "#F2F6FB"
SILVER_DARK = "#8A98AE"
STEEL_DARK = "#6A7A94"
CLOTH = "#3D5480"
CLOTH_DARK = "#2A3B5C"
ACCENT = "#6FD8F5"
ACCENT_DARK = "#3AA6CC"
WOOD = "#7A5A3C"
ROPE = "#C2A672"
BLUE = "#5A82C8"
GREEN = "#7FA96B"
RED = "#B0605C"
AMBER = "#E0B45C"


def mirror(path: str, center: float = 100.0) -> str:
    """Mirrors an absolute path of M/C/L/Z commands across x = center."""
    tokens = re.findall(r"[A-Za-z]|-?\d*\.?\d+", path)
    mirrored = []
    expect_x = True
    for token in tokens:
        if token.isalpha():
            mirrored.append(token)
            expect_x = True
            continue
        value = float(token)
        mirrored.append(f"{2 * center - value:g}" if expect_x else f"{value:g}")
        expect_x = not expect_x
    return " ".join(mirrored)


def pair(path: str, fill: str, extra: str = "", center: float = 100.0) -> str:
    """A shape and its mirror image."""
    return (f'<path d="{path}" fill="{fill}" {extra}/>'
            f'<path d="{mirror(path, center)}" fill="{fill}" {extra}/>')


def pair_stroke(path: str, color: str, width: float, extra: str = "", center: float = 100.0) -> str:
    """A stroked path and its mirror image."""
    style = f'stroke="{color}" stroke-width="{width}" stroke-linecap="round" {extra}'
    return (f'<path d="{path}" fill="none" {style}/>'
            f'<path d="{mirror(path, center)}" fill="none" {style}/>')


def with_halo(content: str, width: int = 10) -> str:
    """A pale silhouette behind content, so the art reads on any page colour."""

    def recolour(match: "re.Match[str]") -> str:
        attribute, value = match.group(1), match.group(2)
        if attribute == "fill" and value == "none":
            return match.group(0)
        return f'{attribute}="{HALO}"'

    silhouette = re.sub(r'(fill|stroke)="([^"]*)"', recolour, content)
    silhouette = re.sub(r'stroke-width="[^"]*"', f'stroke-width="{width}"', silhouette)
    silhouette = re.sub(r'(fill-opacity|opacity|stroke-dasharray)="[^"]*"', "", silhouette)
    return silhouette + content


def text_haloed(x: float, y: float, text: str, font: str, size: int, fill: str,
                weight: str = "400", italic: bool = False, halo_width: int = 9) -> str:
    """A line of text with a pale outline, so it reads on light and dark pages."""
    style = (f'font-family="{font}" font-size="{size}" font-weight="{weight}" '
             f'text-anchor="middle"')
    if italic:
        style += ' font-style="italic"'
    encoded = escape(text)
    outline = (f'<text x="{x}" y="{y}" {style} fill="{HALO}" stroke="{HALO}" '
               f'stroke-width="{halo_width}" stroke-linejoin="round">{encoded}</text>')
    return outline + f'<text x="{x}" y="{y}" {style} fill="{fill}">{encoded}</text>'


def tigress_head() -> str:
    """A Thor tigress head in a 200 x 200 box: winged silver helm over a tiger face."""
    outline = f'stroke="{INK}" stroke-width="3" stroke-linejoin="round"'
    face = ("M100,36 C136,36 164,58 168,92 C170,108 176,118 184,126 C175,130 169,136 167,146 "
            "C159,166 133,182 100,182 C67,182 41,166 33,146 C31,136 25,130 16,126 "
            "C24,118 30,108 32,92 C36,58 64,36 100,36 Z")
    muzzle = ("M100,108 C88,108 70,116 64,130 C60,144 72,160 100,164 "
              "C128,160 140,144 136,130 C130,116 112,108 100,108 Z")
    ear_outer = "M54,62 C46,42 50,22 68,16 C82,18 90,32 92,50 C80,53 66,58 54,62 Z"
    ear_inner = "M60,56 C55,42 57,29 69,25 C77,27 82,37 84,48 C76,51 67,54 60,56 Z"
    helm = ("M100,20 C122,20 142,34 152,56 C134,49 116,45 100,45 "
            "C84,45 66,49 48,56 C58,34 78,20 100,20 Z")
    band = "M48,58 C68,51 132,51 152,58 L148,70 C130,62 70,62 52,70 Z"
    wing = ("M46,64 C26,58 10,38 4,10 C18,20 30,32 40,48 C38,30 44,14 56,4 "
            "C58,22 56,42 50,54 C54,50 58,48 62,48 C54,56 50,62 46,64 Z")
    crest = "M100,-2 C105,6 105,14 100,20 C95,14 95,6 100,-2 Z"
    gem = "M100,55 L107,62 L100,69 L93,62 Z"
    braid = "M40,72 C34,94 34,120 42,144 C46,156 52,164 58,170"
    ticks = ["M29,92 L41,88", "M28,108 L40,104", "M32,124 L44,120", "M39,140 L51,136", "M46,156 L58,152"]
    eye = "M62,94 C69,84 81,84 88,94 C81,102 69,102 62,94 Z"
    mark = "M100,72 L95,83 L100,83 L96,93 L105,80 L100,80 L103,72 Z"
    stripes = [
        "M46,86 C54,88 59,93 62,100 C55,98 49,93 46,86 Z",
        "M34,104 C42,106 48,110 52,117 C44,115 37,110 34,104 Z",
        "M36,122 C44,124 50,128 54,135 C46,133 39,128 36,122 Z",
        "M48,140 C54,142 58,146 61,152 C55,150 50,146 48,140 Z",
    ]
    parts = [
        pair(ear_outer, FUR, outline),
        pair(ear_inner, STRIPE),
        f'<path d="{face}" fill="{FUR}" {outline}/>',
        pair_stroke(braid, FUR_DARK, 12),
        pair_stroke(braid, FUR_LIGHT, 7),
        pair(wing, SILVER, outline),
        f'<path d="{helm}" fill="{SILVER}" {outline}/>',
        f'<path d="M64,38 C80,29 120,29 136,38" fill="none" stroke="{SILVER_LIGHT}" stroke-width="4" stroke-linecap="round"/>',
        f'<path d="{band}" fill="{SILVER_DARK}" stroke="{INK}" stroke-width="2" stroke-linejoin="round"/>',
        f'<path d="{crest}" fill="{CLOTH}" stroke="{INK}" stroke-width="2" stroke-linejoin="round"/>',
        f'<path d="{gem}" fill="{ACCENT}" stroke="{INK}" stroke-width="2" stroke-linejoin="round"/>',
        f'<path d="{muzzle}" fill="{CREAM}"/>',
    ]
    parts += [pair(stripe, STRIPE) for stripe in stripes]
    parts += [pair_stroke(tick, FUR_DARK, 3) for tick in ticks]
    parts += [
        f'<circle cx="58" cy="170" r="3" fill="{SILVER}" stroke="{INK}" stroke-width="2"/>'
        f'<circle cx="142" cy="170" r="3" fill="{SILVER}" stroke="{INK}" stroke-width="2"/>',
        f'<path d="{mark}" fill="{ACCENT}"/>',
        pair(eye, EYE, f'stroke="{INK}" stroke-width="2.5"'),
        f'<ellipse cx="74" cy="93" rx="2.6" ry="6" fill="{INK}"/>'
        f'<ellipse cx="126" cy="93" rx="2.6" ry="6" fill="{INK}"/>',
        '<circle cx="67" cy="90" r="2" fill="#FFFFFF"/>'
        '<circle cx="133" cy="90" r="2" fill="#FFFFFF"/>',
        pair_stroke("M60,94 L52,89", INK, 2.5),
        f'<path d="M89,116 L111,116 C111,122 105,128 100,130 C95,128 89,122 89,116 Z" '
        f'fill="{NOSE}" stroke="{INK}" stroke-width="2"/>',
        f'<path d="M100,130 L100,138 M100,138 C96,145 88,145 83,140 M100,138 C104,145 112,145 117,140" '
        f'fill="none" stroke="{INK}" stroke-width="2.4" stroke-linecap="round"/>',
        pair_stroke("M68,122 C56,118 44,118 30,114", SILVER, 1.6),
        pair_stroke("M70,134 C58,133 46,136 32,140", SILVER, 1.6),
    ]
    for x, y in [(80, 126), (86, 131), (79, 134)]:
        parts.append(f'<circle cx="{x}" cy="{y}" r="1.6" fill="{INK}"/>'
                     f'<circle cx="{200 - x}" cy="{y}" r="1.6" fill="{INK}"/>')
    return "".join(parts)


def cape() -> str:
    """The storm cape behind the body, centred on x = 130."""
    outline = f'stroke="{INK}" stroke-width="3" stroke-linejoin="round"'
    half = ("M130,200 C92,200 62,218 50,252 C42,284 42,326 46,372 "
            "L100,354 C92,314 100,262 130,224 Z")
    lining = ("M130,212 C106,218 86,234 76,258 C70,286 70,322 72,356 "
              "L100,344 C92,310 98,266 130,228 Z")
    return pair(half, CLOTH, outline, center=130) + pair(lining, CLOTH_DARK, "", center=130)


def tail() -> str:
    """A thin striped tail curving up on the left, behind the body."""
    path = "M86,320 C52,314 26,296 26,272 C26,256 36,248 48,252"
    return (
        f'<path d="{path}" fill="none" stroke="{INK}" stroke-width="17" stroke-linecap="round"/>'
        f'<path d="{path}" fill="none" stroke="{FUR}" stroke-width="12" stroke-linecap="round"/>'
        f'<path d="{path}" fill="none" stroke="{STRIPE}" stroke-width="12" stroke-dasharray="4 10"/>'
    )


def limb(path: str, width: float = 24) -> str:
    """A fur limb drawn as a thick rounded stroke with a dark outline."""
    return (f'<path d="{path}" fill="none" stroke="{INK}" stroke-width="{width + 6}" stroke-linecap="round"/>'
            f'<path d="{path}" fill="none" stroke="{FUR}" stroke-width="{width}" stroke-linecap="round"/>')


def torso() -> str:
    """The armoured chest and belly, centred on x = 130."""
    outline = f'stroke="{INK}" stroke-width="3" stroke-linejoin="round"'
    neck = f'<path d="M116,178 L144,178 L140,206 L120,206 Z" fill="{FUR}" {outline}/>'
    body = ("M92,196 C84,214 84,236 88,262 C90,286 94,306 102,318 "
            "C116,330 144,330 158,318 C166,306 170,286 172,262 "
            "C176,236 176,214 168,196 C152,186 108,186 92,196 Z")
    plate = ("M104,218 C118,212 142,212 156,218 C162,244 162,278 156,300 "
             "C140,310 120,310 104,300 C98,278 98,244 104,218 Z")
    inset = ("M110,228 C122,222 138,222 150,228 C154,250 154,276 150,294 "
             "C138,302 122,302 110,294 C106,276 106,250 110,228 Z")
    return (
        neck
        + f'<path d="{body}" fill="{FUR}" {outline}/>'
        + f'<path d="{plate}" fill="{SILVER}" {outline}/>'
        + f'<path d="{inset}" fill="{SILVER_DARK}"/>'
        + f'<path d="M130,242 L139,253 L130,264 L121,253 Z" fill="{ACCENT}" stroke="{INK}" stroke-width="2" stroke-linejoin="round"/>'
        + f'<path d="M114,284 C122,279 138,279 146,284" fill="none" stroke="{SILVER}" stroke-width="3" stroke-linecap="round"/>'
        + f'<rect x="94" y="308" width="72" height="13" rx="6" fill="{CLOTH_DARK}" stroke="{INK}" stroke-width="2"/>'
    )


def arms() -> str:
    """One arm down at the left, one raised to the right holding the tool."""
    outline = f'stroke="{INK}" stroke-width="3" stroke-linejoin="round"'
    return (
        limb("M98,204 C82,220 70,248 70,278", 20)
        + limb("M162,204 C186,206 204,220 206,240", 20)
        + f'<ellipse cx="95" cy="204" rx="17" ry="15" fill="{SILVER}" {outline}/>'
        + f'<ellipse cx="165" cy="204" rx="17" ry="15" fill="{SILVER}" {outline}/>'
    )


def paws() -> str:
    """The two front paws, drawn over the tools so they look held."""
    outline = f'stroke="{INK}" stroke-width="3" stroke-linejoin="round"'
    return (f'<ellipse cx="70" cy="280" rx="14" ry="13" fill="{FUR}" {outline}/>'
            f'<ellipse cx="206" cy="242" rx="14" ry="13" fill="{FUR}" {outline}/>')


def legs() -> str:
    """Two fur legs with paws, centred on x = 130."""
    outline = f'stroke="{INK}" stroke-width="3" stroke-linejoin="round"'
    return (limb("M112,312 L112,392", 24) + limb("M148,312 L148,392", 24)
            + f'<ellipse cx="112" cy="398" rx="19" ry="12" fill="{FUR}" {outline}/>'
            + f'<ellipse cx="148" cy="398" rx="19" ry="12" fill="{FUR}" {outline}/>')


def hammer() -> str:
    """Mjolnir: a silver war hammer with the handle pointing down."""
    return (
        f'<rect x="-7" y="0" width="14" height="94" rx="6" fill="{WOOD}" stroke="{INK}" stroke-width="2.5"/>'
        f'<rect x="-9" y="62" width="18" height="12" rx="3" fill="{STEEL_DARK}" stroke="{INK}" stroke-width="2"/>'
        f'<rect x="-40" y="-28" width="80" height="40" rx="6" fill="{SILVER}" stroke="{INK}" stroke-width="3"/>'
        f'<rect x="-40" y="-28" width="80" height="12" rx="5" fill="{SILVER_LIGHT}"/>'
        f'<rect x="-40" y="4" width="80" height="8" rx="3" fill="{SILVER_DARK}"/>'
        f'<path d="M-9,-8 L1,-8 L-4,2 L4,2 L-9,17 L-4,5 L-11,5 Z" fill="{ACCENT}" stroke="{ACCENT_DARK}" stroke-width="1.2"/>'
    )


def lasso() -> str:
    """A coiled rope with a running loop."""
    return (
        f'<ellipse cx="0" cy="0" rx="46" ry="18" fill="none" stroke="{INK}" stroke-width="9"/>'
        f'<ellipse cx="0" cy="0" rx="46" ry="18" fill="none" stroke="{ROPE}" stroke-width="6" stroke-dasharray="7 3"/>'
        f'<path d="M38,10 C60,40 30,70 50,100" fill="none" stroke="{INK}" stroke-width="9" stroke-linecap="round"/>'
        f'<path d="M38,10 C60,40 30,70 50,100" fill="none" stroke="{ROPE}" stroke-width="6" stroke-linecap="round" stroke-dasharray="7 3"/>'
    )


def bolt(scale: float = 1.0, glow: float = 1.0) -> str:
    """A lightning bolt, top centred on the origin."""
    return (
        f'<g transform="scale({scale})" opacity="{glow:.2f}">'
        f'<path d="M14,0 L-16,46 L2,46 L-12,92 L30,32 L10,32 L26,0 Z" fill="{ACCENT}" '
        f'stroke="{ACCENT_DARK}" stroke-width="3" stroke-linejoin="round"/></g>'
    )


def magnifier() -> str:
    """A reading glass with a silver rim."""
    return (
        f'<rect x="20" y="20" width="12" height="52" rx="5" transform="rotate(-45 26 46)" fill="{WOOD}" stroke="{INK}" stroke-width="2.5"/>'
        f'<circle cx="0" cy="0" r="30" fill="#E7F1FE" fill-opacity="0.85" stroke="{SILVER_DARK}" stroke-width="7"/>'
        f'<circle cx="0" cy="0" r="30" fill="none" stroke="{INK}" stroke-width="2.5"/>'
        f'<path d="M-14,-12 C-8,-20 4,-22 12,-16" fill="none" stroke="#FFFFFF" stroke-width="4" stroke-linecap="round"/>'
    )


def quill() -> str:
    """A quill pen."""
    return (
        f'<path d="M0,0 C30,-40 70,-70 96,-80 C86,-50 50,-14 8,8 Z" fill="{CREAM}" stroke="{INK}" stroke-width="2.5"/>'
        f'<path d="M4,4 C36,-28 66,-56 92,-76" fill="none" stroke="{SILVER_DARK}" stroke-width="2"/>'
        f'<path d="M0,0 L-10,14 L4,6 Z" fill="{INK}"/>'
    )


def scales() -> str:
    """A balance with an A pan and a B pan, carried on a silver frame."""
    return (
        f'<rect x="-4" y="-60" width="8" height="110" fill="{SILVER_DARK}"/>'
        f'<rect x="-30" y="50" width="60" height="10" rx="3" fill="{SILVER_DARK}"/>'
        f'<path d="M-70,-52 L70,-52" stroke="{SILVER_DARK}" stroke-width="6" stroke-linecap="round"/>'
        f'<circle cx="0" cy="-60" r="8" fill="{SILVER}" stroke="{INK}" stroke-width="2"/>'
        + "".join(
            f'<path d="M{x - 26},-10 L{x},-50 L{x + 26},-10" fill="none" stroke="{INK}" stroke-width="1.6"/>'
            f'<path d="M{x - 30},-10 C{x - 30},10 {x + 30},10 {x + 30},-10 Z" fill="{SILVER}" stroke="{INK}" stroke-width="2.5"/>'
            f'<text x="{x}" y="4" font-family="Georgia, serif" font-size="18" font-weight="700" text-anchor="middle" fill="{INK}">{label}</text>'
            for x, label in [(-70, "A"), (70, "B")]
        )
    )


def gauge() -> str:
    """A dial with a needle in the upper range."""
    return (
        f'<path d="M-60,0 A60,60 0 0 1 60,0 Z" fill="#FFFFFF" stroke="{INK}" stroke-width="3"/>'
        f'<path d="M-50,0 A50,50 0 0 1 -25,-43" fill="none" stroke="{RED}" stroke-width="8"/>'
        f'<path d="M-25,-43 A50,50 0 0 1 25,-43" fill="none" stroke="{AMBER}" stroke-width="8"/>'
        f'<path d="M25,-43 A50,50 0 0 1 50,0" fill="none" stroke="{GREEN}" stroke-width="8"/>'
        f'<path d="M0,0 L32,-36" stroke="{INK}" stroke-width="4" stroke-linecap="round"/>'
        f'<circle cx="0" cy="0" r="6" fill="{INK}"/>'
    )


def page(label: str, accent: str) -> str:
    """A sheet of paper with a folded corner and a label."""
    return (
        f'<path d="M0,0 h70 l20,20 v90 h-90 z" fill="#FFFFFF" stroke="{INK}" stroke-width="2.5"/>'
        f'<path d="M70,0 v20 h20" fill="none" stroke="{INK}" stroke-width="2.5"/>'
        + "".join(f'<rect x="12" y="{y}" width="{w}" height="5" rx="2" fill="#C7D5E6"/>'
                  for y, w in [(34, 62), (46, 54), (58, 66), (70, 48), (82, 60)])
        + f'<text x="40" y="24" font-family="Menlo, monospace" font-size="{11 if len(label) <= 8 else 8}" '
          f'text-anchor="middle" fill="{accent}">{escape(label)}</text>'
    )


def chip() -> str:
    """A GPU module with a lightning bolt."""
    pins = "".join(f'<rect x="{x}" y="-8" width="6" height="8" fill="{SILVER_DARK}"/>'
                   f'<rect x="{x}" y="100" width="6" height="8" fill="{SILVER_DARK}"/>' for x in range(14, 140, 16))
    return (
        pins
        + f'<rect x="0" y="0" width="150" height="100" rx="10" fill="#2C3A4B" stroke="{INK}" stroke-width="3"/>'
        + f'<rect x="20" y="18" width="110" height="64" rx="6" fill="#1E2A38"/>'
        + f'<text x="75" y="56" font-family="Menlo, monospace" font-size="15" text-anchor="middle" fill="{SILVER}">sm_110</text>'
        + f'<g transform="translate(118,-34)">{bolt(0.6)}</g>'
    )


def placed(content: str, x: float, y: float, scale: float = 1.0, rotate: float = 0.0) -> str:
    """Moves, scales and rotates a drawing."""
    return f'<g transform="translate({x},{y}) rotate({rotate}) scale({scale})">{content}</g>'


def character(kind: str) -> str:
    """One full-body Thor tigress holding her tool, in a 260 x 440 box."""
    tools = {
        "hammer": placed(hammer(), 206, 208, 0.64),
        "lasso": placed(lasso(), 174, 178, 0.64, -18),
        "spark": placed(bolt(1.0), 206, 174, 0.80),
        "review": placed(magnifier(), 180, 199, 0.72),
        "quill": placed(quill(), 206, 239, 0.64),
    }
    art = (cape() + tail() + legs() + torso() + arms()
           + placed(tigress_head(), 30, 0, 1.0) + tools[kind] + paws())
    return with_halo(art, 10)


def svg(width: int, height: int, body: str, label: str) -> str:
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" '
            f'role="img" aria-label="{label}">{body}</svg>\n')


def plate(scene: str, label: str) -> str:
    return svg(900, 400, scene, label)


def plates() -> dict:
    """The chapter plates, keyed by file name. Each is drawn with no background."""
    flow_arrow = (f'<path d="M0,0 L60,0" stroke="{ACCENT_DARK}" stroke-width="5" stroke-linecap="round"/>'
                  f'<path d="M52,-10 L70,0 L52,10 Z" fill="{ACCENT_DARK}"/>')
    return {
        "ch01.svg": (
            placed(character("hammer"), 20, 70, 0.55)
            + placed(flow_arrow, 195, 190, 0.6)
            + placed(character("review"), 240, 70, 0.55)
            + placed(flow_arrow, 415, 190, 0.6)
            + placed(character("lasso"), 460, 70, 0.55)
            + placed(flow_arrow, 635, 190, 0.6)
            + placed(character("spark"), 680, 70, 0.55),
            "Four Thor tigresses in a row joined by arrows: Hammer, the reviewer with a magnifier, Lasso and Spark",
        ),
        "ch02.svg": (
            placed(character("review"), 40, 26, 0.80)
            + with_halo(placed(page("chat", BLUE), 400, 110, 1.5))
            + with_halo(placed(page("book", GREEN), 560, 130, 1.5, 4))
            + with_halo(placed(page("code", "#C4703A"), 720, 110, 1.5, -3)),
            "The reviewing Thor tigress beside three pages labelled chat, book and code",
        ),
        "ch03.svg": (
            placed(character("hammer"), 40, 26, 0.80)
            + with_halo(placed(page("raw", STEEL_DARK), 430, 110, 1.4, -6))
            + placed(flow_arrow, 592, 180)
            + with_halo(placed(page("train.jsonl", GREEN), 692, 100, 1.5)),
            "Hammer the Thor tigress turning a raw page into train.jsonl",
        ),
        "ch04.svg": (
            placed(character("review"), 40, 26, 0.80)
            + with_halo(placed(page("example", BLUE), 480, 80, 1.9))
            + f'<path d="M520,200 C550,192 580,208 610,198" fill="none" stroke="{RED}" stroke-width="4" stroke-linecap="round"/>'
            + f'<text x="625" y="234" font-family="Georgia, serif" font-size="20" font-style="italic" fill="{RED}">slop</text>',
            "The reviewing Thor tigress with a magnifier over an example page with one line underlined in red",
        ),
        "ch05.svg": (
            placed(character("hammer"), 40, 26, 0.80)
            + with_halo(placed(chip(), 560, 140, 1.5)),
            "Hammer the Thor tigress beside a GPU module marked sm_110 with a lightning bolt",
        ),
        "ch06.svg": (
            placed(character("spark"), 40, 26, 0.80)
            + with_halo(placed(scales(), 640, 210, 1.4)),
            "Spark the Thor tigress beside a balance with pans labelled A and B",
        ),
        "ch07.svg": (
            placed(character("spark"), 40, 26, 0.80)
            + with_halo(placed(gauge(), 620, 270, 1.7)),
            "Spark the Thor tigress beside a dial whose needle points into the green range",
        ),
        "ch08.svg": (
            placed(character("quill"), 40, 26, 0.80)
            + with_halo(placed(page("paper.pdf", INK), 540, 80, 2.1)),
            "A Thor tigress with a quill beside a page labelled paper.pdf",
        ),
        "appendix.svg": (
            placed(character("hammer"), 40, 26, 0.80)
            + with_halo(placed(lasso(), 600, 200, 1.0, -18))
            + with_halo(placed(bolt(1.1), 800, 110)),
            "Hammer the Thor tigress with the tools of the other two characters: a lasso and a lightning bolt",
        ),
    }


def tight_svg(body: str, label: str, canvas: int = 1024, pad: int = 18) -> str:
    """An SVG whose viewBox is cropped to the drawn art, so the logo has no empty margin."""
    probe = svg(canvas, canvas, body, label)
    png = cairosvg.svg2png(bytestring=probe.encode(), output_width=canvas, output_height=canvas)
    bounds = Image.open(io.BytesIO(png)).convert("RGBA").getbbox()
    if bounds is None:
        return probe
    x0, y0, x1, y1 = bounds
    x0, y0 = max(0, x0 - pad), max(0, y0 - pad)
    x1, y1 = min(canvas, x1 + pad), min(canvas, y1 + pad)
    width, height = x1 - x0, y1 - y0
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x0} {y0} {width} {height}" '
            f'width="{width}" height="{height}" role="img" aria-label="{label}">{body}</svg>\n')


def logo(glow: float = 1.0) -> str:
    """The free-form logo: a full-body Thor tigress with her hammer, between two bolts."""
    bolt_glow = 0.4 + 0.6 * glow
    art = (
        f'<g transform="translate(30,210) rotate(-8) scale(1.25)">{bolt(1.0, bolt_glow)}</g>'
        f'<g transform="translate(560,210) rotate(8) scale(-1.25,1.25)">{bolt(1.0, bolt_glow)}</g>'
        + placed(character("hammer"), 175, 20, 0.95)
    )
    return tight_svg(with_halo(art, 12),
                     "Thor Thunder Tigress logo: a winged-helm tigress holding a war hammer between two lightning bolts")


def cover() -> str:
    """The portrait cover: title, subtitle, one large Thor tigress, and the author."""
    title = text_haloed(360, 132, "Thor Thunder Tigress", "Georgia, serif", 56, INK,
                        weight="700", halo_width=12)
    subtitle = text_haloed(360, 180, "Fine-tuning a large language model on a Jetson AGX Thor",
                           "Georgia, serif", 20, CLOTH, italic=True, halo_width=7)
    author = text_haloed(360, 966, "Arpan Pathak", "Georgia, serif", 18, INK, halo_width=6)
    body = title + subtitle + placed(character("hammer"), 158, 235, 1.55) + author
    return svg(720, 1000, body,
               "Book cover: the title Thor Thunder Tigress above the winged-helm tigress mascot holding her war hammer")


def render_png(svg_text: str, path: "Path | None", scale: float = 1.0) -> Image.Image:
    png = cairosvg.svg2png(bytestring=svg_text.encode(), scale=scale)
    image = Image.open(io.BytesIO(png)).convert("RGBA")
    if path is not None:
        image.save(path)
    return image


def animated_logo(path: Path) -> None:
    """A 2-second loop: the bolts flash twice. The background stays transparent."""
    glow_per_frame = [0.15, 0.15, 0.15, 1.0, 0.4, 1.0, 0.6, 0.45, 0.35, 0.25, 0.2, 0.15,
                      0.15, 0.15, 0.15, 0.15, 0.15, 0.15, 0.15, 0.15]
    frames = []
    for glow in glow_per_frame:
        image = render_png(logo(glow), None, 0.5)
        paletted = image.convert("RGB").convert("P", palette=Image.ADAPTIVE, colors=255)
        transparent = image.split()[3].point(lambda alpha: 255 if alpha < 128 else 0)
        paletted.paste(255, mask=transparent)
        frames.append(paletted)
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=100, loop=0,
                   transparency=255, disposal=2, optimize=True)


def main() -> None:
    ART.mkdir(parents=True, exist_ok=True)
    for name, (scene, label) in plates().items():
        (ART / name).write_text(plate(scene, label))
    (ART / "logo.svg").write_text(logo())
    (ART / "cover.svg").write_text(cover())
    for name in ["cover", "logo"]:
        render_png((ART / f"{name}.svg").read_text(), ART / f"{name}.png")
    animated_logo(ART / "logo.gif")
    print(f"wrote {len(list(ART.iterdir()))} files to {ART}")


if __name__ == "__main__":
    main()
