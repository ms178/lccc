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

export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}

printf '%s\n' "Building LCCC (fastbuild profile: -O1, no LTO, incremental)"

exec cargo build --profile fastbuild --locked -j "${CARGO_BUILD_JOBS}"
