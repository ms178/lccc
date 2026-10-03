#!/usr/bin/env bash
# Regression coverage for scripts/rust_toolchain.sh.
#
# Keep manifest parsing and override precedence stable: a harness restore or
# bisection must never silently pick whichever rustup default happened to be
# installed.  This test does not install any toolchain; its synthetic channel
# names prove only selection behavior.
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
# shellcheck source=../../scripts/rust_toolchain.sh
source "$repo_root/scripts/rust_toolchain.sh"

expect_eq() {
    local actual=$1 expected=$2 label=$3
    if [[ "$actual" != "$expected" ]]; then
        printf 'FAIL %s: got %q, expected %q\n' "$label" "$actual" "$expected" >&2
        exit 1
    fi
}

manifest_channel=$(lccc_rust_toolchain_from_manifest "$repo_root")
(
    unset LCCC_RUST_TOOLCHAIN RUSTUP_TOOLCHAIN LCCC_SELECTED_RUST_TOOLCHAIN
    lccc_select_rust_toolchain "$repo_root"
    expect_eq "$RUSTUP_TOOLCHAIN" "$manifest_channel" "repository manifest channel"
    expect_eq "$LCCC_SELECTED_RUST_TOOLCHAIN" "$manifest_channel" "reported manifest channel"
)

(
    export RUSTUP_TOOLCHAIN=caller-override
    unset LCCC_RUST_TOOLCHAIN LCCC_SELECTED_RUST_TOOLCHAIN
    lccc_select_rust_toolchain "$repo_root"
    expect_eq "$RUSTUP_TOOLCHAIN" caller-override "caller Rustup override"
)

(
    export RUSTUP_TOOLCHAIN=caller-override
    export LCCC_RUST_TOOLCHAIN=lccc-override
    unset LCCC_SELECTED_RUST_TOOLCHAIN
    lccc_select_rust_toolchain "$repo_root"
    expect_eq "$RUSTUP_TOOLCHAIN" lccc-override "LCCC override precedence"
)

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-toolchain-selector.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
printf '[toolchain]\nchannel = "synthetic-channel"\n' > "$tmp/rust-toolchain.toml"
expect_eq "$(lccc_rust_toolchain_from_manifest "$tmp")" synthetic-channel "synthetic manifest channel"
rm "$tmp/rust-toolchain.toml"
expect_eq "$(lccc_rust_toolchain_from_manifest "$tmp")" stable "missing-manifest fallback"

printf 'PASS rust_toolchain selector: manifest=%s, overrides honored\n' "$manifest_channel"
