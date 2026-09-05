"""Compile the isolated fault probe using the workspace's exact built libraries."""

import json
from pathlib import Path
import subprocess


def main():
    research = Path(__file__).resolve().parent
    root = research.parents[3]
    result = subprocess.run(
        ["cargo", "+1.98.0", "build", "--locked", "--offline", "-p", "zterm-daemon",
         "--message-format=json"],
        cwd=root, capture_output=True, text=True, check=True,
    )
    libraries = {}
    native_paths = set()
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact":
            for filename in item["filenames"]:
                if filename.endswith(".rlib"):
                    libraries[item["target"]["name"]] = filename
        elif item.get("reason") == "build-script-executed":
            native_paths.update(item["linked_paths"])
    output = root / "target" / "architecture-review"
    output.mkdir(exist_ok=True)
    binary = output / "attachment-deadline-probe"
    dependency_dir = Path(libraries["zterm_core"]).parent
    command = ["rustc", "+1.98.0", "--edition=2024", str(research / "attachment_deadline_probe.rs"),
               "-L", f"dependency={dependency_dir}", "-o", str(binary)]
    for name in ("tokio", "tempfile", "zterm_core", "zterm_proto", "zterm_terminal", "zterm_daemon"):
        command.extend(["--extern", f"{name}={libraries[name]}"])
    for path in sorted(native_paths):
        command.extend(["-L", path])
    subprocess.run(command, cwd=root, check=True)
    subprocess.run([str(binary)], cwd=root, check=True, timeout=30)


if __name__ == "__main__":
    main()
