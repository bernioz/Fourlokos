#!/usr/bin/env python3
"""Validate self-contained JSON glTF files produced by geo_to_gltf.py.

Usage: ``python3 -I tools/validate_gltf.py crates/client/assets/models/*.gltf``
"""

import base64
import json
import sys
from pathlib import Path


def validate(path):
    doc = json.loads(Path(path).read_text(encoding="utf-8"))
    assert doc.get("asset", {}).get("version") == "2.0", "missing glTF 2.0 asset"
    buffers = doc.get("buffers", [])
    assert buffers, "no buffers"
    for buffer in buffers:
        uri = buffer.get("uri", "")
        assert uri.startswith("data:"), "expected embedded buffer"
        data = base64.b64decode(uri.split(",", 1)[1])
        assert len(data) == buffer["byteLength"], "buffer byteLength mismatch"
    for view in doc.get("bufferViews", []):
        buffer = buffers[view["buffer"]]
        assert view.get("byteOffset", 0) + view["byteLength"] <= buffer["byteLength"], "bufferView outside buffer"
    for accessor in doc.get("accessors", []):
        view = doc["bufferViews"][accessor["bufferView"]]
        assert view.get("byteOffset", 0) + view["byteLength"] <= buffers[view["buffer"]]["byteLength"], "accessor outside buffer"
    node_count = len(doc.get("nodes", []))
    accessor_count = len(doc.get("accessors", []))
    for mesh in doc.get("meshes", []):
        for primitive in mesh.get("primitives", []):
            assert primitive["indices"] < accessor_count
            assert all(index < accessor_count for index in primitive.get("attributes", {}).values())
    for animation in doc.get("animations", []):
        for channel in animation.get("channels", []):
            assert channel["target"]["node"] < node_count
            assert channel["sampler"] < len(animation["samplers"])
    return len(doc.get("nodes", [])), len(doc.get("animations", []))


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit("usage: validate_gltf.py FILE...")
    for filename in sys.argv[1:]:
        nodes, animations = validate(filename)
        print(f"OK {filename}: {nodes} nodos, {animations} animaciones")
