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

# Honour the repo's clang+mold preference (.cargo/config.toml) when both are
# on PATH. If only mold is available, drive it through the gcc driver
# (gcc >= 12 resolves -fuse-ld=mold to ld.mold on PATH) so the fast-linker
# preference survives with only mold installed (e.g. conda-forge mold next
# to a system gcc).
# `--config` overrides the *linker*, but it cannot CANCEL the committed
# `target.<triple>.rustflags` entry that passes `-fuse-ld=mold`: cargo keeps
# the config-file value for a key it also finds on the command line, so
# `--config 'target...rustflags=[]'` leaves `-fuse-ld=mold` in every link and
# the build dies with "collect2: fatal error: cannot find 'ld'" on hosts
# without mold.  The RUSTFLAGS environment variable is the only override that
# outranks every config file, so the linker mode is expressed through it.
cargo_config=()
rustflags=""
if command -v clang >/dev/null 2>&1 && command -v mold >/dev/null 2>&1; then
    : # keep .cargo/config.toml (clang driver, -fuse-ld=mold)
    # Reproduce the committed flags explicitly: RUSTFLAGS is authoritative in
    # every mode, so it must not silently drop the mold preference.
    rustflags="-C link-arg=-fuse-ld=mold"
elif command -v ld.mold >/dev/null 2>&1; then
    # mold without clang: gcc driver + mold backend. `-fuse-ld=mold` requires
    # an `ld.mold` on PATH (the conda-forge/make-install symlink layout);
    # `command -v ld.mold` guarantees that exact resolution.
    cargo_config+=(
        --config 'target.x86_64-unknown-linux-gnu.linker="gcc"'
        --config 'target.i686-unknown-linux-gnu.linker="gcc"'
    )
    rustflags="-C link-arg=-fuse-ld=mold"
    printf '%s\n' "note: clang not found; linking with gcc driver + mold backend"
else
    cargo_config+=(
        --config 'target.x86_64-unknown-linux-gnu.linker="gcc"'
        --config 'target.i686-unknown-linux-gnu.linker="gcc"'
    )
    printf '%s\n' "note: clang/mold not found; linking with gcc + GNU ld (bfd)"
fi

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
    rustflags="${rustflags:+$rustflags }-D warnings"
fi
# Export unconditionally: RUSTFLAGS outranks every config file, so it is the
# single authoritative source of rustc flags in all three link modes. (Only the
# host target is built here; the i686 `-m32` link arg in .cargo/config.toml
# applies to `--target i686-unknown-linux-gnu`, which this script never uses.)
export RUSTFLAGS="$rustflags"

exec cargo build --profile fastbuild --locked -j "${CARGO_BUILD_JOBS}" \
    "${cargo_config[@]}"
