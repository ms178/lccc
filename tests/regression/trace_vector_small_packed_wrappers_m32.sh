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

GCC=${GCC:-gcc-16}
if ! "$CCC" -O2 -m32 -msse2 -g -c \
    "$root/tests/regression/diagnostics/vector_small_packed_wrappers_m32_trace.c" \
    -o "$work/fixture.o" >"$work/build.out" 2>&1; then
    echo "::warning title=i686 vector trace unavailable::LCCC fixture-object build failed"
    tail -n 20 "$work/build.out"
    exit 0
fi
if ! "$GCC" -m32 -g -no-pie \
    "$root/tests/regression/diagnostics/vector_small_packed_wrappers_m32_signal.c" \
    "$work/fixture.o" -o "$work/probe" >>"$work/build.out" 2>&1; then
    echo "::warning title=i686 vector trace unavailable::GCC signal-harness link failed"
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
if objdump -d -M intel --disassemble=ret "$work/probe" \
    >"$work/disassembly" 2>/dev/null; then
    symbol_disassembly=$(tr '\n' ';' < "$work/disassembly")
    printf '::notice title=i686 ret function disassembly::%s\n' "$symbol_disassembly"
fi
registers=$(grep -m1 '^SIGSEGV EIP=' "$work/run.err" || true)
stack_words=$(grep -m1 '^STACK=' "$work/run.err" || true)
if [[ -n $stack_words ]]; then
    echo "::notice title=i686 fault stack words at ESP+0x40::$stack_words"
fi
return_addr=$(sed -n 's/^RETADDR=//p' "$work/run.err" | head -n 1)
if [[ -n $return_addr ]]; then
    echo "::notice title=i686 faulting call return address::$return_addr"
fi
eip=$(sed -n 's/^SIGSEGV EIP=0x\([[:xdigit:]]\{8\}\).*/0x\1/p' "$work/run.err" | head -n 1)
if [[ -n $eip ]]; then
    source_line=$(addr2line -e "$work/probe" -f -C "$eip" 2>/dev/null | tr '\n' ' ')
    echo "::notice title=i686 vector fault registers::$registers"
    echo "::notice title=i686 vector fault address::EIP=$eip $source_line (rc=$rc)"
    if [[ -n $return_addr ]]; then
        printf -v caller_pc '%x' "$((return_addr))"
        if objdump -d -M intel "$work/probe" >"$work/full-disassembly" 2>/dev/null; then
            return_line=$(grep -n -m1 -E "^[[:space:]]*${caller_pc}:" "$work/full-disassembly" | cut -d: -f1 || true)
            if [[ -n $return_line ]]; then
                context_start=$((return_line > 36 ? return_line - 36 : 1))
                context_end=$((return_line + 2))
                call_context=$(sed -n "${context_start},${context_end}p" "$work/full-disassembly" | tr '\n' ';')
                printf '::notice title=i686 faulting return-address call-site context::%s\n' "$call_context"
            fi
        fi
    fi
    addr=$((eip))
    lo=$((addr > 24 ? addr - 24 : 0))
    hi=$((addr + 24))
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
