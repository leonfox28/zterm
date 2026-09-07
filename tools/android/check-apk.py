#!/usr/bin/env python3
"""Verify APK zip alignment and every packaged arm64 ELF load segment."""

import argparse
from pathlib import Path
import subprocess
import tempfile
import zipfile


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--ndk-version", default="28.2.13676358")
    args = parser.parse_args()
    readelf = next((args.sdk / "ndk" / args.ndk_version / "toolchains/llvm/prebuilt").glob("*/bin/llvm-readelf"))
    subprocess.run([str(args.sdk / "build-tools/36.0.0/zipalign"), "-c", "-P", "16", "4", str(args.apk)], check=True)
    with zipfile.ZipFile(args.apk) as archive, tempfile.TemporaryDirectory(prefix="zterm-elf-") as temporary:
        libraries = [name for name in archive.namelist() if name.startswith("lib/") and name.endswith(".so")]
        if "lib/arm64-v8a/libzterm_android.so" not in libraries or "lib/arm64-v8a/libjnidispatch.so" not in libraries:
            raise SystemExit("APK is missing the Rust library or Android JNA")
        for name in libraries:
            if not name.startswith("lib/arm64-v8a/"):
                raise SystemExit(f"Unexpected ABI: {name}")
            library = Path(temporary) / "library.so"
            library.write_bytes(archive.read(name))
            headers = subprocess.check_output([str(readelf), "--program-headers", "--wide", str(library)], text=True)
            segments = [line.split() for line in headers.splitlines() if line.strip().startswith("LOAD ")]
            if not segments or any(int(segment[-1], 16) < 16384 or
                    (int(segment[2], 16) - int(segment[1], 16)) % 16384 for segment in segments):
                raise SystemExit(f"ELF is not aligned for 16 KB pages: {name}")
            print(f"16 KB ELF: {name}")
    print("APK native-library and zip alignment verified")


if __name__ == "__main__":
    main()
