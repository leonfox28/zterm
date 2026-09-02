#!/bin/sh

set -eu

rust_sources=$(git ls-files '*.rs')
if [ -z "$rust_sources" ]; then
    echo "expected at least one tracked Rust source" >&2
    exit 1
fi

resolved_attributes=$(
    printf '%s\n' "$rust_sources" | git check-attr --stdin eol
)
unexpected_attributes=$(
    printf '%s\n' "$resolved_attributes" | grep -v ': eol: lf$' || true
)
if [ -n "$unexpected_attributes" ]; then
    echo "Rust sources must resolve to eol=lf:" >&2
    printf '%s\n' "$unexpected_attributes" >&2
    exit 1
fi

carriage_return=$(printf '\r')
crlf_sources=$(git grep -I -l "$carriage_return" -- '*.rs' || true)
if [ -n "$crlf_sources" ]; then
    echo "Rust sources contain carriage returns after checkout:" >&2
    printf '%s\n' "$crlf_sources" >&2
    exit 1
fi

manifest_files="Cargo.toml $(find crates tools -mindepth 2 -maxdepth 2 -name Cargo.toml -print)"
if grep -nE '^[[:space:]]*vt100[[:space:]]*=' $manifest_files; then
    echo "vt100 must not be present in the workspace dependency graph" >&2
    exit 1
fi
if grep -niE '^[[:space:]]*([[:alnum:]_-]*ghostty[[:alnum:]_-]*|vte)[[:space:]]*=|git[[:space:]]*=' $manifest_files; then
    echo "terminal dependencies must use the pinned official Alacritty crates.io boundary" >&2
    exit 1
fi
if ! grep -Fqx 'alacritty_terminal = { version = "=0.26.0", default-features = false }' Cargo.toml; then
    echo "official alacritty_terminal 0.26.0 pin/default-feature policy is missing" >&2
    exit 1
fi
if grep -E 'name = "vt100"' Cargo.lock >/dev/null; then
    echo "Cargo.lock unexpectedly contains vt100" >&2
    exit 1
fi

unsafe_code=$(find crates tools -type f -name '*.rs' -exec grep -nHE '(^|[^[:alnum:]_])(unsafe[[:space:]]+((extern[[:space:]]+"[[:alnum:]_-]+"[[:space:]]+)?(fn|impl|trait)|extern)|unsafe[[:space:]]*\{|unsafe[[:space:]]*\()' {} + || true)
if [ -n "$unsafe_code" ]; then
    echo "zterm-owned Rust code must not contain unsafe code:" >&2
    printf '%s\n' "$unsafe_code" >&2
    exit 1
fi

for client_manifest in crates/core/Cargo.toml crates/proto/Cargo.toml; do
    if grep -E 'alacritty_terminal|zterm-terminal|^[[:space:]]*vte[[:space:]]*=' "$client_manifest" >/dev/null; then
        echo "$client_manifest must remain host-engine-free" >&2
        exit 1
    fi
done
if grep -E 'alacritty_terminal|zterm-terminal|^[[:space:]]*vte[[:space:]]*=' crates/cli/Cargo.toml >/dev/null; then
    echo "zterm-cli must not directly depend on the terminal engine" >&2
    exit 1
fi
forbidden_alacritty_api=$(find crates -type f -name '*.rs' -exec grep -nHE 'alacritty_terminal::(tty|event_loop)' {} + || true)
if [ -n "$forbidden_alacritty_api" ]; then
    echo "zterm must keep PTY/process ownership out of the Alacritty boundary" >&2
    printf '%s\n' "$forbidden_alacritty_api" >&2
    exit 1
fi

journal_probe=.trellis/workspace/source-policy-probe/journal-probe.md
journal_attribute=$(git check-attr merge -- "$journal_probe")
if [ "$journal_attribute" != "$journal_probe: merge: union" ]; then
    echo "Trellis journal merge=union contract was not preserved" >&2
    exit 1
fi

echo "source checkout policy verified"
