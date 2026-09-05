"""Observe the v0.1.16 failure without changing any production source file."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib


def main():
    research = Path(__file__).resolve().parent
    root = next(parent for parent in research.parents if (parent / "Cargo.toml").is_file())
    output = root / "target" / "terminal-sync-scroll"
    output.mkdir(parents=True, exist_ok=True)
    build = subprocess.run(
        ["cargo", "+1.98.0", "test", "-p", "zterm-cli", "--lib", "--no-run",
         "--locked", "--offline", "--all-features", "--message-format=json"],
        cwd=root, capture_output=True, text=True, check=True,
    )
    (output / "build.jsonl").write_text(build.stdout)
    (output / "build.log").write_text(build.stderr)
    libraries = {}
    native_paths = set()
    for line in build.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact":
            if item["target"]["name"] == "rustix" and "event" not in item["features"]:
                continue  # Do not select the build-script-only feature instance.
            for filename in item["filenames"]:
                if filename.endswith(".rlib"):
                    libraries[item["target"]["name"]] = filename
        elif item.get("reason") == "build-script-executed":
            native_paths.update(item["linked_paths"])
    copied = output / "cli-source-copy"
    shutil.copytree(root / "crates" / "cli" / "src", copied, dirs_exist_ok=True)
    module = copied / "terminal_ui.rs"
    source = module.read_text()
    marker = "    mod tests {\n"
    assert source.count(marker) == 1
    fixture = json.dumps(str(root / "crates/daemon/tests/support/session_fixture.rs"))
    additions = f"        #[path = {fixture}] mod planning_session_fixture;\n"
    for name in ("resume_presentation_probe.rs", "queued_delta_ack_probe.rs"):
        probe = json.dumps(str(research / name))
        additions += f"        include!({probe});\n"
    module.write_text(source.replace(marker, marker + additions, 1))
    binary = output / "presentation-probe"
    dependency_dir = Path(libraries["zterm_core"]).parent
    command = ["rustc", "+1.98.0", "--edition=2024", "--test", "--crate-name",
               "zterm_cli", str(copied / "lib.rs"), "-L", f"dependency={dependency_dir}",
               "-o", str(binary)]
    for name in ("base64", "clap", "futures_util", "nix", "rustix", "serde",
                 "serde_json", "tempfile", "tokio", "unicode_width", "zeroize",
                 "zterm_core", "zterm_daemon", "zterm_platform"):
        command.extend(["--extern", f"{name}={libraries[name]}"])
    for path in sorted(native_paths):
        command.extend(["-L", path])
    environment = dict(os.environ)
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    environment["CARGO_PKG_VERSION"] = manifest["workspace"]["package"]["version"]
    subprocess.run(command, cwd=root, env=environment, check=True)
    run = subprocess.run(
        [str(binary), "planning_probe_", "--nocapture"],
        cwd=root, capture_output=True, text=True, timeout=30,
    )
    (output / "presentation-probe.log").write_text(run.stdout + run.stderr)
    print(run.stdout + run.stderr)
    print(f"PROBE_TEST_EXIT={run.returncode} (101 is the expected baseline failure)")
    if run.returncode != 101:
        raise SystemExit("baseline failure was not reproduced; inspect the evidence")


if __name__ == "__main__":
    main()
