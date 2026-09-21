"""Offline regressions for original-draft recovery and its only write boundary."""

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("recover_draft", ROOT / "tools/release/recover-draft.py")
recovery = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(recovery)


def evidence():
    selection = {"tag": "v0.1.35", "run_id": 12, "artifact_id": 34, "release_id": 56}
    commit, annotation = "a" * 40, "b" * 40
    payloads = {"zterm-release.json": json.dumps({"tag": selection["tag"],
                                                 "source_commit": commit,
                                                 "classification": "stable"}).encode(),
                "payload.apk": b"original signed payload"}
    assets = [{"id": index + 1, "name": name, "size": len(value), "state": "uploaded",
               "digest": "sha256:" + hashlib.sha256(value).hexdigest()}
              for index, (name, value) in enumerate(payloads.items())]
    records = {
        "run": {"id": 12, "head_sha": commit, "event": "push", "head_branch": "v0.1.35",
                "path": ".github/workflows/release.yml", "status": "completed", "conclusion": "failure"},
        "reference": {"ref": "refs/tags/v0.1.35", "object": {"type": "tag", "sha": annotation}},
        "annotated": {"sha": annotation, "object": {"type": "commit", "sha": commit}},
        "artifact": {"id": 34, "name": "signed-release-v0.1.35-1", "expired": False,
                     "digest": "sha256:" + "c" * 64,
                     "workflow_run": {"id": 12, "head_sha": commit, "head_branch": "v0.1.35"}},
        "release": {"id": 56, "tag_name": "v0.1.35", "target_commitish": commit,
                    "draft": True, "immutable": False, "prerelease": False, "assets": assets,
                    "html_url": "https://github.com/leonfox28/zterm/releases/tag/v0.1.35"},
        "jobs": [{"name": name, "status": "completed", "conclusion": "success"}
                 for name in recovery.REQUIRED_JOBS]
                + [{"name": recovery.PUBLISH_JOB, "status": "completed", "conclusion": "failure"}],
    }
    return selection, records, payloads


