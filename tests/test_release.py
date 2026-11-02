"""Exercise release publication with fake gh responses; no network requests."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
COMMIT = "a" * 40
BINARIES = (
    "acp-traces-linux-amd64",
    "acp-traces-linux-arm64",
    "acp-traces-macos-amd64",
    "acp-traces-macos-arm64",
    "acp-traces-windows-amd64.exe",
    "acp-traces-windows-arm64.exe",
)
FAKE_GH = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
log = pathlib.Path(os.environ["CALL_LOG"])
calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
with log.open("a") as output:
    output.write(json.dumps(args) + "\n")
if "--slurp" in args and "--jq" in args:
    print("gh: --slurp is not supported with --jq", file=sys.stderr)
    sys.exit(2)
scenario = os.environ["SCENARIO"]
if "--paginate" in args:
    if scenario == "lookup-error":
        sys.exit(1)
    if scenario == "existing-release":
        print("7")
elif "repos/owner/repo/commits/v0.3.1" in args:
    prior = sum("repos/owner/repo/commits/v0.3.1" in call for call in calls)
    moved = scenario == "moved-before" or (scenario == "moved-after" and prior > 0)
    print(("b" if moved else "a") * 40)
elif "--method" in args and args[args.index("--method") + 1] == "POST":
    if any("uploads.github.com" in value for value in args):
        if scenario == "upload-error":
            sys.exit(1)
        print("{}")
    else:
        print("invalid" if scenario == "invalid-id" else "42")
elif "--method" in args and args[args.index("--method") + 1] == "PATCH":
    print("{}")
else:
    sys.exit(9)
'''


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.assets = self.directory / "assets"
        self.assets.mkdir()
        manifest = []
        for name in BINARIES:
            contents = f"fixture for {name}".encode()
            (self.assets / name).write_bytes(contents)
            manifest.append(f"{hashlib.sha256(contents).hexdigest()}  {name}")
        (self.assets / "SHA256SUMS").write_text("\n".join(manifest) + "\n")
        self.bin = self.directory / "bin"
        self.bin.mkdir()
        gh = self.bin / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.log = self.directory / "calls.jsonl"

    def run_publication(self, scenario="success"):
        env = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "GH_REPO": "owner/repo",
            "RELEASE_TAG": "v0.3.1",
            "RELEASE_COMMIT": COMMIT,
            "CALL_LOG": str(self.log),
            "SCENARIO": scenario,
        }
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/publish-release.sh"), str(self.assets)],
            cwd=ROOT, env=env, text=True, capture_output=True,
        )
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def test_success_uploads_complete_set_and_publishes_exact_draft_id(self):
        result, calls = self.run_publication()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            calls[0],
            ["api", "--paginate", "repos/owner/repo/releases", "--jq",
             '.[] | select(.tag_name == "v0.3.1") | .id'],
        )
        uploads = [call for call in calls if any("uploads.github.com" in value for value in call)]
        self.assertEqual(len(uploads), 7)
        names = {next(value for value in call if "uploads.github.com" in value).split("?name=")[1] for call in uploads}
        self.assertEqual(names, set(BINARIES) | {"SHA256SUMS"})
        self.assertTrue(all("/releases/42/assets?" in " ".join(call) for call in uploads))
        self.assertIn("draft=true", " ".join(calls[2]))
        self.assertIn("repos/owner/repo/releases/42", calls[-1])
        self.assertIn("PATCH", calls[-1])
        self.assertIn("draft=false", calls[-1])

    def test_existing_release_lookup_error_or_moved_tag_prevents_creation(self):
        for scenario in ["existing-release", "lookup-error", "moved-before"]:
            with self.subTest(scenario=scenario):
                self.log.unlink(missing_ok=True)
                result, calls = self.run_publication(scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any("--method" in call for call in calls))

    def test_upload_failure_moved_tag_or_invalid_id_never_publishes(self):
        for scenario in ["upload-error", "moved-after", "invalid-id"]:
            with self.subTest(scenario=scenario):
                self.log.unlink(missing_ok=True)
                result, calls = self.run_publication(scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any("PATCH" in call for call in calls))

    def test_missing_or_corrupt_asset_fails_before_any_api_call(self):
        binary = self.assets / BINARIES[0]
        binary.unlink()
        result, calls = self.run_publication()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])
        binary.write_text("corrupt")
        result, calls = self.run_publication()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
