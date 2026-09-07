#!/usr/bin/env python3
"""Build the workspace-pinned Kotlin bindings and arm64 Android library."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import shutil

ROOT = Path(__file__).resolve().parents[2]


def run(*args: str, env: dict[str, str] | None = None) -> None:
    subprocess.run(args, cwd=ROOT, env=env, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--ndk-version", required=True)
    parser.add_argument("--kotlin-out", type=Path, required=True)
    parser.add_argument("--jni-out", type=Path, required=True)
    args = parser.parse_args()
    ndk = args.sdk / "ndk" / args.ndk_version
    if not (ndk / "source.properties").is_file():
        parser.error(f"install Android NDK {args.ndk_version} with the Android SDK manager")
    run("cargo", "+1.98.0", "build", "--locked", "-p", "zterm-android", "-p", "zterm-uniffi-bindgen")
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    library = target / "debug" / ("libzterm_android.dylib" if sys.platform == "darwin" else "libzterm_android.so")
    args.kotlin_out.mkdir(parents=True, exist_ok=True)
    run(str(target / "debug" / "zterm-uniffi-bindgen"), "generate", str(library),
        "--language", "kotlin", "--config", str(ROOT / "crates/android/uniffi.toml"),
        "--out-dir", str(args.kotlin_out), "--no-format")
    env = os.environ.copy()
    env["ANDROID_NDK_HOME"] = str(ndk)
    env["ANDROID_HOME"] = str(args.sdk)
    run("cargo", "+1.98.0", "ndk", "--target", "arm64-v8a", "--platform", "26",
        "build", "--locked", "--release", "-p", "zterm-android", env=env)
    # cargo-ndk's output copy can pick up stale dependency cdylibs in Cargo's
    # target directory. Package only this bridge; its Rust dependencies are static.
    destination = args.jni_out / "arm64-v8a"
    destination.mkdir(parents=True, exist_ok=True)
    for stale in destination.glob("*.so"):
        if stale.name != "libzterm_android.so":
            stale.unlink()
    shutil.copy2(target / "aarch64-linux-android/release/libzterm_android.so", destination / "libzterm_android.so")


if __name__ == "__main__":
    main()
