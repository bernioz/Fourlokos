"""Tests for the voxel-model scripts against a fake OpenAI API (no key, no network, no cost).

Run from anywhere:  python -m unittest discover -s .agents/skills/voxel-model/tests
"""
import base64
import contextlib
import io
import json
import os
import sys
import tempfile
import threading
import unittest
from unittest import mock
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS))
import generate_reference as gr  # noqa: E402
import openai_images as oi  # noqa: E402
import voxel_doctor as doctor  # noqa: E402

PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
KEY = "sk-test-secret-key"


class FakeApi(BaseHTTPRequestHandler):
    calls: list = []
    models = ["gpt-4o", "gpt-image-1", "gpt-image-2.5-flare"]
    fail: tuple | None = None  # (status, body dict) for the next image call

    def log_message(self, *a):
        pass

    def _send(self, status, obj):
        data = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _handle(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length)
        FakeApi.calls.append((self.command, self.path, self.headers.get("Authorization"), body))
        if self.headers.get("Authorization") != f"Bearer {KEY}":
            return self._send(401, {"error": {"message": "Incorrect API key provided", "code": "invalid_api_key"}})
        if self.path == "/models":
            return self._send(200, {"data": [{"id": m} for m in FakeApi.models]})
        if FakeApi.fail:
            status, obj = FakeApi.fail
            FakeApi.fail = None
            return self._send(status, obj)
        if self.path in ("/images/generations", "/images/edits"):
            return self._send(200, {"data": [{"b64_json": base64.b64encode(PNG).decode()}]})
        self._send(404, {"error": {"message": "nope"}})

    do_GET = do_POST = _handle


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = HTTPServer(("127.0.0.1", 0), FakeApi)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.url = f"http://127.0.0.1:{cls.server.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def setUp(self):
        FakeApi.calls = []
        FakeApi.fail = None
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name) / "art"
        self._env = {k: os.environ.get(k) for k in ("OPENAI_API_KEY", "OPENAI_BASE_URL", "VOXEL_IMAGE_MODEL")}
        os.environ["OPENAI_BASE_URL"] = self.url
        os.environ["OPENAI_API_KEY"] = KEY
        os.environ.pop("VOXEL_IMAGE_MODEL", None)
        self.addCleanup(self._restore)

    def _restore(self):
        for k, v in self._env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def run_cli(self, *argv):
        buf, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(err):
            code = gr.main([*argv, "--out-root", str(self.out)])
        lines = buf.getvalue().strip().splitlines()
        self.assertEqual(len(lines), 1, f"stdout must be exactly one JSON object, got {lines}")
        self.assertNotIn(KEY, buf.getvalue() + err.getvalue(), "the key leaked")
        return code, json.loads(lines[0])


class GenerateTests(Base):
    def test_dry_run_spends_nothing(self):
        code, out = self.run_cli("a moss golem", "--dry-run")
        self.assertEqual(code, 0)
        self.assertTrue(out["dry_run"])
        self.assertIn("moss golem", out["prompt"])
        self.assertEqual(FakeApi.calls, [])
        self.assertFalse(self.out.exists(), "a dry run must not create folders")

    def test_generate_picks_best_model_and_writes_draft(self):
        code, out = self.run_cli("A Moss Golem!")
        self.assertEqual(code, 0, out)
        self.assertEqual(out["model"], "gpt-image-2.5-flare")
        self.assertEqual(out["slug"], "a-moss-golem")
        path = Path(out["image_path"])
        self.assertEqual(path, self.out / "a-moss-golem" / "drafts" / "reference-v1.png")
        self.assertEqual(path.read_bytes(), PNG)
        sent = json.loads([c for c in FakeApi.calls if c[1] == "/images/generations"][0][3])
        self.assertEqual(sent["model"], "gpt-image-2.5-flare")
        self.assertIn("moss golem", sent["prompt"].lower())

    def test_model_override(self):
        code, out = self.run_cli("a slime", "--model", "gpt-image-1")
        self.assertEqual(out["model"], "gpt-image-1")
        self.assertFalse([c for c in FakeApi.calls if c[1] == "/models"])

    def test_refine_uploads_previous_image_and_bumps_version(self):
        _, first = self.run_cli("a slime")
        code, second = self.run_cli("a slime", "--continue", first["image_path"], "--feedback", "add a hat")
        self.assertEqual(code, 0, second)
        self.assertTrue(second["image_path"].endswith("reference-v2.png"))
        edit = [c for c in FakeApi.calls if c[1] == "/images/edits"][0]
        self.assertIn(b'name="image[]"', edit[3])
        self.assertIn(PNG, edit[3])
        self.assertIn(b"add a hat", edit[3])

    def test_continue_needs_feedback(self):
        _, first = self.run_cli("a slime")
        code, out = self.run_cli("a slime", "--continue", first["image_path"])
        self.assertEqual((code, out["error"]), (2, "missing_feedback"))

    def test_approve_copies_draft(self):
        _, first = self.run_cli("a slime")
        code, out = self.run_cli("--approve", first["image_path"])
        self.assertEqual(code, 0, out)
        self.assertEqual(Path(out["image_path"]), self.out / "a-slime" / "reference.png")
        self.assertEqual(Path(out["image_path"]).read_bytes(), PNG)


