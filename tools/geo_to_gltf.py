#!/usr/bin/env python3
"""Convert a Bedrock geometry JSON to a small, self-contained glTF 2.0 file.

Usage (also works with ``python3 -I``)::

    python3 -I tools/geo_to_gltf.py art/bread_item/bread_item.geo.json \
        --texture art/bread_item/bread_item.png \
        --animation art/bread_item/bread_item.animation.json \
        -o crates/client/assets/models/bread.gltf

The converter intentionally uses only the Python standard library.  Bedrock
cubes become one mesh per bone, attached below the bone node.  Thus pivots,
parenting, stage visibility animations, and ordinary bone animations remain
editable in glTF without requiring a skinned mesh.  Coordinates are divided
by 16 (one Blockbench unit is one sixteenth of a metre).
"""

import argparse
import base64
import json
import math
import mimetypes
import struct
from pathlib import Path


FACE_AXES = {
    "north": ((0, 0, -1), ("x", "y")),
    "south": ((0, 0, 1), ("x", "y")),
    "east": ((1, 0, 0), ("z", "y")),
    "west": ((-1, 0, 0), ("z", "y")),
    "up": ((0, 1, 0), ("x", "z")),
    "down": ((0, -1, 0), ("x", "z")),
}


def read_png_size(path):
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"not a PNG: {path}")
    return struct.unpack(">II", data[16:24])


def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return [aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz]


def euler_quaternion(degrees):
    # Bedrock animation rotations are XYZ Euler degrees.  glTF stores quaternions.
    x, y, z = (math.radians(float(v)) for v in (degrees or [0, 0, 0]))
    qx = [math.sin(x / 2), 0, 0, math.cos(x / 2)]
    qy = [0, math.sin(y / 2), 0, math.cos(y / 2)]
    qz = [0, 0, math.sin(z / 2), math.cos(z / 2)]
    return qmul(qmul(qz, qy), qx)


class GltfBuilder:
    def __init__(self):
        self.buf = bytearray()
        self.views = []
        self.accessors = []

    def add_data(self, raw, target=None):
        while len(self.buf) % 4:
            self.buf.append(0)
        offset = len(self.buf)
        self.buf.extend(raw)
        view = {"buffer": 0, "byteOffset": offset, "byteLength": len(raw)}
        if target is not None:
            view["target"] = target
        self.views.append(view)
        return len(self.views) - 1

    def accessor(self, values, component_type, kind, target=None):
        packed = {5126: "f", 5123: "H", 5125: "I"}[component_type]
        raw = struct.pack("<" + packed * len(values), *values)
        view = self.add_data(raw, target)
        width = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[kind]
        rows = [values[i:i + width] for i in range(0, len(values), width)]
        accessor = {"bufferView": view, "componentType": component_type,
                    "count": len(rows), "type": kind}
        if component_type == 5126 and rows:
            accessor["min"] = [min(r[j] for r in rows) for j in range(width)]
            accessor["max"] = [max(r[j] for r in rows) for j in range(width)]
        self.accessors.append(accessor)
        return len(self.accessors) - 1


