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

journal_probe=.trellis/workspace/source-policy-probe/journal-probe.md
journal_attribute=$(git check-attr merge -- "$journal_probe")
if [ "$journal_attribute" != "$journal_probe: merge: union" ]; then
    echo "Trellis journal merge=union contract was not preserved" >&2
    exit 1
fi

echo "source checkout policy verified"