class RecoveryTest(unittest.TestCase):
    def setUp(self):
        self.selection, self.records, self.payloads = evidence()
        self.plan = recovery.validate_evidence(self.selection, self.records)
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.signed = self.root / "signed"
        self.roundtrip = self.root / "roundtrip"
        self.write_inventory(self.signed)

    def write_inventory(self, directory):
        directory.mkdir(exist_ok=True)
        for name, value in self.payloads.items():
            (directory / name).write_bytes(value)

    def test_original_evidence_and_inventory_are_accepted(self):
        recovery.verify_files(self.plan, self.signed)
        recovery.check_release(self.plan, self.records["release"])
        self.assertEqual(self.plan["source_commit"], "a" * 40)
        self.plan["prerelease"] = True
        with self.assertRaisesRegex(ValueError, "classification"):
            recovery.verify_files(self.plan, self.signed)

    def test_wrong_or_missing_evidence_is_rejected(self):
        mutations = [
            ("run", "head_sha", "d" * 40), ("run", "id", 99),
            ("run", "event", "pull_request"), ("run", "head_branch", "v0.1.34"),
            ("run", "path", ".github/workflows/ci.yml"), ("run", "conclusion", "success"),
            ("artifact", "id", 99), ("artifact", "expired", True),
            ("artifact", "digest", None), ("artifact", "name", "unsigned"),
            ("release", "id", 99), ("release", "tag_name", "v0.1.34"),
            ("release", "target_commitish", "d" * 40), ("release", "draft", False),
            ("release", "immutable", True),
        ]
        for owner, field, value in mutations:
            with self.subTest(owner=owner, field=field):
                records = copy.deepcopy(self.records)
                records[owner][field] = value
                with self.assertRaises(ValueError):
                    recovery.validate_evidence(self.selection, records)
        for field in ("id", "head_sha", "head_branch"):
            records = copy.deepcopy(self.records)
            records["artifact"]["workflow_run"][field] = "wrong"
            with self.subTest(artifact_owner=field), self.assertRaises(ValueError):
                recovery.validate_evidence(self.selection, records)
        for name in recovery.REQUIRED_JOBS:
            records = copy.deepcopy(self.records)
            next(job for job in records["jobs"] if job["name"] == name)["conclusion"] = "failure"
            with self.subTest(job=name), self.assertRaises(ValueError):
                recovery.validate_evidence(self.selection, records)
        records = copy.deepcopy(self.records)
        records["reference"]["object"]["type"] = "commit"
        with self.assertRaises(ValueError):
            recovery.validate_evidence(self.selection, records)

    def test_asset_replacement_incomplete_and_corrupt_inventory_are_rejected(self):
        for field, value in (("id", 100), ("digest", "sha256:" + "f" * 64),
                             ("state", "starter"), ("name", "../escape")):
            records = copy.deepcopy(self.records["release"])
            records["assets"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                recovery.check_release(self.plan, records)
        payload = self.signed / "payload.apk"
        payload.write_bytes(b"corrupt")
        with self.assertRaisesRegex(ValueError, "bytes differ"):
            recovery.verify_files(self.plan, self.signed)
        payload.unlink()
        with self.assertRaisesRegex(ValueError, "incomplete"):
            recovery.verify_files(self.plan, self.signed)
        self.write_inventory(self.signed)
        (self.signed / "unexpected").write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            recovery.verify_files(self.plan, self.signed)

    def test_transient_download_uses_fresh_directories_and_preserves_original_assets(self):
        attempts = []

        def run(command, **kwargs):
            directory = Path(command[command.index("--dir") + 1])
            attempts.append(directory)
            self.assertEqual(list(directory.iterdir()), [])
            if len(attempts) == 1:
                (directory / "partial").write_bytes(b"partial download")
                return subprocess.CompletedProcess(command, 1, "", "HTTP 500 (GitHub asset)")
            self.write_inventory(directory)
            return subprocess.CompletedProcess(command, 0, "", "")

        with patch.object(recovery, "check_current") as current, \
                patch.object(recovery.subprocess, "run", side_effect=run), \
                patch.object(recovery.time, "sleep"):
            recovery.download(self.plan, self.signed, self.roundtrip)
        self.assertEqual(len(attempts), 2)
        self.assertNotEqual(attempts[0], attempts[1])
        self.assertEqual(current.call_count, 2)
        recovery.verify_files(self.plan, self.roundtrip)

    def test_permanent_and_exhausted_downloads_leave_no_completed_inventory(self):
        for error, expected_attempts in (("HTTP 403", 1), ("HTTP 500", 3)):
            with self.subTest(error=error), patch.object(recovery, "check_current"), \
                    patch.object(recovery.subprocess, "run", return_value=
                                 subprocess.CompletedProcess([], 1, "", error)) as run, \
                    patch.object(recovery.time, "sleep"), self.assertRaises(ValueError):
                recovery.download(self.plan, self.signed, self.roundtrip)
            self.assertEqual(run.call_count, expected_attempts)
            self.assertFalse(self.roundtrip.exists())
            self.assertEqual(list(self.root.iterdir()), [self.signed])

    def test_publish_only_updates_the_original_draft_after_rechecks(self):
        self.write_inventory(self.roundtrip)
        calls = []
        published = copy.deepcopy(self.records["release"])
        published.update(draft=False, immutable=True)

        def api(path, **kwargs):
            calls.append((path, kwargs))
            if path.startswith("git/ref/"):
                return self.records["reference"]
            if kwargs.get("data"):
                self.assertEqual(kwargs["data"], {"draft": False})
                return published
            return published if any(args.get("data") for _, args in calls) else self.records["release"]

        with patch.object(recovery, "api", side_effect=api):
            self.assertEqual(recovery.publish(self.plan, self.signed, self.roundtrip), published["html_url"])
        self.assertEqual([(path, args) for path, args in calls if args.get("data")],
                         [("releases/56", {"data": {"draft": False}})])
        for changed in ("tag", "asset"):
            with self.subTest(changed=changed):
                if changed == "tag":
                    self.records["reference"]["object"]["sha"] = "d" * 40
                else:
                    self.records["reference"]["object"]["sha"] = self.plan["tag_object"]
                    self.records["release"]["assets"][0]["id"] = 999
                calls.clear()
                with patch.object(recovery, "api", side_effect=api), self.assertRaises(ValueError):
                    recovery.publish(self.plan, self.signed, self.roundtrip)
                self.assertFalse(any(args.get("data") for _, args in calls))


if __name__ == "__main__":
    unittest.main()
