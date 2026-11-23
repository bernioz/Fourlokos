#!/usr/bin/env python3
"""ASCII material map of a reference image region, for measuring parts in pixels.

Usage: reference_map.py IMAGE X0 Y0 X1 Y1 [STEP=5]
Needs Pillow (python -m pip install pillow). Legend: B bone/light  w warm brown/beige  # black
c dark/other  R red  . flat gray background.
"""
import sys


def classify(r: int, g: int, b: int) -> str:
    if r > 150 and g > 140 and b > 100:
        return "B"
    if r > 120 and g < 60 and b < 60:
        return "R"
    if abs(r - g) < 12 and abs(g - b) < 14 and 40 <= r <= 75:
        return "."
    if r > 85 and r > b + 25 and g > b:
        return "w"
    if r < 45 and g < 45 and b < 45:
        return "#"
    return "c"


def main(argv: list[str]) -> int:
    if len(argv) not in (6, 7):
        print(__doc__, file=sys.stderr)
        return 1
    try:
        from PIL import Image
    except ImportError:
        print("Pillow is missing: python -m pip install pillow", file=sys.stderr)
        return 1
    x0, y0, x1, y1 = map(int, argv[2:6])
    step = int(argv[6]) if len(argv) == 7 else 5
    img = Image.open(argv[1]).convert("RGB")
    w, h = img.size
    x1, y1 = min(x1, w), min(y1, h)
    px = img.load()
    print("      " + "".join(str((x // 10) % 10) if x % 10 == 0 else " " for x in range(x0, x1, step)))
    for y in range(y0, y1, step):
        print(f"{y:5d} " + "".join(classify(*px[x, y]) for x in range(x0, x1, step)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
