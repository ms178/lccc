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

# Use the persisted Rust toolchain rather than whichever Cargo the base image
# happened to ship.  The shared selector follows rust-toolchain.toml's current
# channel instead of duplicating a version number in every maintenance script.
if [[ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]]; then
  export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi
# shellcheck source=rust_toolchain.sh
source "$repo_root/scripts/rust_toolchain.sh"
lccc_select_rust_toolchain "$repo_root"
printf '%s\n' "Rust toolchain: $LCCC_SELECTED_RUST_TOOLCHAIN ($(rustc --version))"

# Cargo has no direct `-O1` CLI spelling.  This profile override is the Cargo/
# rustc equivalent and is intentionally scoped to this invocation.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=1
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0

# Treat a Rust warning as a failed compiler build, just as fastbuild does.
# Callers doing an intentionally incomplete local refactor can explicitly opt
# out with LCCC_ALLOW_WARNINGS=1; CI and normal release builds never do.
if [[ "${LCCC_ALLOW_WARNINGS:-0}" != "1" ]]; then
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"
fi

printf '%s\n' "Building LCCC with Rust opt-level=1, Cargo jobs=2, warnings=denied"
printf '%s\n' "Active swap (if any):"
if [[ -r /proc/swaps ]]; then
  cat /proc/swaps
fi

exec cargo build --release --locked -j 2
