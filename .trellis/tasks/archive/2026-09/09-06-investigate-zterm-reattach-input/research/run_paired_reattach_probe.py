"""Reuse the disposable paired-Herdr smoke with the current fixed CLI build."""
import importlib.util
import json
from pathlib import Path
import sys
import subprocess

sys.dont_write_bytecode = True
ROOT = next(p for p in Path(__file__).resolve().parents if (p / "Cargo.toml").is_file())
ARCHIVED = ROOT / ".trellis/tasks/archive/2026-09/09-05-fix-terminal-sync-scroll/research"
BINARY = ROOT / "target/reattach-input-probe/fixed/cli-runtime-probe"
assert BINARY.is_file(), "run the local --verify-fix probe first"
sys.path.insert(0, str(ARCHIVED))
spec = importlib.util.spec_from_file_location("paired_probe", ARCHIVED / "run_paired_herdr_probe.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.build = lambda *_args: BINARY
output = ROOT / "target/terminal-sync-scroll"
output.mkdir(parents=True, exist_ok=True)
existing = set(output.glob("zterm-causal-*/outcome.json"))
sys.argv = [str(Path(__file__)), "--target", "dev", "--reattach"]
cleanup_error = None
try:
    module.main()
except RuntimeError as error:
    if 'test Session cleanup failed:' not in str(error) or 'operation_outcome_unknown' not in str(error):
        raise
    cleanup_error = str(error)
created = set(output.glob("zterm-causal-*/outcome.json")) - existing
assert len(created) == 1, "one isolated paired result"
result = json.loads(created.pop().read_text())
if cleanup_error:
    # Ambiguous remote mutation results require observation, not a new close.
    query = subprocess.run([str(ROOT / 'target/debug/zterm'), 'session', 'list', 'dev'],
                           capture_output=True, text=True, timeout=20, check=True)
    assert result['name'] not in query.stdout, 'the exact private Session still exists'
    result['cleanup'] = 'close outcome unknown; read-only list confirmed exact Session absent'
else:
    result['cleanup'] = 'close exit 0'
Path(__file__).with_name("post-fix-paired-outcome.json").write_text(json.dumps(result, indent=2) + "\n")
assert result["herdr_visible"] and result["reattach_herdr_visible"]
assert result["reattach_input_roundtrip"], "paired retained Herdr must respond to actual input"
assert not result["not_synchronized"] and not result["reattach_not_synchronized"]
