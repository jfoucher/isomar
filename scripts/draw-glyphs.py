"""Draws the value glyphs (isomar-satellite-symbolic and the rest): 16x16 filled outlines, which GTK recolours.

Run from the repository root: python3 scripts/draw-glyphs.py [out-dir]. Needs shapely."""
import math
import sys
from shapely.geometry import Point, Polygon, LineString, box
from shapely.ops import unary_union
from shapely import affinity

OUT = sys.argv[1] if len(sys.argv) > 1 else "data/icons/scalable/actions"


def rect(x, y, w, h):
    return box(x, y, x + w, y + h)


def disc(cx, cy, r):
    return Point(cx, cy).buffer(r, quad_segs=16)


def ring(cx, cy, r, w):
    return disc(cx, cy, r).difference(disc(cx, cy, r - w))


def line(pts, w, cap="flat", join="mitre"):
    return LineString(pts).buffer(w / 2, cap_style=cap, join_style=join, mitre_limit=4, quad_segs=16)


def poly(pts):
    return Polygon(pts)


def arrow(start, angle_deg, length, shaft=2.0, head_len=4.0, head_w=5.5):
    """An arrow from `start` heading `angle_deg` (0 = up, clockwise), tip `length` away."""
    a = math.radians(angle_deg)
    d = (math.sin(a), -math.cos(a))
    n = (-d[1], d[0])
    tip = (start[0] + d[0] * length, start[1] + d[1] * length)
    base = (tip[0] - d[0] * head_len, tip[1] - d[1] * head_len)
    s = line([start, (base[0] + d[0] * 0.5, base[1] + d[1] * 0.5)], shaft)
    h = poly([tip, (base[0] + n[0] * head_w / 2, base[1] + n[1] * head_w / 2), (base[0] - n[0] * head_w / 2, base[1] - n[1] * head_w / 2)])
    return s.union(h)


def sine(x0, x1, y, amp, period, phase=0.0, step=0.25):
    pts = []
    x = x0
    while x <= x1 + 1e-9:
        pts.append((x, y - amp * math.cos(2 * math.pi * (x - phase) / period)))
        x += step
    return pts


glyphs = {}

# Satellite: body, two panels on struts, a dish facing away; turned 45 degrees.
sat = unary_union([
    rect(6.5, 4.5, 3, 6.5),
    rect(0.8, 5.5, 4.7, 4.5), rect(10.5, 5.5, 4.7, 4.5),
    rect(5.5, 7.0, 1, 1.5), rect(9.5, 7.0, 1, 1.5),
    rect(7.25, 11, 1.5, 1.2),
    disc(8, 14.6, 2.8).intersection(rect(4, 10, 8, 4.6)),
])
glyphs["satellite"] = affinity.rotate(sat, 45, origin=(8, 8))

# Speed: an arrowhead running right, motion bars behind it.
glyphs["speed"] = unary_union([
    poly([(8, 2.5), (15, 8), (8, 13.5)]),
    rect(1, 7, 6, 2), rect(3, 3.5, 4, 2), rect(3, 10.5, 4, 2),
])

# Engine: the engine warning light's silhouette.
glyphs["engine"] = unary_union([
    rect(5, 1.5, 6, 2), rect(7, 3.5, 2, 2.5),
    rect(4, 6, 8, 7.5),
    rect(12, 8.5, 1.2, 2.5), rect(13, 7, 2, 6),
    rect(2.5, 9, 1.6, 2), rect(1, 7.5, 2, 5),
])

# Depth: the surface, an arrow down, the seabed.
glyphs["depth"] = unary_union([
    line(sine(1, 15, 3, 1.1, 7, 1 + 7 / 4), 2),
    arrow((8, 6), 180, 7.5, head_len=3.5, head_w=6.5),
    rect(1, 14, 14, 2),
])

# Waves: the sea under two crests, one band of water cut out below the surface.
surface = sine(1, 15, 7.5, 2.2, 8, 4)
sea = poly(surface + [(15, 15), (1, 15)])
cut = line(sine(0, 16, 11.5, 2.2, 8, 4), 1.5)
glyphs["waves"] = sea.difference(cut)

# Current: a stream arrow over a band of water, the way a tidal-stream arrow is drawn on a chart.
glyphs["current"] = unary_union([
    arrow((1.5, 6), 90, 13, shaft=2.5, head_len=5, head_w=8),
    line(sine(1, 15, 13, 1.1, 7, 1 + 7 / 4), 2),
])

# Pressure: a barometer's dial and needle.
a = math.radians(45)
tip = (8 + 4.4 * math.sin(a), 8 - 4.4 * math.cos(a))
glyphs["pressure"] = unary_union([
    ring(8, 8, 7.5, 2),
    poly([tip, (8 - 1.1 * math.cos(a), 8 - 1.1 * math.sin(a)), (8 + 1.1 * math.cos(a), 8 + 1.1 * math.sin(a))]),
    disc(8, 8, 1.8),
])

# Temperature: a thermometer, its column up to the middle, two ticks.
outer = unary_union([line([(8, 3.5), (8, 10)], 5, cap="round"), disc(8, 12, 3.5)])
inner = line([(8, 3.5), (8, 7.5)], 2, cap="round")
glyphs["temperature"] = unary_union([outer.difference(inner), rect(12, 3, 2.5, 1.5), rect(12, 6, 2.5, 1.5)])

# Compass: a ring round a needle, north solid, south hollow.
north = poly([(8, 2.2), (10.6, 8), (5.4, 8)])
south = poly([(5.4, 8), (10.6, 8), (8, 13.8)])
glyphs["compass"] = unary_union([
    ring(8, 8, 7.7, 1.5),
    north,
    south.difference(south.buffer(-1.2, join_style="mitre")),
])

# Upwind (close-hauled): the wind coming down, the boat heading up 40 degrees off it.
glyphs["upwind"] = unary_union([
    arrow((13, 0.5), 180, 9, head_len=3.5, head_w=5.5),
    arrow((9.5, 15.5), -40, 12.5, head_len=4, head_w=5.5),
])

# Tack: a zigzag up to an arrowhead.
pts = [(3.5, 15), (8.5, 11), (4, 7), (7.5, 4.2)]
d = (pts[-1][0] - pts[-2][0], pts[-1][1] - pts[-2][1])
heading = math.degrees(math.atan2(d[0], -d[1]))
glyphs["tack"] = unary_union([
    line(pts, 2),
    arrow(pts[-1], heading, 5.2, head_len=4.5, head_w=6),
])


def path_d(g):
    def ring_d(coords):
        c = list(coords)[:-1]
        return "M" + "L".join(f"{x:.2f} {y:.2f}".replace(".00", "") for x, y in c) + "Z"

    parts = []
    geoms = getattr(g, "geoms", [g])
    for p in geoms:
        parts.append(ring_d(p.exterior.coords))
        for h in p.interiors:
            parts.append(ring_d(h.coords))
    return "".join(parts)


for name, g in glyphs.items():
    g = g.intersection(box(0, 0, 16, 16))
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">'
        f'<g fill="#222"><path fill-rule="evenodd" d="{path_d(g)}"/></g></svg>\n'
    )
    with open(f"{OUT}/isomar-{name}-symbolic.svg", "w") as f:
        f.write(svg)
    print(name, len(svg))
