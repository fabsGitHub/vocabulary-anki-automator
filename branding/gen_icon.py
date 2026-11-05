"""Generate the final app icon SVG (1024 grid). Usage: gen_icon.py OUT.svg [key=value ...]"""
import math
import sys

P = dict(
    ox=-32, oy=32,                       # front-card offset, multiples of 32
    tlx=284, tly=296,                  # back card: where its top-left vertex lands
    back_rot=-10.0, back_w=528, back_h=408, back_op=0.55,
    line="#D9D0F0", body_top="#FF9A5C", body_bot="#E2306B",
    gx1=412, gx2=612,
    serif_w=104, serif_t=28, stem_w=32,
    shadow_op=0.30, fillet=8,
)
for a in sys.argv[2:]:
    k, v = a.split("=")
    P[k] = v if isinstance(P[k], str) else type(P[k])(v)

OX, OY = P["ox"], P["oy"]
# Front card, 576 x 448, on the 32-unit grid
CX0, CY0, CX1, CY1, CR = 224 + OX, 320 + OY, 800 + OX, 768 + OY, 64
# Selection 128 tall; its right edge is the caret stem's centre line
SX0, SY0, SY1, SR = 288 + OX, 384 + OY, 512 + OY, 24
STEM_C = 656 + OX
STEM_W, SERIF_W, SERIF_T = P["stem_w"], P["serif_w"], P["serif_t"]
# Serifs centred on the selection's top and bottom edges
T_TOP = SY0 - SERIF_T / 2
B_BOT = SY1 + SERIF_T / 2

# Back card: rotated, placed by its top-left vertex
bw, bh, rot = P["back_w"], P["back_h"], P["back_rot"]
t = math.radians(rot)
bcx = P["tlx"] + OX - (-bw / 2 * math.cos(t) + bh / 2 * math.sin(t))
bcy = P["tly"] + OY - 32 - (-bw / 2 * math.sin(t) - bh / 2 * math.cos(t))


def ibeam():
    r = SERIF_T / 2
    sl = STEM_C - SERIF_W / 2 + r      # left end of the serifs' straight part
    sr = STEM_C + SERIF_W / 2 - r      # right end
    s0, s1 = STEM_C - STEM_W / 2, STEM_C + STEM_W / 2
    f = P["fillet"]                     # inner-corner fillet where stem meets serif
    tb = T_TOP + SERIF_T                # underside of the top serif
    bt = B_BOT - SERIF_T                # top of the bottom serif
    return (
        f"M{sl:g} {T_TOP:g}H{sr:g}A{r:g} {r:g} 0 0 1 {sr:g} {tb:g}"
        f"H{s1 + f:g}A{f:g} {f:g} 0 0 0 {s1:g} {tb + f:g}V{bt - f:g}A{f:g} {f:g} 0 0 0 {s1 + f:g} {bt:g}H{sr:g}"
        f"A{r:g} {r:g} 0 0 1 {sr:g} {B_BOT:g}H{sl:g}A{r:g} {r:g} 0 0 1 {sl:g} {bt:g}"
        f"H{s0 - f:g}A{f:g} {f:g} 0 0 0 {s0:g} {bt - f:g}V{tb + f:g}A{f:g} {f:g} 0 0 0 {s0 - f:g} {tb:g}"
        f"H{sl:g}A{r:g} {r:g} 0 0 1 {sl:g} {T_TOP:g}Z"
    )


def selection():
    # rounded on the left; square on the right, where the caret stem covers the edge
    return (
        f"M{SX0 + SR} {SY0}H{STEM_C}V{SY1}H{SX0 + SR}"
        f"A{SR} {SR} 0 0 1 {SX0} {SY1 - SR}V{SY0 + SR}A{SR} {SR} 0 0 1 {SX0 + SR} {SY0}Z"
    )


svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <!-- App icon: a selected word, with the text caret at its end, sits on a flashcard in front of a deck.
       1024 grid. Body 824x824 at (100,100), r=185. Front card, selection and caret stem are on the 32-unit grid.
       Layers stay separate (body, back card, front card, selection, word, caret) for tinted or clear variants. -->
  <defs>
    <linearGradient id="icon-body" x1="{P['gx1']}" y1="100" x2="{P['gx2']}" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="{P['body_top']}"/>
      <stop offset="1" stop-color="{P['body_bot']}"/>
    </linearGradient>
    <linearGradient id="icon-sheen" x1="0" y1="100" x2="0" y2="512" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.20"/>
      <stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="icon-paper" x1="0" y1="{CY0}" x2="0" y2="{CY1}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF"/>
      <stop offset="1" stop-color="#FFF5F3"/>
    </linearGradient>
    <linearGradient id="icon-sel" x1="0" y1="{SY0}" x2="0" y2="{SY1}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#AE98FF"/>
      <stop offset="1" stop-color="#9478FA"/>
    </linearGradient>
    <linearGradient id="icon-ink" x1="0" y1="{T_TOP:g}" x2="0" y2="{B_BOT:g}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#3A1650"/>
      <stop offset="1" stop-color="#2C0F40"/>
    </linearGradient>
    <filter id="icon-bodyShadow" x="0" y="0" width="1024" height="1024" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
      <feOffset in="SourceAlpha" dy="12" result="o"/>
      <feGaussianBlur in="o" stdDeviation="14" result="b"/>
      <feFlood flood-color="#000000" flood-opacity="0.28"/>
      <feComposite in2="b" operator="in" result="s"/>
      <feMerge><feMergeNode in="s"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <filter id="icon-cardShadow" x="0" y="0" width="1024" height="1024" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
      <feOffset in="SourceAlpha" dy="18" result="o"/>
      <feGaussianBlur in="o" stdDeviation="20" result="b"/>
      <feFlood flood-color="#7A1238" flood-opacity="{P['shadow_op']}"/>
      <feComposite in2="b" operator="in" result="s"/>
      <feMerge><feMergeNode in="s"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
  </defs>

  <g id="layer-body">
    <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#icon-body)" filter="url(#icon-bodyShadow)"/>
    <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#icon-sheen)"/>
  </g>

  <g id="layer-back-card">
    <rect x="{-bw / 2:g}" y="{-bh / 2:g}" width="{bw}" height="{bh}" rx="{CR}" fill="#FFFFFF" fill-opacity="{P['back_op']}"
          transform="translate({bcx:.1f} {bcy:.1f}) rotate({rot:g})"/>
  </g>

  <g id="layer-front-card">
    <rect x="{CX0}" y="{CY0}" width="{CX1 - CX0}" height="{CY1 - CY0}" rx="{CR}" fill="url(#icon-paper)" filter="url(#icon-cardShadow)"/>
    <rect x="{SX0}" y="{608 + OY}" width="448" height="32" rx="16" fill="{P['line']}"/>
    <rect x="{SX0}" y="{672 + OY}" width="320" height="32" rx="16" fill="{P['line']}"/>
  </g>

  <g id="layer-selection">
    <path d="{selection()}" fill="url(#icon-sel)"/>
  </g>

  <g id="layer-word">
    <rect x="{SX0 + 48}" y="{432 + OY}" width="256" height="32" rx="16" fill="url(#icon-ink)"/>
  </g>

  <g id="layer-caret">
    <path d="{ibeam()}" fill="url(#icon-ink)"/>
  </g>
</svg>
'''
open(sys.argv[1], "w").write(svg)
