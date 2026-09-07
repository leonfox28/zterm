#!/usr/bin/env python3
"""Build and verify a locally signed arm64 acceptance APK with a durable external key."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def run(*command: str, capture: bool = False) -> str:
    return subprocess.run(command, cwd=ROOT, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout or ""


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version_code", type=int)
    parser.add_argument("--sdk", type=Path, default=os.environ.get("ANDROID_HOME", Path.home() / "Library/Android/sdk"))
    parser.add_argument("--signing-dir", type=Path, default=Path.home() / ".local/share/zterm/android-signing")
    args = parser.parse_args()
    if not 1 <= args.version_code <= 2_100_000_000:
        parser.error("version_code must be a positive Android versionCode")
    signing = args.signing_dir.expanduser().resolve()
    if signing.is_relative_to(ROOT):
        parser.error("signing material must remain outside the checkout")
    signing.mkdir(parents=True, exist_ok=True, mode=0o700)
    keystore, password_file = signing / "acceptance.p12", signing / "password"
    if keystore.exists() != password_file.exists():
        raise SystemExit("Incomplete signing identity; restore its missing file before continuing")
    java = os.environ.get("JAVA_HOME")
    if not java and os.uname().sysname == "Darwin":
        java = run("/usr/libexec/java_home", "-v", "17", capture=True).strip()
    keytool = str(Path(java) / "bin/keytool") if java else "keytool"
    if not keystore.exists():
        with tempfile.TemporaryDirectory(dir=signing, prefix="new-key-") as temporary:
            stage = Path(temporary)
            secret_file = stage / "password"
            secret_file.write_text(secrets.token_urlsafe(48) + "\n")
            secret_file.chmod(0o600)
            run(keytool, "-genkeypair", "-noprompt", "-storetype", "PKCS12", "-keystore", str(stage / "key.p12"),
                "-storepass:file", str(secret_file), "-alias", "zterm", "-keyalg", "RSA", "-keysize", "3072",
                "-validity", "10000", "-dname", "CN=zterm local acceptance")
            (stage / "key.p12").chmod(0o600)
            os.link(stage / "key.p12", keystore)
            os.link(secret_file, password_file)
    previous = signing / "last-version-code"
    if previous.exists() and args.version_code <= int(previous.read_text()):
        raise SystemExit("Use a versionCode greater than the last signed build")
    run("sh", "tools/android/build.sh", ":app:assembleRelease", f"-PztermVersionCode={args.version_code}")
    output = ROOT / "target/android-apk" / str(args.version_code)
    output.mkdir(parents=True, exist_ok=False)
    apk = output / f"zterm-android-arm64-{args.version_code}.apk"
    tools = args.sdk / "build-tools/36.0.0"
    env_java = os.environ.get("JAVA_HOME")
    if java and not env_java:
        os.environ["JAVA_HOME"] = java
    run(str(tools / "apksigner"), "sign", "--ks", str(keystore), "--ks-key-alias", "zterm",
        "--ks-pass", f"file:{password_file}", "--out", str(apk),
        str(ROOT / "apps/android/app/build/outputs/apk/release/app-release-unsigned.apk"))
    signature = run(str(tools / "apksigner"), "verify", "--verbose", "--print-certs", str(apk), capture=True)
    run("python3", "tools/android/check-apk.py", str(apk), "--sdk", str(args.sdk))
    with apk.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    (output / "SHA256SUMS").write_text(f"{digest}  {apk.name}\n")
    (output / "signature.txt").write_text(signature)
    (output / "build.json").write_text(json.dumps({
        "versionCode": args.version_code,
        "commit": run("git", "rev-parse", "HEAD", capture=True).strip(),
        "dirty": bool(run("git", "status", "--porcelain", capture=True)),
        "sha256": digest, "abi": "arm64-v8a", "minSdk": 26, "targetSdk": 36,
    }, indent=2) + "\n")
    previous.write_text(str(args.version_code) + "\n")
    print(f"Verified APK: {apk}\nSHA-256: {digest}")


if __name__ == "__main__":
    main()
