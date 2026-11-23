#!/usr/bin/env python3
"""generate_reference — idea in, reference image out (OpenAI Images API). Stdout: exactly one JSON object.

  generate_reference.py "a moss golem"                      new image  -> art/a-moss-golem/drafts/reference-v1.png
  generate_reference.py "a moss golem" --continue art/a-moss-golem/drafts/reference-v1.png \\
                        --feedback "add a red scarf"        refine an earlier draft -> reference-v2.png
  generate_reference.py --approve art/a-moss-golem/drafts/reference-v2.png
                                                            copy it to art/a-moss-golem/reference.png

Needs the OPENAI_API_KEY environment variable. Env: VOXEL_IMAGE_MODEL, OPENAI_BASE_URL.
Exit codes: 0 ok, 2 error (see the "error" field of the JSON).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
import unicodedata
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import openai_images as oi  # noqa: E402

TEMPLATE = Path(__file__).resolve().parents[1] / "prompt-template.txt"
DEFAULT_SIZE = "1536x1024"
DEFAULT_QUALITY = "medium"
_VERSION_RE = re.compile(r"^reference-v(\d+)\.(png|jpe?g|webp|gif)$")


def slugify(subject: str) -> str:
    ascii_text = unicodedata.normalize("NFKD", subject).encode("ascii", "ignore").decode()
    slug = re.sub(r"[^a-z0-9]+", "-", ascii_text.lower()).strip("-")[:48].strip("-")
    return slug or "model"


def build_prompt(subject: str) -> str:
    return TEMPLATE.read_text(encoding="utf-8").strip().replace("[SUBJECT]", subject.strip())


def refine_prompt(feedback: str) -> str:
    return (f"{feedback.strip()}\nKeep everything else the same: the same subject, the same turnaround layout "
            "(front, back and isometric views), the same blocky voxel style and the same plain dark gray background.")


def next_draft_path(out_root: Path, slug: str, create: bool = True) -> Path:
    folder = out_root / slug / "drafts"
    if create:
        folder.mkdir(parents=True, exist_ok=True)
    existing = folder.iterdir() if folder.is_dir() else ()
    versions = [int(m.group(1)) for p in existing if (m := _VERSION_RE.match(p.name))]
    return folder / f"reference-v{max(versions, default=0) + 1}.png"


def slug_of_draft(path: Path) -> str | None:
    """`.../art/<slug>/drafts/reference-vN.png` -> `<slug>`."""
    return path.resolve().parent.parent.name if path.resolve().parent.name == "drafts" else None


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="generate_reference", description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("subject", nargs="?", help="what to model, e.g. 'a moss golem'")
    p.add_argument("--continue", dest="previous", type=Path, default=None,
                   help="a draft image to refine (needs --feedback)")
    p.add_argument("--feedback", default=None, help="the change request, in English; required with --continue")
    p.add_argument("--approve", type=Path, default=None, help="copy this draft to art/<slug>/reference.<ext>")
    p.add_argument("--out-root", type=Path, default=Path("art"), help="default: ./art (run from the repo root)")
    p.add_argument("--model", default=os.environ.get("VOXEL_IMAGE_MODEL"), help="default: best gpt-image model the key can use")
    p.add_argument("--size", default=DEFAULT_SIZE)
    p.add_argument("--quality", default=DEFAULT_QUALITY, help="low, medium, high, ... (higher costs more)")
    p.add_argument("--timeout", type=int, default=300, help="seconds to wait for the API")
    p.add_argument("--dry-run", action="store_true", help="print what would be sent; call nothing, spend nothing")
    return p


def _emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def approve(args: argparse.Namespace) -> dict:
    src = args.approve
    if not src.is_file():
        raise oi.GenError("file_not_found", f"{src} does not exist")
    slug = slug_of_draft(src) or slugify(args.subject or src.stem)
    dest = args.out_root / slug / f"reference{src.suffix.lower()}"
    dest.parent.mkdir(parents=True, exist_ok=True)
    for old in dest.parent.glob("reference.*"):
        old.unlink()
    shutil.copyfile(src, dest)
    return {"ok": True, "action": "approve", "image_path": str(dest), "slug": slug}


def generate(args: argparse.Namespace) -> dict:
    if not args.subject:
        raise oi.GenError("missing_subject", "give the subject to model (or --approve <draft>)")
    if args.previous and not args.feedback:
        raise oi.GenError("missing_feedback", "--continue needs --feedback with the change request")
    if args.previous and not args.previous.is_file():
        raise oi.GenError("file_not_found", f"{args.previous} does not exist")
    slug = (slug_of_draft(args.previous) if args.previous else None) or slugify(args.subject)
    prompt = refine_prompt(args.feedback) if args.previous else build_prompt(args.subject)
    out = next_draft_path(args.out_root, slug, create=not args.dry_run)
    if args.dry_run:
        return {"ok": True, "dry_run": True, "slug": slug, "prompt": prompt, "would_write": str(out),
                "mode": "edit" if args.previous else "generate", "size": args.size, "quality": args.quality,
                "model": args.model or "(best available)"}
    key = oi.api_key()
    model = oi.resolve_model(key, args.model)
    if args.previous:
        data = oi.edit(key, model, prompt, args.previous, args.size, args.quality, args.timeout)
    else:
        data = oi.generate(key, model, prompt, args.size, args.quality, args.timeout)
    out = out.with_suffix(oi.extension_of(data))
    out.write_bytes(data)
    return {"ok": True, "image_path": str(out), "session_id": str(out), "slug": slug,
            "backend": "openai-api", "model": model, "prompt": prompt}


def main(argv: list[str] | None = None) -> int:
    if hasattr(sys.stdout, "reconfigure"):  # Windows consoles default to a legacy code page
        sys.stdout.reconfigure(encoding="utf-8")
    args = _parser().parse_args(argv)
    try:
        _emit(approve(args) if args.approve else generate(args))
        return 0
    except oi.GenError as e:
        _emit({"ok": False, "error": e.code, "message": str(e)})
        return 2
    except Exception as e:  # keep the one-JSON-object contract even on bugs
        import traceback
        traceback.print_exc(file=sys.stderr)
        _emit({"ok": False, "error": "internal_error", "message": f"{type(e).__name__}: {e}"})
        return 2


if __name__ == "__main__":
    sys.exit(main())
