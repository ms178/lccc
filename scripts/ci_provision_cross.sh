#!/usr/bin/env bash
# Provision the AArch64 / RISC-V cross toolchains for a CI lane, once.
#
# WHY THIS EXISTS
# ---------------
# The same 13-line `run:` body was pasted into three lanes (assembler/linker,
# contract shard a, contract shard b). Three copies of a procedure is three
# places for the i386 invariants below to rot, and the hosted run made the cost
# visible: 331 s in the assembler/linker lane, 165 s in shard b and 23 s in
# shard a for the identical package set, because every lane re-downloaded it
# from scratch. The body now lives here -- one owner of the contract, testable
# without a runner -- and the download itself is served from
# LCCC_CROSS_DEB_DIR (see ensure_cross_toolchains.sh), which the workflow
# caches.
#
# WHY THE PROBES AROUND IT
# ------------------------
# `scripts/ensure_cross_toolchains.sh` unpacks the cross packages with
# `dpkg -x` to avoid apt's dependency solver, which would otherwise REMOVE the
# i686 multilib that the same lane needs for the i386 suites. Those suites fail
# (they do not skip) when CI sets `LCCC_REQUIRE_I386=1`, so a silently broken
# multilib 600 lines later is a wall of red with no cause attached. The probe
# asserts the invariant on both sides of the provisioning step: the tool that
# may break it is the one that reports breaking it.
#
# A provisioning failure is a WARNING, not an error: the cross-executed halves
# of the TLS and relocation gates report SKIP explicitly rather than pretending
# to have measured something, so the lane stays honest and green. The i386
# invariant is the exception -- that one is an error, because it invalidates
# gates that do not skip.
set -uo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPO" || exit 1

probe=${LCCC_M32_PROBE:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/m32-probe}
printf 'int main(void) { return 0; }\n' > "$probe.c"

m32_ok() { gcc -m32 -O1 -o "$probe" "$probe.c"; }

if ! m32_ok; then
    echo "::error::i386 multilib is broken before cross provisioning"
    exit 1
fi

if ! bash "$REPO/scripts/ensure_cross_toolchains.sh" all; then
    echo "::warning::cross toolchain provisioning failed; the TLS runtime half will SKIP"
fi

if ! m32_ok; then
    echo "::error::cross provisioning broke the i386 multilib"
    exit 1
fi

for tool in aarch64 riscv64; do
    if command -v "$tool-linux-gnu-gcc" >/dev/null 2>&1; then
        "$tool-linux-gnu-gcc" --version | head -n 1
    else
        echo "::warning::$tool-linux-gnu-gcc unavailable"
    fi
done
