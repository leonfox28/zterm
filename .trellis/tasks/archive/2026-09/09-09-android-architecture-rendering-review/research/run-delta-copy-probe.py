"""Build the real host library and run a synthetic, process-free review probe."""
import json
from pathlib import Path
import subprocess
import tempfile

source = Path(__file__).resolve().with_name("delta-copy-probe.rs")
repository = Path(__file__).resolve().parents[4]
build = subprocess.run(
    ["cargo", "build", "-p", "zterm-terminal", "--message-format=json"],
    cwd=repository, check=True, capture_output=True, text=True,
)
libraries = {}
for line in build.stdout.splitlines():
    item = json.loads(line)
    if item.get("reason") != "compiler-artifact":
        continue
    name = item["target"]["name"]
    if name in ("zterm_core", "zterm_terminal"):
        for path in item["filenames"]:
            if path.endswith(".rlib"):
                libraries[name] = path
assert set(libraries) == {"zterm_core", "zterm_terminal"}
with tempfile.TemporaryDirectory(prefix="zterm-android-review-") as scratch:
    executable = Path(scratch) / "delta-copy-probe"
    command = ["rustc", "--edition=2024", str(source), "-L",
               f"dependency={Path(libraries['zterm_core']).parent}", "-o", str(executable)]
    for name, path in libraries.items():
        command.extend(["--extern", f"{name}={path}"])
    subprocess.run(command, cwd=repository, check=True)
    subprocess.run([str(executable)], check=True)
