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
# system-image Cargo is intentionally not the project toolchain; select the
# current channel from rust-toolchain.toml rather than pinning it in scripts.
if [[ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]]; then
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi
# shellcheck source=rust_toolchain.sh
source "$repo_root/scripts/rust_toolchain.sh"
lccc_select_rust_toolchain "$repo_root"
# Compiler builds are deliberately bounded to two jobs on the constrained
# research host; do not silently inherit an oversubscribing Cargo setting.
export CARGO_BUILD_JOBS=2
printf '%s\n' "Rust toolchain: $LCCC_SELECTED_RUST_TOOLCHAIN ($(rustc --version))"
printf '%s\n' "Cargo jobs: $CARGO_BUILD_JOBS (fixed research policy)"

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
# Small hosts (< 6 GB): link with `--no-keep-memory`. GNU ld's default keeps
# symbol/string tables resident for the whole link; on the monolithic lib-test
# binary that spikes past what a 4 GB cgroup leaves free and the OOM killer
# SIGKILLs rustc at the link step (verified: same invocation, SIGKILL without
# the flag, clean link with it). The flag makes ld trade link SPEED for
# bounded memory (re-reading sections instead of caching) — the right default
# exactly where RAM is the constraint. CI-sized hosts keep the fast path.
# Published through target/lccc-rustflags so `scripts/ci_local.sh`'s
# cargo-test leg reuses the SAME flag set (one cargo cache, no rebuild).
total_mb=$(free -m 2>/dev/null | awk '/^Mem:/{print $2}')
# BSD/macOS have no `free`; sysctl reports total bytes there (hw.memsize on
# Darwin, hw.physmem on some BSDs). WO-8 (red-team audit, 2026-09-18): the
# old code silently skipped the small-host flag on those hosts — graceful,
# but recoverable. If neither probe answers, the flag stays unapplied
# (unchanged legacy behavior).
if [ -z "$total_mb" ] && command -v sysctl >/dev/null 2>&1; then
    bytes=$(sysctl -n hw.memsize 2>/dev/null || sysctl -n hw.physmem 2>/dev/null || true)
    case "$bytes" in
        '' | *[!0-9]*) ;; # absent or non-numeric: leave unset
        *) total_mb=$((bytes / 1048576)) ;;
    esac
fi
if [ -n "$total_mb" ] && [ "$total_mb" -lt 6000 ]; then
    rustflags="$rustflags -C link-arg=-Wl,--no-keep-memory"
fi
# Publish the resolved flags. RUSTFLAGS is part of cargo's fingerprint, so a
# later `cargo test` that passed a different value would rebuild the entire
# tree; `scripts/ci_local.sh` reads this file to reuse the build's cache.
mkdir -p target && printf '%s\n' "$rustflags" > target/lccc-rustflags

# Export unconditionally: RUSTFLAGS outranks every config file, so it is the
# single authoritative source of rustc flags in all three link modes. (Only the
# host target is built here; the i686 `-m32` link arg in .cargo/config.toml
# applies to `--target i686-unknown-linux-gnu`, which this script never uses.)
export RUSTFLAGS="$rustflags"

exec cargo build --profile fastbuild --locked -j "${CARGO_BUILD_JOBS}" \
    "${cargo_config[@]}"
