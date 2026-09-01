#!/bin/sh
set -eu

[ "$#" -gt 0 ] || {
    echo 'python syntax check requires at least one source file' >&2
    exit 64
}

python3 -c '
import pathlib
import sys

for source_path in sys.argv[1:]:
    source = pathlib.Path(source_path).read_bytes()
    compile(source, source_path, "exec")
' "$@"
