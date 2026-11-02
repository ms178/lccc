#!/usr/bin/env bash
# Byte-boundary and bulk-copy ABI oracle. The same fixture probes mmap guard
# pages, two aggregate arguments, scalar/F64 neighbors, regparm -> cdecl,
# fastcall, and over-aligned local sources. Both GCC/LCCC directions matter.
#
# The three compiler pairs (lccc:lccc, lccc:gcc, gcc:lccc) are the point: a
# caller and a callee built by different compilers must agree, so a pair that
# links but returns the wrong value is an ABI split rather than a codegen bug.
#
# Host capability, not compiler capability: an earlier version of this gate had
#   for bits in -m64 -m32; do timeout "$GCC" "$bits" preflight.c -o preflight
# under `set -e`, so a host whose gcc cannot link a 32-bit executable (no
# 32-bit CRT: `cannot find Scrt1.o`) aborted inside the linker and printed a
# `FAIL` that named nothing.  That is a different claim from "the compiler
# mis-passes an aggregate", and it was indistinguishable from it.  The mode is
# now decided by the shared three-level host probe (i386_exec.sh: run | link |
# none) because linking is not evidence that a 32-bit binary can be *executed*;
# this gate's assertion is an exit code, so it needs `run`.  Unmeasurable legs are
# counted and reported as unmeasured -- never as passes -- and LCCC_REQUIRE_I686=1
# turns the shortfall into a failure for hosts that are expected to have it.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$ROOT/target/fastbuild/lccc}
GCC=${GCC:-gcc}
TIMEOUT=${TIMEOUT:-30}
[[ $TIMEOUT =~ ^[1-9][0-9]*$ ]] || { echo 'invalid TIMEOUT' >&2; exit 2; }
[[ -x $CCC ]] || { echo "missing compiler: $CCC" >&2; exit 2; }
command -v "$GCC" >/dev/null
command -v timeout >/dev/null
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
SRC=$ROOT/tests/regression/large_aggregate_call_copy.c
INC=$("$GCC" -print-file-name=include)
printf 'int main(void) { return 0; }\n' >"$WORK/preflight.c"

source "$ROOT/tests/regression/i386_exec.sh"
_i386_ensure_memo "$GCC"   # populate the PARENT-side memo before any $( ) read
i386_cap=$(i386_capability "$GCC")

OPTS=(-O0 -O1 -O2 -O3 -Os)
# Every partial stack-word size plus both sides of the bulk threshold.
SIZES=(16 17 18 19 20 21 22 23 31 2047 2048 2049 4097)
PER_MODE=$(( ${#OPTS[@]} * ${#SIZES[@]} * 3 ))
count=0
unmeasured=0
for bits in -m64 -m32; do
    case $bits in
    -m32) mode_cap=$i386_cap ;;
    -m64) mode_cap=run
          # The 64-bit leg is the oracle for everything here, so an inability to
          # build and run the trivial hosted program is a broken harness and says
          # nothing about lccc.  Say which.
          if ! timeout "$TIMEOUT" "$GCC" $bits "$WORK/preflight.c" -o "$WORK/preflight" 2>"$WORK/preflight.err"; then
              echo "FAIL: $GCC cannot build a hosted -m64 program; this gate has no oracle to compare against" >&2
              sed -n '1,3p' "$WORK/preflight.err" >&2
              exit 1
          fi
          if ! timeout "$TIMEOUT" "$WORK/preflight"; then
              echo "FAIL: a -m64 program built by $GCC did not execute; the host cannot run what this gate measures" >&2
              exit 1
          fi ;;
    esac
    if [[ $mode_cap == none ]]; then
        # Every mode the host cannot even link: counted, named, and not passed.
        echo "SKIP $bits: $GCC cannot link a 32-bit executable on this host -- $PER_MODE configurations unmeasured, not passed"
        unmeasured=$((unmeasured + PER_MODE))
        continue
    fi
    run_legs=0
    [[ $mode_cap == run ]] && run_legs=1
    for opt in "${OPTS[@]}"; do
        for size in "${SIZES[@]}"; do
            for key in gcc lccc; do
                cc=$GCC; [[ $key != lccc ]] || cc=$CCC
                timeout "$TIMEOUT" "$cc" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                    -DBLOB_BYTES="$size" -DCALLER_ONLY -c "$SRC" -o "$WORK/$key-caller.o"
                timeout "$TIMEOUT" "$cc" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                    -DBLOB_BYTES="$size" -DCALLEE_ONLY -c "$SRC" -o "$WORK/$key-callee.o"
            done
            for pair in lccc:lccc lccc:gcc gcc:lccc; do
                caller=${pair%:*}; callee=${pair#*:}
                timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/$caller-caller.o" \
                    "$WORK/$callee-callee.o" -o "$WORK/mixed"
                if [[ $run_legs == 0 ]]; then
                    # The link of a mixed pair is still an assertion (a disagreeing
                    # struct layout can fail to link at all), so it was made above;
                    # what this host cannot do is judge the value.
                    unmeasured=$((unmeasured + 1))
                    continue
                fi
                if ! timeout "$TIMEOUT" "$WORK/mixed"; then
                    echo "FAIL $bits $opt size=$size caller=$caller callee=$callee" >&2
                    exit 1
                fi
                count=$((count + 1))
            done
            if [[ $run_legs == 1 ]]; then
                printf 'PASS %s %s size=%s (three compiler pairs)\n' "$bits" "$opt" "$size"
            fi
        done
    done
    if [[ $run_legs == 0 ]]; then
        printf '%s %s: mixed pairs linked, %d executions unmeasurable (host cannot run 32-bit binaries)\n' \
            SKIP-RUN "$bits" $PER_MODE
        continue
    fi
done
printf 'aggregate-call ABI: %d mixed-pair execution(s) passed, %d unmeasured (host i386 capability: %s)\n' \
    "$count" "$unmeasured" "$i386_cap"
if [[ $count -eq 0 ]]; then
    # The vacuity latch: with nothing executed, "0 failures" is a statement about
    # the host, not about lccc, and a gate that prints a pass for it is worse than
    # no gate: it manufactures a green result.
    echo "FAIL: no configuration was executable -- the harness or the host is broken" >&2
    exit 1
fi
if [[ $unmeasured -gt 0 && ${LCCC_REQUIRE_I686:-0} == 1 ]]; then
    echo "FAIL: LCCC_REQUIRE_I686=1 but $unmeasured i386 configuration(s) were unmeasurable" >&2
    exit 1
fi
