#!/usr/bin/env python3
"""voxel_doctor — check everything the voxel-model workflow needs. Stdout: one JSON object.

Exit 0 when every required check passes, 1 otherwise. A failing check carries a `fix` hint.
Checks: python version, OPENAI_API_KEY (set and accepted by the API, with an image model available),
the Blockbench MCP server, and Pillow (optional, only for reference_map.py).
Each check has a `stage`: `reference` (steps 1-3, needs only the OpenAI key) or `build` (step 6+, needs
Blockbench). `ready_for` says which stages can already run, so the agent can move on with what works.
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
    stage: str = "all"  # reference | build | all


KEY_FIX = ("Set OPENAI_API_KEY (README, 'Modelos 3D', paso 4), then open a new terminal. If it IS set in the "
           "human's own terminal, the agent is not inheriting it: Codex drops variables named *KEY* by default. Add "
           "`[shell_environment_policy]` with `ignore_default_excludes = true` to ~/.codex/config.toml and restart Codex")
NET_FIX = ("the agent's sandbox seems to block network access: add `[sandbox_workspace_write]` with "
           "`network_access = true` to ~/.codex/config.toml (or approve the command outside the sandbox) and restart Codex")


def check_python() -> Check:
    ok = sys.version_info >= (3, 9)
    return Check("python", ok, "" if ok else "Install Python 3.9 or newer from python.org", detail=sys.version.split()[0])


def check_openai() -> list[Check]:
    if not os.environ.get("OPENAI_API_KEY", "").strip():
        return [Check("openai_api_key", False, KEY_FIX, stage="reference")]
    key = os.environ["OPENAI_API_KEY"].strip()
    try:
        models = oi.list_models(key)
    except oi.GenError as e:
        fix = {"invalid_api_key": "The key was rejected: create a new one at platform.openai.com/api-keys",
               "network_error": f"Could not reach api.openai.com ({e}); if the human's terminal can, {NET_FIX}"}.get(e.code, str(e))
        return [Check("openai_api_key", False, fix, stage="reference")]
    model = oi.pick_model(models)
    return [Check("openai_api_key", True, stage="reference"),
            Check("openai_image_model", model is not None,
                  "" if model else "The key sees no gpt-image model: verify the organization at platform.openai.com/settings/organization/general",
                  detail=model or "", stage="reference")]


def check_blockbench(url: str) -> Check:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "voxel-doctor", "version": "0.2"}}}).encode()
    req = urllib.request.Request(url, data=body, method="POST", headers={
        "Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            if r.status == 200:
                return Check("blockbench_mcp", True, stage="build")
            reason = f"http {r.status}"
    except (urllib.error.URLError, OSError) as e:
        err = e.reason if isinstance(e, urllib.error.URLError) else e
        reason = "refused" if isinstance(err, ConnectionRefusedError) else (
            "blocked" if isinstance(err, PermissionError) else "unreachable")
    if reason == "blocked":
        fix = f"Could not connect to {url}: {NET_FIX}"
    else:
        fix = (f"Open Blockbench (desktop) and load the MCP plugin from {PLUGIN_URL} (File > Plugins > Load Plugin "
               f"from URL); expected at {url} ({reason}). Check `curl -s {url}` and the port in Blockbench Settings > General")
    return Check("blockbench_mcp", False, fix, detail=reason, stage="build")


def check_pillow() -> Check:
    try:
        import PIL  # noqa: F401
        return Check("pillow", True, optional=True)
    except ImportError:
        return Check("pillow", False, "Optional, only for reference_map.py: python -m pip install pillow", optional=True, stage="build")


def ready_for(checks: list[Check]) -> dict[str, bool]:
    req = [c for c in checks if not c.optional]
    return {"reference": all(c.ok for c in req if c.stage in ("reference", "all")),
            "build": all(c.ok for c in req)}


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    checks = [check_python(), *check_openai(), check_blockbench(os.environ.get("BLOCKBENCH_MCP_URL", DEFAULT_MCP_URL)), check_pillow()]
    ok = all(c.ok for c in checks if not c.optional)
    sys.stdout.write(json.dumps({"ok": ok, "ready_for": ready_for(checks), "checks": [asdict(c) for c in checks]}, ensure_ascii=False) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
