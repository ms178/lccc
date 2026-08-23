#!/usr/bin/env bash
# Build the LCCC compiler itself reproducibly under the project research policy:
# Rust optimization level 1 and exactly two Cargo jobs.  This affects the
# compiler executable, not the C code-generation optimization flag passed to
# LCCC by the benchmark runner.
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

# Rust/linker peaks exceed physical memory on the constrained research host.
# The helper is a no-op when any swap is already active and recreates the
# disposable /swapfile after an Arena root-filesystem reset.
"$repo_root/scripts/ensure_swap.sh"

# Use the persisted exact research toolchain rather than whichever Cargo the
# base image happened to ship.  rust-toolchain.toml provides the same pin for
# interactive invocations.
if [[ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]]; then
  export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi
export RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-1.98.0}

# Cargo has no direct `-O1` CLI spelling.  This profile override is the Cargo/
# rustc equivalent and is intentionally scoped to this invocation.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=1
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0

printf '%s\n' "Building LCCC with Rust opt-level=1 and Cargo jobs=2"
printf '%s\n' "Active swap (if any):"
if [[ -r /proc/swaps ]]; then
  cat /proc/swaps
fi

exec cargo build --release --locked -j 2