def cube_mesh(builder, cubes, pivot, tex_size):
    positions, normals, uvs, indices = [], [], [], []
    w, h = tex_size
    for cube in cubes:
        ox, oy, oz = cube.get("origin", [0, 0, 0])
        sx, sy, sz = cube.get("size", [0, 0, 0])
        lo = [ox - pivot[0], oy - pivot[1], oz - pivot[2]]
        hi = [lo[0] + sx, lo[1] + sy, lo[2] + sz]
        corners = {
            "xmin": lo[0] / 16, "xmax": hi[0] / 16,
            "ymin": lo[1] / 16, "ymax": hi[1] / 16,
            "zmin": lo[2] / 16, "zmax": hi[2] / 16,
        }
        # Counter-clockwise when viewed from outside, in glTF's right-handed axes.
        faces = {
            "north": [("xmin", "ymin", "zmin"), ("xmax", "ymin", "zmin"), ("xmax", "ymax", "zmin"), ("xmin", "ymax", "zmin")],
            "south": [("xmax", "ymin", "zmax"), ("xmin", "ymin", "zmax"), ("xmin", "ymax", "zmax"), ("xmax", "ymax", "zmax")],
            "east": [("xmax", "ymin", "zmin"), ("xmax", "ymin", "zmax"), ("xmax", "ymax", "zmax"), ("xmax", "ymax", "zmin")],
            "west": [("xmin", "ymin", "zmax"), ("xmin", "ymin", "zmin"), ("xmin", "ymax", "zmin"), ("xmin", "ymax", "zmax")],
            "up": [("xmin", "ymax", "zmin"), ("xmax", "ymax", "zmin"), ("xmax", "ymax", "zmax"), ("xmin", "ymax", "zmax")],
            "down": [("xmin", "ymin", "zmax"), ("xmax", "ymin", "zmax"), ("xmax", "ymin", "zmin"), ("xmin", "ymin", "zmin")],
        }
        for face, points in faces.items():
            spec = cube.get("uv", {}).get(face)
            if spec is None:
                # Bedrock's compact UV form: a shared rectangle is still useful.
                spec = {"uv": [0, 0], "uv_size": [sx, sy]}
            uv0 = spec.get("uv", [0, 0])
            duv = spec.get("uv_size", [0, 0])
            uv_corners = [(uv0[0], uv0[1]), (uv0[0] + duv[0], uv0[1]),
                          (uv0[0] + duv[0], uv0[1] + duv[1]), (uv0[0], uv0[1] + duv[1])]
            start = len(positions) // 3
            normal = FACE_AXES[face][0]
            for point, (u, v) in zip(points, uv_corners):
                positions.extend(corners[k] for k in point)
                normals.extend(normal)
                uvs.extend([float(u) / w, float(v) / h])
            indices.extend([start, start + 1, start + 2, start, start + 2, start + 3])
    if not positions:
        return None
    return {"attributes": {
        "POSITION": builder.accessor(positions, 5126, "VEC3", 34962),
        "NORMAL": builder.accessor(normals, 5126, "VEC3", 34962),
        "TEXCOORD_0": builder.accessor(uvs, 5126, "VEC2", 34962),
    }, "indices": builder.accessor(indices, 5123, "SCALAR", 34963), "mode": 4}


def keyed_values(value, length):
    if isinstance(value, dict):
        return [(float(k), keyed_values(v, length)[0][1]) for k, v in sorted(value.items(), key=lambda x: float(x[0]))]
    if isinstance(value, (int, float)):
        return [(0.0, [float(value)] * length)]
    return [(0.0, [float(v) for v in value])]


def animation_channel(builder, animations, name, bone_nodes, anim, rest=None):
    rest = rest or {}
    duration = float(anim.get("animation_length", 0.0))
    samplers, channels = [], []
    for bone, transforms in anim.get("bones", {}).items():
        if bone not in bone_nodes:
            continue
        for prop, path, width in (("position", "translation", 3), ("rotation", "rotation", 4), ("scale", "scale", 3)):
            if prop not in transforms:
                continue
            raw = transforms[prop]
            if isinstance(raw, dict):
                points = []
                for key, val in sorted(raw.items(), key=lambda x: float(x[0])):
                    if isinstance(val, dict):
                        val = val.get("post", val.get("pre", [0, 0, 0]))
                    points.append((float(key), val))
            else:
                points = [(0.0, raw)]
            times = [t for t, _ in points]
            values = []
            for _, val in points:
                if path == "rotation":
                    if isinstance(val, (int, float)):
                        val = [val, val, val]
                    values.extend(euler_quaternion(val))
                else:
                    if isinstance(val, (int, float)):
                        val = [val, val, val]
                    values.extend(float(x) for x in val)
                    if path == "translation":
                        # Bedrock positions are offsets from the bone's rest pose; glTF's are absolute.
                        base = rest.get(bone, [0, 0, 0])
                        values[-3:] = [x / 16 + base[i] for i, x in enumerate(values[-3:])]
            inp = builder.accessor(times, 5126, "SCALAR")
            out = builder.accessor(values, 5126, "VEC4" if path == "rotation" else "VEC3")
            sampler = len(samplers)
            samplers.append({"input": inp, "output": out, "interpolation": "LINEAR"})
            channels.append({"sampler": sampler, "target": {"node": bone_nodes[bone], "path": path}})
            duration = max(duration, max(times, default=0.0))
    if channels:
        animations.append({"name": name, "samplers": samplers, "channels": channels})


