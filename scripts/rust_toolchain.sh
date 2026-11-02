#!/usr/bin/env bash
# Shared Rust-toolchain selection for LCCC maintenance scripts.
#
# The repository's rust-toolchain.toml is the single source of truth.  It
# intentionally follows `stable`, so a restored or new environment selects the
# latest stable Rust rather than silently retaining a version baked into an old
# shell script.  Cargo.toml records the tested minimum (`rust-version`) for
# consumers that need a compatibility floor.
#
# Source this file after determining the repository root, then call:
#     lccc_select_rust_toolchain "$repo_root"
# An explicit LCCC_RUST_TOOLCHAIN or pre-existing RUSTUP_TOOLCHAIN remains an
# intentional caller override for bisection/reproduction work.

lccc_rust_toolchain_from_manifest() {
    local repo_root=${1:?repository root is required}
    local manifest="$repo_root/rust-toolchain.toml"
    local channel
    if [[ -r "$manifest" ]]; then
        channel=$(sed -nE 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$manifest" | head -n 1)
        if [[ -n "$channel" ]]; then
            printf '%s\n' "$channel"
            return 0
        fi
    fi
    # A freshly restored workspace may not have a worktree yet.  `stable`
    # makes rustup resolve the newest supported release once it is installed.
    printf '%s\n' stable
}

lccc_select_rust_toolchain() {
    local repo_root=${1:?repository root is required}
    local selected=${LCCC_RUST_TOOLCHAIN:-${RUSTUP_TOOLCHAIN:-}}
    if [[ -z "$selected" ]]; then
        selected=$(lccc_rust_toolchain_from_manifest "$repo_root")
    fi
    export RUSTUP_TOOLCHAIN="$selected"
    export LCCC_SELECTED_RUST_TOOLCHAIN="$selected"
}
