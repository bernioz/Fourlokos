#!/usr/bin/env python3
"""Render a 16x16 isometric inventory icon from a Bedrock .geo.json + texture.
Usage: python3 -I tools/geo_icon.py model.geo.json texture.png out.png"""
import json, math, sys
from PIL import Image

geo, tex, out = sys.argv[1:4]
g = json.load(open(geo))["minecraft:geometry"][0]
T = Image.open(tex).convert("RGBA")
W, H = T.size
S = 8  # supersampling
N = 16 * S
cubes = [c for b in g["bones"] for c in b.get("cubes", [])]

def sample(face, c):
    spec = c["uv"][face] if isinstance(c.get("uv"), dict) and face in c["uv"] else None
    if not spec:
        return (200, 200, 200, 255)
    u, v = spec["uv"]; du, dv = spec["uv_size"]
    x = min(W - 1, max(0, int(u + du / 2))); y = min(H - 1, max(0, int(v + dv / 2)))
    return T.getpixel((x, y))

# isometric: screen x = (x - z) * cos30, screen y = y - (x + z) * sin30 / ... (y up)
c30, s30 = math.cos(math.pi / 6), 0.5
faces = []  # (depth, polygon, color)
for c in cubes:
    ox, oy, oz = c["origin"]; sx, sy, sz = c["size"]
    x0, x1, y0, y1, z0, z1 = ox, ox + sx, oy, oy + sy, oz, oz + sz
    P = lambda x, y, z: ((x - z) * c30, y + (x + z) * s30 * 0.0 - (x + z) * s30 * 0.5)
    def quad(face, pts, shade):
        col = sample(face, c)
        col = tuple(int(col[i] * shade) for i in range(3)) + (col[3],)
        d = sum(p[0] + p[1] + p[2] for p in pts) / 4  # larger = nearer to viewer (+x,+y,+z)
        faces.append((d, [P(*p) for p in pts], col))
    quad("up", [(x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)], 1.0)
    quad("south", [(x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)], 0.8)
    quad("east", [(x1, y0, z0), (x1, y0, z1), (x1, y1, z1), (x1, y1, z0)], 0.62)
faces.sort(key=lambda f: f[0])
xs = [p[0] for f in faces for p in f[1]]; ys = [p[1] for f in faces for p in f[1]]
minx, maxx, miny, maxy = min(xs), max(xs), min(ys), max(ys)
scale = (N - 2 * S) / max(maxx - minx, maxy - miny, 1e-6)
ox = (N - (maxx - minx) * scale) / 2; oy = (N - (maxy - miny) * scale) / 2
img = Image.new("RGBA", (N, N), (0, 0, 0, 0))
from PIL import ImageDraw
d = ImageDraw.Draw(img)
for _, poly, col in faces:
    if col[3] < 128:
        continue
    d.polygon([((x - minx) * scale + ox, (maxy - y) * scale + oy) for x, y in poly], fill=col)
img.resize((16, 16), Image.BOX).save(out)
