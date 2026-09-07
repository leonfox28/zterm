#!/usr/bin/env python3
"""Own Android release identity, candidate inspection and APK signing."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[2]
APK_NAME = "zterm-android-arm64.apk"
METADATA_NAME = "zterm-android.json"
PACKAGE = "io.github.leonfox28.zterm"


def version_code(version: str) -> int:
    """Reserve 100 builds per patch; prereleases precede the stable build."""
    match = re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-(alpha|beta|rc)\.([1-9][0-9]*))?", version)
    if not match:
        raise ValueError("Android releases require X.Y.Z or X.Y.Z-{alpha,beta,rc}.N")
    major, minor, patch = map(int, match.group(1, 2, 3))
    stage = 99
    if match[4]:
        ordinal = int(match[5])
        limit = 38 if match[4] == "rc" else 29
        if ordinal > limit:
            raise ValueError("Android prerelease ordinal exceeds its reserved range")
        stage = {"alpha": 0, "beta": 30, "rc": 60}[match[4]] + ordinal
    code = major * 10_000_000 + minor * 100_000 + patch * 100 + stage
    if minor > 99 or patch > 999 or not 1 <= code <= 2_100_000_000:
        raise ValueError("Android version exceeds the versionCode allocation")
    return code


def product_version() -> str:
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def run(*args: str) -> str:
    return subprocess.run(args, check=True, text=True, stdout=subprocess.PIPE).stdout


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def inspect(apk: Path, sdk: Path, commit: str, signed: bool) -> dict:
    version = product_version()
    expected = {"version": version, "version_code": version_code(version), "source_commit": commit}
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("A full source commit is required")
    with zipfile.ZipFile(apk) as archive:
        if json.loads(archive.read("assets/zterm-build.json")) != expected:
            raise ValueError("Packaged build identity differs from the release source")
    build_tools = sdk / "build-tools/36.0.0"
    badging = run(str(build_tools / "aapt2"), "dump", "badging", str(apk))
    package = re.search(r"^package: name='([^']+)' versionCode='([^']+)' versionName='([^']+)'", badging)
    if not package or package.groups() != (PACKAGE, str(expected["version_code"]), version):
        raise ValueError("APK package/version differs from the release identity")
    for required in ("minSdkVersion:'26'", "targetSdkVersion:'36'", "native-code: 'arm64-v8a'"):
        if required not in badging.splitlines():
            raise ValueError(f"Unexpected Android compatibility floor: {required}")
    subprocess.run(["python3", str(ROOT / "tools/android/check-apk.py"), str(apk), "--sdk", str(sdk)], check=True)
    certificate = (ROOT / "release/android-certificate.sha256").read_text().strip()
    if signed:
        signature = run(str(build_tools / "apksigner"), "verify", "--verbose", "--print-certs", str(apk))
        certs = re.findall(r"Signer #[0-9]+ certificate SHA-256 digest: ([0-9a-f]+)", signature)
        if certs != [certificate]:
            raise ValueError("APK signer differs from the retained release certificate")
    return {"schema": 1, "product": "zterm", **expected, "package": PACKAGE,
            "abi": "arm64-v8a", "min_sdk": 26, "target_sdk": 36,
            "certificate_sha256": certificate, "signed": signed,
            "length": apk.stat().st_size, "sha256": digest(apk)}


def prepare(apk: Path, output: Path, sdk: Path, commit: str) -> None:
    metadata = inspect(apk, sdk, commit, False)
    output.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(apk, output / APK_NAME)
    (output / METADATA_NAME).write_text(json.dumps(metadata, indent=2) + "\n")


def sign(directory: Path, sdk: Path) -> None:
    apk = directory / APK_NAME
    metadata_path = directory / METADATA_NAME
    metadata = json.loads(metadata_path.read_text())
    if metadata["signed"] or metadata != inspect(apk, sdk, metadata["source_commit"], False):
        raise ValueError("Unsigned APK candidate identity is invalid")
    # Secrets arrive only in the protected job; neither argv nor logs contain them.
    with tempfile.TemporaryDirectory(prefix="zterm-apk-sign-") as temporary:
        stage = Path(temporary)
        key, credential_file = stage / "key.p12", stage / "password"
        key.write_bytes(base64.b64decode(os.environ.pop("ZTERM_ANDROID_KEYSTORE_BASE64"), validate=True))
        credential_file.write_text(os.environ.pop("ZTERM_ANDROID_KEYSTORE_PASSWORD") + "\n")
        key.chmod(0o600)
        credential_file.chmod(0o600)
        signed_apk = stage / APK_NAME
        run(str(sdk / "build-tools/36.0.0/apksigner"), "sign", "--ks", str(key),
            "--ks-key-alias", "zterm", "--ks-pass", f"file:{credential_file}",
            "--v1-signing-enabled", "false", "--v2-signing-enabled", "true",
            "--v3-signing-enabled", "true", "--v4-signing-enabled", "false",
            "--out", str(signed_apk), str(apk))
        signed_metadata = inspect(signed_apk, sdk, metadata["source_commit"], True)
        # APK signing may change only signature records, never application payload.
        with zipfile.ZipFile(apk) as before, zipfile.ZipFile(signed_apk) as after:
            if before.namelist() != after.namelist() or any(before.read(name) != after.read(name) for name in before.namelist()):
                raise ValueError("Signing changed APK payload")
        shutil.copyfile(signed_apk, apk)
        metadata_path.write_text(json.dumps(signed_metadata, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["version-code", "prepare", "sign"])
    parser.add_argument("path")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--commit")
    parser.add_argument("--sdk", type=Path, default=os.environ.get("ANDROID_HOME", Path.home() / "Library/Android/sdk"))
    args = parser.parse_args()
    if args.command == "version-code":
        print(version_code(args.path))
    elif args.command == "prepare":
        if not args.output or not args.commit:
            parser.error("prepare requires --output and --commit")
        prepare(Path(args.path), args.output, args.sdk, args.commit)
    else:
        sign(Path(args.path), args.sdk)


if __name__ == "__main__":
    main()
