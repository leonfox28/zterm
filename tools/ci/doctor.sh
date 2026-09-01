#!/bin/sh
set -eu

missing=0

require_tool() {
    command_name=$1
    hint=$2
    if command -v "$command_name" >/dev/null 2>&1; then
        version=$($command_name --version 2>&1 | sed -n '1p')
        printf '%-12s %s\n' "$command_name" "$version"
    else
        printf '%-12s MISSING — %s\n' "$command_name" "$hint" >&2
        missing=1
    fi
}

require_rust_component() {
    component=$1
    if rustup component list --toolchain 1.98.0 --installed 2>/dev/null \
        | grep -Eq "^$component(-|$)"; then
        printf '%-12s %s\n' "rust-$component" installed
    else
        printf '%-12s MISSING — rustup component add %s --toolchain 1.98.0\n' \
            "rust-$component" "$component" >&2
        missing=1
    fi
}

require_shellcheck() {
    if command -v shellcheck >/dev/null 2>&1; then
        version=$(shellcheck --version 2>&1 | sed -n 's/^version: //p')
        [ -n "$version" ] || version=unknown
        printf '%-12s %s\n' shellcheck "$version"
    else
        echo 'shellcheck   MISSING — brew install shellcheck' >&2
        missing=1
    fi
}

require_tool rustup 'install from https://rustup.rs'
require_tool cargo 'install from https://rustup.rs'
if command -v rustc >/dev/null 2>&1 \
    && rustc +1.98.0 --version >/dev/null 2>&1; then
    printf '%-12s %s\n' 'rust-1.98' "$(rustc +1.98.0 --version)"
else
    echo 'rust-1.98    MISSING — rustup toolchain install 1.98.0 --profile minimal --component clippy --component rustfmt --component rust-src' >&2
    missing=1
fi
require_rust_component clippy
require_rust_component rustfmt
require_rust_component rust-src
require_tool just 'brew install just, or cargo install just --version 1.42.4 --locked'
require_shellcheck
require_tool actionlint 'brew install actionlint, or go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12'
require_tool cargo-deny 'cargo install cargo-deny --version 0.20.2 --locked'
require_tool gh 'brew install gh, then gh auth login'
require_tool jq 'brew install jq'
require_tool python3 'install Python 3'

if command -v docker >/dev/null 2>&1; then
    printf '%-12s %s\n' docker "$(docker --version 2>&1 | sed -n '1p')"
    if docker compose version >/dev/null 2>&1; then
        printf '%-12s %s\n' compose "$(docker compose version 2>&1 | sed -n '1p')"
    else
        echo 'compose      OPTIONAL — Docker/QEMU relay evidence remains hosted-only' >&2
    fi
else
    echo 'docker       OPTIONAL — Docker/QEMU relay evidence remains hosted-only' >&2
fi

echo 'HOSTED-ONLY: non-host OS/architectures, glibc 2.28, Docker/QEMU execution, protected signing, final installer matrix, attestation, and immutable publication.'

[ "$missing" -eq 0 ] || exit 1
