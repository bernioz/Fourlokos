"""Minimal OpenAI Images API client (standard library only, so it runs anywhere Python does).

The key is read from the OPENAI_API_KEY environment variable and is never written to disk,
printed, or included in error messages. OPENAI_BASE_URL overrides the API address (tests).
"""
from __future__ import annotations

import base64
import json
import mimetypes
import os
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

DEFAULT_BASE_URL = "https://api.openai.com/v1"
# Newest first. The docs name different models in different places, so the one used is the first of
# these that the account can see (override with --model or VOXEL_IMAGE_MODEL).
MODEL_PREFERENCE = (
    "gpt-image-2.6-flare",
    "gpt-image-2.5-flare",
    "gpt-image-2.6-sunburst",
    "gpt-image-2.5-sunburst",
    "gpt-image-2",
    "gpt-image-1.5",
    "gpt-image-1",
)
_TRANSIENT = (429, 500, 502, 503, 504)


class GenError(Exception):
    """An error with a stable machine-readable `code` for the JSON contract."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def base_url() -> str:
    return os.environ.get("OPENAI_BASE_URL", DEFAULT_BASE_URL).rstrip("/")


def api_key() -> str:
    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise GenError("missing_api_key", "OPENAI_API_KEY is not set (README, 'Modelos 3D', paso 4 de la instalación)")
    return key


def _error_from(e: urllib.error.HTTPError, key: str) -> GenError:
    try:
        err = json.loads(e.read()).get("error") or {}
    except Exception:
        err = {}
    code = err.get("code") or err.get("type") or ""
    message = (err.get("message") or e.reason or "request failed").replace(key, "***")
    if e.code == 401:
        return GenError("invalid_api_key", f"The API rejected the key: {message}")
    if code == "moderation_blocked":
        return GenError("moderation_blocked", f"The prompt or the image was blocked by moderation: {message}")
    if code == "insufficient_quota":
        return GenError("insufficient_quota", f"No API credit left: {message}")
    if e.code == 404 or code == "model_not_found":
        return GenError("model_not_found", f"Model not available to this account: {message}")
    return GenError("api_error", f"HTTP {e.code} {code}: {message}")


def request_json(method: str, path: str, key: str, *, body: bytes | None = None,
                 content_type: str = "application/json", timeout: float = 300) -> dict:
    """One API call with a few retries on rate limits and server errors."""
    headers = {"Authorization": f"Bearer {key}"}
    if body is not None:
        headers["Content-Type"] = content_type
    delay = 2.0
    for attempt in range(4):
        req = urllib.request.Request(base_url() + path, data=body, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            err = _error_from(e, key)
            if e.code in _TRANSIENT and err.code != "insufficient_quota" and attempt < 3:
                time.sleep(delay)
                delay *= 2
                continue
            raise err from None
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            if attempt < 3:
                time.sleep(delay)
                delay *= 2
                continue
            raise GenError("network_error", f"Could not reach the OpenAI API: {e}") from None
    raise GenError("api_error", "unreachable")


def list_models(key: str, timeout: float = 30) -> list[str]:
    data = request_json("GET", "/models", key, timeout=timeout)
    return sorted(m["id"] for m in data.get("data", []) if isinstance(m, dict) and "id" in m)


def pick_model(available: list[str]) -> str | None:
    for name in MODEL_PREFERENCE:
        if name in available:
            return name
    others = sorted((m for m in available if m.startswith("gpt-image")), reverse=True)
    return others[0] if others else None


def resolve_model(key: str, requested: str | None) -> str:
    if requested:
        return requested
    chosen = pick_model(list_models(key))
    if not chosen:
        raise GenError("no_image_model", "This account sees no gpt-image model (is the organization verified for image generation?)")
    return chosen


def _image_bytes(data: dict, key: str, timeout: float) -> bytes:
    items = data.get("data") or []
    if not items:
        raise GenError("api_error", "The API answered without an image")
    item = items[0]
    if item.get("b64_json"):
        return base64.b64decode(item["b64_json"])
    if item.get("url"):
        with urllib.request.urlopen(item["url"], timeout=timeout) as r:
            return r.read()
    raise GenError("api_error", "The API answered with neither b64_json nor url")


def generate(key: str, model: str, prompt: str, size: str, quality: str, timeout: float = 300) -> bytes:
    body = json.dumps({"model": model, "prompt": prompt, "size": size, "quality": quality, "n": 1}).encode()
    return _image_bytes(request_json("POST", "/images/generations", key, body=body, timeout=timeout), key, timeout)


def _multipart(fields: dict[str, str], files: list[tuple[str, Path]]) -> tuple[bytes, str]:
    boundary = uuid.uuid4().hex
    parts: list[bytes] = []
    for name, value in fields.items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode())
    for name, path in files:
        mime = mimetypes.guess_type(path.name)[0] or "image/png"
        parts.append((f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"; filename="{path.name}"\r\n'
                      f"Content-Type: {mime}\r\n\r\n").encode() + path.read_bytes() + b"\r\n")
    parts.append(f"--{boundary}--\r\n".encode())
    return b"".join(parts), f"multipart/form-data; boundary={boundary}"


def edit(key: str, model: str, prompt: str, image: Path, size: str, quality: str, timeout: float = 300) -> bytes:
    """A new image from an earlier one plus a change request (the refinement loop)."""
    body, ctype = _multipart({"model": model, "prompt": prompt, "size": size, "quality": quality, "n": "1"},
                             [("image[]", image)])
    return _image_bytes(request_json("POST", "/images/edits", key, body=body, content_type=ctype, timeout=timeout), key, timeout)


_MAGIC = ((b"\x89PNG", ".png"), (b"\xff\xd8\xff", ".jpg"), (b"GIF8", ".gif"))


def extension_of(data: bytes) -> str:
    for magic, ext in _MAGIC:
        if data.startswith(magic):
            return ext
    return ".webp" if data[:4] == b"RIFF" and data[8:12] == b"WEBP" else ".png"
