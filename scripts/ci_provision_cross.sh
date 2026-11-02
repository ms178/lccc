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
# Hosted lanes require the cross-executed TLS and relocation gates as well as
# i386: an unavailable toolchain is an error, not a silently green skip. Keep
# the diagnostic tail in the check annotation for clients that cannot retrieve
# GitHub's signed log archive.
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

cross_log=$(mktemp "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/lccc-cross.XXXXXX")
trap 'rm -f "$cross_log"' EXIT
cross_rc=0
if ! bash "$REPO/scripts/ensure_cross_toolchains.sh" all 2>&1 | tee "$cross_log"; then
    details=$(tail -n 25 "$cross_log")
    details=${details//'%'/'%25'}
    details=${details//$'\n'/'%0A'}
    details=${details//$'\r'/'%0D'}
    echo "::error title=Cross toolchain provisioning failed::$details"
    cross_rc=1
fi

if ! m32_ok; then
    echo "::error::cross provisioning broke the i386 multilib"
    exit 1
fi

for tool in aarch64 riscv64; do
    if command -v "$tool-linux-gnu-gcc" >/dev/null 2>&1; then
        "$tool-linux-gnu-gcc" --version | head -n 1
    else
        echo "::error::$tool-linux-gnu-gcc unavailable"
        cross_rc=1
    fi
done
exit "$cross_rc"
