#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)

# Scan the repository, including example deployment files. Generated build
# output, Git internals, and local Trellis runtime state are not source inputs.
if grep -rIE \
    --exclude-dir='.git' \
    --exclude-dir='target' \
    --exclude-dir='.runtime' \
    '(BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|ssh-(rsa|ed25519) [A-Za-z0-9+/]{40,}|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{36,}|password[[:space:]]*=[[:space:]]*[^[:space:]#]+|token[[:space:]]*=[[:space:]]*[^[:space:]#]+)' \
    "$repo_root"; then
    echo "possible secret found in the zterm repository" >&2
    exit 1
fi

echo "Phase Zero repository secret scan passed"
