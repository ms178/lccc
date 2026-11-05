#!/usr/bin/env bash
# Native psABI interoperability of the two-unit fixtures under interop/
# (`<name>_callee.c` + `<name>_caller.c`), on the host: x86-64 and, where
# the host can execute 32-bit programs, i386.  check_switch_cross_execute.py
# runs the same fixtures on AArch64 and RV64 against Clang; this gate holds
# the host targets to GCC.
#
# Each fixture is self-checking (exit status 0, else the number of the first
# failing check).  For every LCCC level the three pairs lccc:lccc, lccc:gcc
# and gcc:lccc (caller:callee) are linked and run: a caller and a callee
# that merely agree with each other pass the first pair while disagreeing
# with every other compiler.
#
# i386 runs the fixtures written for it: psabi_pair_interop and
# psabi_wide_interop use `__int128`, which i386 does not have, and
# psabi_hfa_interop assumes a 16-byte long double and passes 8-byte GNU
# vectors, which GCC puts in MMX registers whenever MMX is enabled (LCCC's
# i686 back end does not; see the PR notes).  The i386 leg is decided by the
# shared host probe (i386_exec.sh: run | link | none); unmeasurable legs are
# counted and named, never passed, and LCCC_REQUIRE_I686=1 turns them into a
# failure.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$ROOT/target/fastbuild/lccc}
GCC=${GCC:-gcc}
TIMEOUT=${TIMEOUT:-60}
[[ $TIMEOUT =~ ^[1-9][0-9]*$ ]] || { echo 'invalid TIMEOUT' >&2; exit 2; }
[[ -x $CCC ]] || { echo "missing compiler: $CCC" >&2; exit 2; }
command -v "$GCC" >/dev/null || { echo "missing oracle: $GCC" >&2; exit 2; }
command -v timeout >/dev/null
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
DIR=$ROOT/tests/regression/interop
FIXTURES_M64=(psabi_pair_interop psabi_float_interop psabi_hfa_interop psabi_wide_interop
              psabi_byref_interop)
FIXTURES_M32=(psabi_float_interop psabi_byref_interop)
LEVELS=(-O0 -O2)

source "$ROOT/tests/regression/i386_exec.sh"
_i386_ensure_memo "$GCC"   # populate the PARENT-side memo before any $( ) read
i386_cap=$(i386_capability "$GCC")

count=0
unmeasured=0
for bits in -m64 -m32; do
    mode_cap=run
    [[ $bits == -m32 ]] && mode_cap=$i386_cap
    if [[ $mode_cap == none ]]; then
        echo "SKIP $bits: $GCC cannot link a 32-bit executable on this host -- unmeasured, not passed"
        unmeasured=$((unmeasured + 1))
        continue
    fi
    fixtures=("${FIXTURES_M64[@]}")
    [[ $bits == -m32 ]] && fixtures=("${FIXTURES_M32[@]}")
    for fx in "${fixtures[@]}"; do
        if ! timeout "$TIMEOUT" "$GCC" "$bits" -O2 -w -c "$DIR/${fx}_callee.c" \
                -o "$WORK/gcc-callee.o" 2>"$WORK/gcc.err" ||
           ! timeout "$TIMEOUT" "$GCC" "$bits" -O2 -w -c "$DIR/${fx}_caller.c" \
                -o "$WORK/gcc-caller.o" 2>>"$WORK/gcc.err"; then
            echo "FAIL $bits $fx: the oracle rejects the fixture" >&2
            sed -n '1,5p' "$WORK/gcc.err" >&2
            exit 1
        fi
        for level in "${LEVELS[@]}"; do
            for unit in callee caller; do
                if ! timeout "$TIMEOUT" "$CCC" "$bits" "$level" -w -c "$DIR/${fx}_$unit.c" \
                        -o "$WORK/lccc-$unit.o"; then
                    echo "FAIL $bits $level $fx: lccc cannot compile the $unit" >&2
                    exit 1
                fi
            done
            for pair in lccc:lccc lccc:gcc gcc:lccc; do
                caller=${pair%:*}
                callee=${pair#*:}
                timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/$callee-callee.o" "$WORK/$caller-caller.o" \
                    -o "$WORK/interop"
                if [[ $mode_cap != run ]]; then
                    unmeasured=$((unmeasured + 1))
                    continue
                fi
                status=0
                timeout "$TIMEOUT" "$WORK/interop" || status=$?
                if [[ $status -ne 0 ]]; then
                    echo "FAIL $bits $level $fx caller=$caller callee=$callee: exit $status" \
                         "(the number of the failing check)" >&2
                    exit 1
                fi
                count=$((count + 1))
            done
            [[ $mode_cap == run ]] && echo "PASS $bits $level $fx (three compiler pairs)"
        done
    done
done
printf 'native psABI interop: %d pair execution(s) passed, %d unmeasured (host i386 capability: %s)\n' \
    "$count" "$unmeasured" "$i386_cap"
if [[ $count -eq 0 ]]; then
    echo "FAIL: nothing was executed -- the harness or the host is broken" >&2
    exit 1
fi
if [[ $unmeasured -gt 0 && ${LCCC_REQUIRE_I686:-0} == 1 ]]; then
    echo "FAIL: LCCC_REQUIRE_I686=1 but $unmeasured i386 configuration(s) were unmeasurable" >&2
    exit 1
fi
