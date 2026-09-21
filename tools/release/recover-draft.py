#!/usr/bin/env python3
"""Resume one existing signed release draft without replacing public state."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time


REPOSITORY = "leonfox28/zterm"
REQUIRED_JOBS = {
    "Validate exact immutable release source",
    "Approve and sign exact manifest bytes",
    "Install aarch64-apple-darwin from local HTTPS",
    "Install aarch64-unknown-linux-gnu from local HTTPS",
    "Install x86_64-unknown-linux-gnu from local HTTPS",
}
PUBLISH_JOB = "Create, verify, attest, and publish the immutable Release"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def api(path, *, pages=False, data=None):
    command = ["gh", "api", "-H", "Accept: application/vnd.github+json",
               "-H", "X-GitHub-Api-Version: 2022-11-28"]
    if pages:
        command += ["--paginate", "--slurp"]
    if data is not None:
        command += ["--method", "PATCH", "--input", "-"]
    command.append(f"repos/{REPOSITORY}/{path}")
    result = subprocess.run(command, input=None if data is None else json.dumps(data),
                            capture_output=True, text=True, check=True, timeout=120)
    return json.loads(result.stdout)


def assets(release):
    records = []
    for asset in release["assets"]:
        name = asset["name"]
        require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name), "unsafe asset name")
        require(type(asset["id"]) is int and asset["id"] > 0, "invalid asset ID")
        require(type(asset["size"]) is int and asset["size"] > 0, "empty asset")
        require(asset["state"] == "uploaded", "asset upload is incomplete")
        require(re.fullmatch(r"sha256:[0-9a-f]{64}", asset.get("digest") or ""),
                "asset has no server digest")
        records.append({key: asset[key] for key in ("id", "name", "size", "digest", "state")})
    require(records and len({a["name"] for a in records}) == len(records),
            "empty or duplicate asset inventory")
    require(len({a["id"] for a in records}) == len(records), "duplicate asset IDs")
    return sorted(records, key=lambda asset: asset["name"])


def validate_evidence(selection, records):
    tag, run_id, artifact_id, release_id = (
        selection[key] for key in ("tag", "run_id", "artifact_id", "release_id"))
    run, reference, annotated, artifact, release, jobs = (
        records[key] for key in ("run", "reference", "annotated", "artifact", "release", "jobs"))
    commit = run["head_sha"]
    require(re.fullmatch(r"[0-9a-f]{40}", commit), "invalid source commit")
    require(run["id"] == run_id and run["event"] == "push" and run["head_branch"] == tag
            and run["path"] == ".github/workflows/release.yml"
            and run["status"] == "completed" and run["conclusion"] == "failure",
            "run is not the failed original tag publication")
    require(reference["ref"] == f"refs/tags/{tag}" and reference["object"]["type"] == "tag",
            "release tag is not annotated")
    require(annotated["sha"] == reference["object"]["sha"]
            and annotated["object"]["type"] == "commit" and annotated["object"]["sha"] == commit,
            "annotated tag does not identify the original source")
    for name in REQUIRED_JOBS:
        matching = [job for job in jobs if job["name"] == name]
        require(len(matching) == 1 and matching[0]["status"] == "completed"
                and matching[0]["conclusion"] == "success", f"missing successful evidence: {name}")
    publish = [job for job in jobs if job["name"] == PUBLISH_JOB]
    require(len(publish) == 1 and publish[0]["conclusion"] == "failure",
            "original publication did not fail")
    owner = artifact["workflow_run"]
    require(artifact["id"] == artifact_id and artifact["expired"] is False
            and owner["id"] == run_id and owner["head_sha"] == commit
            and owner["head_branch"] == tag
            and re.fullmatch(rf"signed-release-{re.escape(tag)}-[1-9][0-9]*", artifact["name"])
            and re.fullmatch(r"sha256:[0-9a-f]{64}", artifact.get("digest") or ""),
            "signed artifact is missing, expired or from a different run/source")
    require(release["id"] == release_id and release["tag_name"] == tag
            and release["target_commitish"] == commit and release["draft"] is True
            and release["immutable"] is False, "release is not the original unpublished draft")
    return {**selection, "source_commit": commit, "tag_object": annotated["sha"],
            "prerelease": release["prerelease"], "assets": assets(release)}


def inspect(selection):
    tag = selection["tag"]
    require(re.fullmatch(r"v[0-9][A-Za-z0-9.-]*", tag), "invalid tag selector")
    reference = api(f"git/ref/tags/{tag}")
    require(reference["object"]["type"] == "tag", "release tag must be annotated")
    records = {
        "reference": reference,
        "annotated": api(f"git/tags/{reference['object']['sha']}"),
        "run": api(f"actions/runs/{selection['run_id']}"),
        "artifact": api(f"actions/artifacts/{selection['artifact_id']}"),
        "release": api(f"releases/{selection['release_id']}"),
        "jobs": [job for page in api(
            f"actions/runs/{selection['run_id']}/jobs?filter=latest&per_page=100", pages=True)
                 for job in page["jobs"]],
    }
    return validate_evidence(selection, records)


def check_release(plan, release, *, published=False):
    require(release["id"] == plan["release_id"] and release["tag_name"] == plan["tag"]
            and release["target_commitish"] == plan["source_commit"]
            and release["prerelease"] == plan["prerelease"]
            and release["draft"] is (not published) and release["immutable"] is published,
            "release identity or publication state changed")
    require(assets(release) == plan["assets"], "draft asset identities or digests changed")


def check_current(plan):
    reference = api(f"git/ref/tags/{plan['tag']}")
    require(reference["object"]["type"] == "tag"
            and reference["object"]["sha"] == plan["tag_object"], "tag changed during recovery")
    check_release(plan, api(f"releases/{plan['release_id']}"))


def verify_files(plan, directory):
    files = list(directory.iterdir())
    require(all(path.is_file() and not path.is_symlink() for path in files),
            "inventory contains a non-regular file")
    require({path.name for path in files} == {asset["name"] for asset in plan["assets"]},
            "asset inventory is incomplete or contains unexpected files")
    for asset in plan["assets"]:
        path = directory / asset["name"]
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        require(path.stat().st_size == asset["size"] and f"sha256:{digest}" == asset["digest"],
                f"asset bytes differ: {asset['name']}")
    manifest = json.loads((directory / "zterm-release.json").read_text())
    require(manifest["tag"] == plan["tag"] and manifest["source_commit"] == plan["source_commit"],
            "signed inventory belongs to a different tag/source")
    require(manifest["classification"] in ("stable", "prerelease")
            and (manifest["classification"] == "prerelease") is plan["prerelease"],
            "draft classification differs from the signed release")


def download(plan, signed, destination):
    verify_files(plan, signed)
    check_current(plan)
    require(not destination.exists(), "download destination already exists")
    for attempt in range(3):
        with tempfile.TemporaryDirectory(dir=destination.parent, prefix="draft-download-") as temporary:
            try:
                result = subprocess.run(
                    ["gh", "release", "download", plan["tag"], "--repo", REPOSITORY,
                     "--dir", temporary, "--pattern", "*"],
                    capture_output=True, text=True, timeout=180)
                if result.returncode == 0:
                    verify_files(plan, Path(temporary))
                    check_current(plan)
                    Path(temporary).rename(destination)
                    return
                transient = re.search(r"HTTP (429|500|502|503|504)\b", result.stderr)
                require(transient is not None, "draft download failed with a permanent error")
            except subprocess.TimeoutExpired:
                pass
        require(attempt < 2, "draft download failed after three transient attempts")
        time.sleep(2 ** attempt)


def publish(plan, signed, roundtrip):
    verify_files(plan, signed)
    verify_files(plan, roundtrip)
    check_current(plan)
    # The workflow's successful provenance step is required before this command.
    api(f"releases/{plan['release_id']}", data={"draft": False})
    result = api(f"releases/{plan['release_id']}")
    check_release(plan, result, published=True)
    return result["html_url"]


def positive_id(value):
    require(value.isdecimal() and int(value) > 0, "GitHub IDs must be positive integers")
    return int(value)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("inspect")
    prepare.add_argument("--tag", required=True)
    for name in ("run-id", "artifact-id", "release-id"):
        prepare.add_argument(f"--{name}", required=True, type=positive_id)
    prepare.add_argument("--plan", required=True, type=Path)
    for name in ("download", "publish"):
        command = commands.add_parser(name)
        command.add_argument("--plan", required=True, type=Path)
        command.add_argument("--signed", required=True, type=Path)
        command.add_argument("--roundtrip", required=True, type=Path)
    args = parser.parse_args()
    if args.command == "inspect":
        selection = {key: getattr(args, key) for key in ("tag", "run_id", "artifact_id", "release_id")}
        plan = inspect(selection)
        args.plan.write_text(json.dumps(plan, indent=2) + "\n")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.write(f"source_commit={plan['source_commit']}\n")
        print(f"Verified original publication evidence for {plan['tag']} at {plan['source_commit']}")
    else:
        plan = json.loads(args.plan.read_text())
        if args.command == "download":
            download(plan, args.signed, args.roundtrip)
            print("Draft round-trip matches the original signed inventory")
        else:
            url = publish(plan, args.signed, args.roundtrip)
            print(f"Published immutable Release: {url}")
            if os.environ.get("GITHUB_STEP_SUMMARY"):
                with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
                    summary.write(f"Recovered original draft {plan['release_id']}: {url}\n")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(f"draft recovery failed: {error}") from error
