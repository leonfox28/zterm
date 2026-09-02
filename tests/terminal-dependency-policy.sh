#!/bin/sh

set -eu

reject_host_engine() {
    package=$1
    tree=$(cargo +1.98.0 tree --locked -p "$package" --charset ascii)
    if printf '%s\n' "$tree" \
        | grep -E '(^|[[:space:]])(zterm-terminal|alacritty_terminal|vte) v' >/dev/null; then
        echo "$package must remain independent of the host terminal engine" >&2
        printf '%s\n' "$tree" >&2
        exit 1
    fi
}

reject_host_engine zterm-core
reject_host_engine zterm-proto

vte_tree=$(
    cargo +1.98.0 tree --locked --workspace --invert vte@0.15.0 --charset ascii \
        | sed -E 's/ v[0-9][^ ]* \([^)]*\)$//'
)
expected_vte_tree='vte v0.15.0
`-- alacritty_terminal v0.26.0
    `-- zterm-terminal
        `-- zterm-daemon
            `-- zterm-cli'

if [ "$vte_tree" != "$expected_vte_tree" ]; then
    echo "vte must have exactly the pinned official Alacritty dependency path" >&2
    printf '%s\n' "$vte_tree" >&2
    exit 1
fi

echo "terminal dependency policy verified"
