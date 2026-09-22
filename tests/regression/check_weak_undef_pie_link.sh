#!/usr/bin/env bash
# Undefined-weak address materialisation must link under the SYSTEM gcc
# driver's default PIE mode (PR #581 regression gate).
#
# `(fn != NULL)` on an undefined weak function materialises the symbol
# address. lccc emitted a direct `leaq sym(%rip)` (R_X86_64_PC32), which
# GNU ld rejects for undefined weaks in PIE links:
#   relocation R_X86_64_PC32 against undefined symbol
#   `ZSTD_trace_compress_begin' can not be used when making a PIE object
# (real-world hit: an lccc-built libzstd.a linked by the distro gcc
# driver, lib/common/zstd_trace.h tracing hooks). The fix routes weak
# addresses through @GOTPCREL in every code model.
#
# The corpus runner links through lccc's built-in linker, which accepts
# the PC32 form — so this pipeline is only checkable end-to-end here:
# lccc -c  ->  system gcc driver link (default PIE)  ->  run.
# Pre-fix this gate goes red at the link step.
set -u -o pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC="$(cd "$(dirname "$CCC")" && pwd)/$(basename "$CCC")"
src="$(dirname "$0")/weak_extern_pie_link.c"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

"$CCC" -O2 -c "$src" -o "$tmp/wu.o" || { >&2 echo "FAIL: lccc -c"; exit 1; }
gcc "$tmp/wu.o" -o "$tmp/wu" || {
    >&2 echo "FAIL: system gcc driver rejected the lccc object (PIE link)"
    exit 1
}
out="$("$tmp/wu")" || { >&2 echo "FAIL: run"; exit 1; }
[ "$out" = "v=41" ] || { >&2 echo "FAIL: output '$out' != 'v=41'"; exit 1; }

# GCC oracle: same object shape must link and agree (differential anchor).
gcc -O2 "$src" -o "$tmp/wu_gcc" 2>/dev/null && {
    out_gcc="$("$tmp/wu_gcc")"
    [ "$out_gcc" = "$out" ] || { >&2 echo "FAIL: lccc '$out' != gcc '$out_gcc'"; exit 1; }
}

echo "OK check_weak_undef_pie_link"