def convert(geo_path, texture_path, out_path, animation_path=None, geometry_index=0):
    geo = json.loads(Path(geo_path).read_text(encoding="utf-8"))
    geometries = geo.get("minecraft:geometry", [])
    if not geometries:
        raise ValueError("minecraft:geometry is empty")
    geometry = geometries[geometry_index]
    desc = geometry.get("description", {})
    tex_size = (desc.get("texture_width", read_png_size(texture_path)[0]),
                desc.get("texture_height", read_png_size(texture_path)[1]))
    builder, nodes, meshes, bone_nodes = GltfBuilder(), [], [], {}
    bones = geometry.get("bones", [])
    by_name = {b["name"]: b for b in bones}
    for bone in bones:
        parent = bone.get("parent")
        pivot = bone.get("pivot", [0, 0, 0])
        parent_pivot = by_name.get(parent, {}).get("pivot", [0, 0, 0]) if parent else [0, 0, 0]
        node = {"name": bone["name"], "translation": [(pivot[i] - parent_pivot[i]) / 16 for i in range(3)]}
        mesh = cube_mesh(builder, bone.get("cubes", []), pivot, tex_size)
        if mesh is not None:
            mesh["material"] = 0
            meshes.append({"name": bone["name"], "primitives": [mesh]})
            node["mesh"] = len(meshes) - 1
        bone_nodes[bone["name"]] = len(nodes)
        nodes.append(node)
    for bone in bones:
        if bone.get("parent") in bone_nodes:
            nodes[bone_nodes[bone["parent"]]].setdefault("children", []).append(bone_nodes[bone["name"]])
    animations = []
    if animation_path and Path(animation_path).exists():
        anim_json = json.loads(Path(animation_path).read_text(encoding="utf-8"))
        for name, anim in anim_json.get("animations", {}).items():
            animation_channel(builder, animations, name, bone_nodes, anim, {n['name']: n['translation'] for n in nodes})
    image = Path(texture_path).read_bytes()
    mime = mimetypes.guess_type(str(texture_path))[0] or "image/png"
    doc = {"asset": {"version": "2.0", "generator": "Fourlokos geo_to_gltf.py"},
           "scene": 0, "scenes": [{"nodes": [i for i, b in enumerate(bones) if not b.get("parent")] }],
           "nodes": nodes, "meshes": meshes,
           "materials": [{"name": "bedrock_texture", "doubleSided": True,
                          "alphaMode": "MASK", "alphaCutoff": 0.5,
                          "pbrMetallicRoughness": {"roughnessFactor": 1.0,
                          "baseColorTexture": {"index": 0}}}],
           "textures": [{"sampler": 0, "source": 0}],
           "samplers": [{"magFilter": 9728, "minFilter": 9728, "wrapS": 33071, "wrapT": 33071}],
           "images": [{"mimeType": mime, "uri": "data:" + mime + ";base64," + base64.b64encode(image).decode("ascii")}],
           "buffers": [{"byteLength": len(builder.buf), "uri": "data:application/octet-stream;base64," + base64.b64encode(builder.buf).decode("ascii")}],
           "bufferViews": builder.views, "accessors": builder.accessors}
    if animations:
        doc["animations"] = animations
    Path(out_path).parent.mkdir(parents=True, exist_ok=True)
    Path(out_path).write_text(json.dumps(doc, separators=(",", ":"), ensure_ascii=False), encoding="utf-8")
    return doc


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("geometry", type=Path)
    parser.add_argument("--texture", type=Path, required=True)
    parser.add_argument("--animation", type=Path)
    parser.add_argument("-o", "--output", type=Path, required=True)
    parser.add_argument("--geometry-index", type=int, default=0)
    args = parser.parse_args()
    convert(args.geometry, args.texture, args.output, args.animation, args.geometry_index)


if __name__ == "__main__":
    main()