class ErrorTests(Base):
    def test_missing_key(self):
        os.environ.pop("OPENAI_API_KEY")
        code, out = self.run_cli("a slime")
        self.assertEqual((code, out["error"]), (2, "missing_api_key"))

    def test_wrong_key(self):
        os.environ["OPENAI_API_KEY"] = "sk-wrong"
        code, out = self.run_cli("a slime")
        self.assertEqual((code, out["error"]), (2, "invalid_api_key"))
        self.assertNotIn("sk-wrong", out["message"])

    def test_moderation_block(self):
        FakeApi.fail = (400, {"error": {"code": "moderation_blocked", "message": "blocked"}})
        code, out = self.run_cli("a slime", "--model", "gpt-image-1")
        self.assertEqual((code, out["error"]), (2, "moderation_blocked"))

    def test_no_image_model(self):
        FakeApi.models = ["gpt-4o"]
        self.addCleanup(setattr, FakeApi, "models", ["gpt-4o", "gpt-image-1", "gpt-image-2.5-flare"])
        code, out = self.run_cli("a slime")
        self.assertEqual((code, out["error"]), (2, "no_image_model"))


class DoctorTests(Base):
    def test_openai_checks_pass_with_good_key(self):
        checks = doctor.check_openai()
        self.assertEqual([(c.name, c.ok) for c in checks], [("openai_api_key", True), ("openai_image_model", True)])
        self.assertEqual(checks[1].detail, "gpt-image-2.5-flare")

    def test_openai_check_fails_without_key(self):
        os.environ.pop("OPENAI_API_KEY")
        self.assertFalse(doctor.check_openai()[0].ok)

    def test_blockbench_check_reports_closed_port(self):
        c = doctor.check_blockbench("http://127.0.0.1:9/bb-mcp")
        self.assertFalse(c.ok)
        self.assertEqual(c.detail, "refused")
        self.assertIn("Open Blockbench", c.fix)

    def test_blockbench_sandbox_block_is_not_reported_as_closed(self):
        with mock.patch.object(doctor.urllib.request, "urlopen", side_effect=PermissionError("denied")):
            c = doctor.check_blockbench("http://127.0.0.1:3000/bb-mcp")
        self.assertEqual(c.detail, "blocked")
        self.assertIn("network_access", c.fix)

    def test_missing_key_fix_explains_agent_env_filtering(self):
        os.environ.pop("OPENAI_API_KEY")
        fix = doctor.check_openai()[0].fix
        self.assertIn("ignore_default_excludes", fix)

    def test_checks_carry_stage_and_ready_for(self):
        self.assertEqual(doctor.check_openai()[0].stage, "reference")
        self.assertEqual(doctor.check_blockbench("http://127.0.0.1:9/bb-mcp").stage, "build")
        checks = [doctor.Check("a", True, stage="reference"), doctor.Check("b", False, stage="build")]
        self.assertEqual(doctor.ready_for(checks), {"reference": True, "build": False})


class PureTests(unittest.TestCase):
    def test_slugify(self):
        self.assertEqual(gr.slugify("Un Golem de Musgo!"), "un-golem-de-musgo")
        self.assertEqual(gr.slugify("¿¡"), "model")
        self.assertLessEqual(len(gr.slugify("x" * 200)), 48)

    def test_pick_model_prefers_newest_flare(self):
        self.assertEqual(oi.pick_model(["gpt-image-1", "gpt-image-2.6-flare"]), "gpt-image-2.6-flare")
        self.assertIsNone(oi.pick_model(["gpt-4o"]))


if __name__ == "__main__":
    unittest.main()
