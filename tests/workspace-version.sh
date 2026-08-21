#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

fail() {
    echo "workspace version check failed: $*" >&2
    exit 1
}

cargo +1.98.0 metadata --locked --no-deps --format-version 1 \
    --manifest-path "$repo_root/Cargo.toml" >/dev/null

for required_crate in cli core daemon platform proto; do
    [ -f "$repo_root/crates/$required_crate/Cargo.toml" ] \
        || fail "required product crate is missing: $required_crate"
done

product_version=
product_count=0
for manifest in "$repo_root"/crates/*/Cargo.toml; do
    [ -f "$manifest" ] || fail "no product crate manifests found"
    product_count=$((product_count + 1))

    inherited_version_count=$(grep -Ec \
        '^[[:space:]]*version[.]workspace[[:space:]]*=[[:space:]]*true[[:space:]]*$' \
        "$manifest" || true)
    [ "$inherited_version_count" -eq 1 ] \
        || fail "$manifest must inherit exactly one workspace version"
    if grep -Eq '^[[:space:]]*version[[:space:]]*=' "$manifest"; then
        fail "$manifest defines a component-specific package version"
    fi

    package_id=$(cargo +1.98.0 pkgid --locked --manifest-path "$manifest")
    package_version=${package_id##*@}
    [ "$package_version" != "$package_id" ] \
        || fail "could not resolve the Cargo package version for $manifest"

    if [ -z "$product_version" ]; then
        product_version=$package_version
    elif [ "$package_version" != "$product_version" ]; then
        fail "$manifest resolved $package_version instead of lockstep $product_version"
    fi
done

[ "$product_count" -gt 0 ] || fail "no product crate manifests found"

echo "workspace product version $product_version is inherited by all $product_count crates"
