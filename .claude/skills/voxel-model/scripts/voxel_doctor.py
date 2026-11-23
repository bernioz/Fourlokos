#!/usr/bin/env python3
"""voxel_doctor — check everything the voxel-model workflow needs. Stdout: one JSON object.

Exit 0 when every required check passes, 1 otherwise. A failing check carries a `fix` hint.
Checks: python version, OPENAI_API_KEY (set and accepted by the API, with an image model available),
the Blockbench MCP server, and Pillow (optional, only for reference_map.py).
Nothing here spends API credit: listing models is free.
"""
from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import openai_images as oi  # noqa: E402

DEFAULT_MCP_URL = "http://localhost:3000/bb-mcp"
PLUGIN_URL = "https://jasonjgardner.github.io/blockbench-mcp-plugin/mcp.js"


@dataclass
class Check:
    name: str
    ok: bool
    fix: str = ""
    optional: bool = False
    detail: str = ""


def check_python() -> Check:
    ok = sys.version_info >= (3, 9)
    return Check("python", ok, "" if ok else "Install Python 3.9 or newer from python.org", detail=sys.version.split()[0])


def check_openai() -> list[Check]:
    if not os.environ.get("OPENAI_API_KEY", "").strip():
        return [Check("openai_api_key", False, "Set OPENAI_API_KEY (README, 'Modelos 3D', paso 4 de la instalación), then open a new terminal")]
    key = os.environ["OPENAI_API_KEY"].strip()
    try:
        models = oi.list_models(key)
    except oi.GenError as e:
        fix = {"invalid_api_key": "The key was rejected: create a new one at platform.openai.com/api-keys"}.get(e.code, str(e))
        return [Check("openai_api_key", False, fix)]
    model = oi.pick_model(models)
    return [Check("openai_api_key", True),
            Check("openai_image_model", model is not None,
                  "" if model else "The key sees no gpt-image model: verify the organization at platform.openai.com/settings/organization/general",
                  detail=model or "")]


def check_blockbench(url: str) -> Check:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "voxel-doctor", "version": "0.2"}}}).encode()
    req = urllib.request.Request(url, data=body, method="POST", headers={
        "Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            if r.status == 200:
                return Check("blockbench_mcp", True)
    except (urllib.error.URLError, OSError):
        pass
    return Check("blockbench_mcp", False,
                 f"Open Blockbench (desktop) and load the MCP plugin from {PLUGIN_URL} (File > Plugins > Load Plugin from URL); expected at {url}")


def check_pillow() -> Check:
    try:
        import PIL  # noqa: F401
        return Check("pillow", True, optional=True)
    except ImportError:
        return Check("pillow", False, "Optional, only for reference_map.py: python -m pip install pillow", optional=True)


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    checks = [check_python(), *check_openai(), check_blockbench(os.environ.get("BLOCKBENCH_MCP_URL", DEFAULT_MCP_URL)), check_pillow()]
    ok = all(c.ok for c in checks if not c.optional)
    sys.stdout.write(json.dumps({"ok": ok, "checks": [asdict(c) for c in checks]}, ensure_ascii=False) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
