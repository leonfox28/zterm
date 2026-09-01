#!/bin/sh
set -eu

repo_root=${SECRET_SCAN_ROOT:-$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)}

# Scan every source/deployment input. Build output, Git internals, and local
# Trellis runtime state are not inputs shipped to users.
if grep -rIlE \
    --exclude-dir='.git' \
    --exclude-dir='.trellis' \
    --exclude-dir='target' \
    --exclude-dir='.runtime' \
    '(BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|ssh-(rsa|ed25519) [A-Za-z0-9+/]{40,}|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{36,}|password[[:space:]]*=[[:space:]]*[^[:space:]#]+|token[[:space:]]*=[[:space:]]*[^[:space:]#]+)' \
    "$repo_root"; then
    echo "possible secret found in the zterm repository" >&2
    exit 1
fi

echo "repository secret scan passed"
