#!/usr/bin/env bash
# Fast-iteration build of the LCCC compiler for development / research loops.
#
# Uses the `fastbuild` Cargo profile (see Cargo.toml): opt-level 1 (project
# research policy), LTO off, incremental compilation on, 256 codegen units,
# line-tables-only debuginfo. On a 2-core box this restores the fast
# edit-compile-test cycle that the release profile's thin LTO destroyed
# (LTO disables incremental compilation and serializes codegen).
#
# Binaries land in target/fastbuild/{lccc,lccc-x86,lccc-arm,lccc-riscv,
# lccc-i686,lccc-ld}.
#
# Ship-quality binaries still come from scripts/build_lccc_o1_j2.sh
# (release profile, thin LTO, non-incremental, reproducible).
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

"$repo_root/scripts/ensure_swap.sh"

# Prefer the persisted rustup installation after an Arena restore.  The
# system-image Cargo is intentionally not the research baseline: it was 1.85
# while this repository now pins Rust/Cargo 1.98.0 in rust-toolchain.toml.
if [[ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]]; then
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi
export RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-1.98.0}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}

printf '%s\n' "Building LCCC (fastbuild profile: -O1, no LTO, incremental)"

# Fail on warnings unless explicitly opted out.
#
# A `private_interfaces` warning shipped once because the build was only ever
# grepped for '^error': a public enum variant exposed a private RAII type, and
# `cargo build` reported it while every check treated a zero exit status as
# success. Warnings are part of the build contract, so the build enforces them.
#
# Set LCCC_ALLOW_WARNINGS=1 for a scratch build mid-refactor.
if [ "${LCCC_ALLOW_WARNINGS:-0}" != "1" ]; then
    export RUSTFLAGS="${RUSTFLAGS:-} -D warnings"
fi

exec cargo build --profile fastbuild --locked -j "${CARGO_BUILD_JOBS}"
