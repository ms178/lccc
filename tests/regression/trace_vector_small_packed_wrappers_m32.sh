#!/usr/bin/env bash
# Best-effort CI diagnostic for the i686 small-vector crash. This runs the
# exact fixture body under a SIGSEGV handler and reports a symbolized EIP; it
# is deliberately non-gating so its diagnostic result never masks the normal
# regression corpus verdict.
set -u
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-vsmall-trace.XXXXXX")
trap 'rm -rf "$work"' EXIT

if ! "$CCC" -O2 -m32 -msse2 -g -no-pie \
    "$root/tests/regression/diagnostics/vector_small_packed_wrappers_m32_trace.c" \
    -o "$work/probe" >"$work/build.out" 2>&1; then
    echo "::warning title=i686 vector trace unavailable::diagnostic build failed"
    tail -n 20 "$work/build.out"
    exit 0
fi
set +e
"$work/probe" >"$work/run.out" 2>"$work/run.err"
rc=$?
set -e
if [[ $rc -eq 0 ]]; then
    echo "i686 vector trace probe passed; trace-only build did not reproduce the crash"
    exit 0
fi
cat "$work/run.err"
eip=$(sed -n 's/^SIGSEGV EIP=0x\([[:xdigit:]]\{8\}\)$/0x\1/p' "$work/run.err" | head -n 1)
if [[ -n $eip ]]; then
    source_line=$(addr2line -e "$work/probe" -f -C "$eip" 2>/dev/null | tr '\n' ' ')
    echo "::notice title=i686 vector fault address::EIP=$eip $source_line (rc=$rc)"
    addr=$((eip))
    lo=$((addr > 24 ? addr - 24 : 0))
    hi=$((addr + 24))
    if objdump -d -M intel --disassemble=ret "$work/probe" \
        >"$work/disassembly" 2>/dev/null; then
        symbol_disassembly=$(tr '\n' ';' < "$work/disassembly")
        printf '::notice title=i686 ret function disassembly::%s\n' "$symbol_disassembly"
    fi
    if objdump -d -M intel --disassemble=vector_small_packed_wrappers_test_main \
        "$work/probe" >"$work/caller-disassembly" 2>/dev/null; then
        caller_disassembly=$(head -n 56 "$work/caller-disassembly" | tr '\n' ';')
        printf '::notice title=i686 vector caller disassembly::%s\n' "$caller_disassembly"
    fi
    if objdump -d -M intel --start-address="$lo" --stop-address="$hi" \
        "$work/probe" >"$work/disassembly" 2>/dev/null; then
        context_disassembly=$(tr '\n' ';' < "$work/disassembly")
        printf '::notice title=i686 fault-window disassembly::%s\n' "$context_disassembly"
    fi
else
    echo "::notice title=i686 vector trace probe failed::rc=$rc; no EIP marker (signal handler or executable environment unavailable)"
fi
# Diagnostics only: the ordinary regression owns the pass/fail result.
exit 0
