#!/usr/bin/env bash
# Byte-boundary and bulk-copy ABI oracle over both by-value aggregate families:
# a size sweep of a byte-array blob (INTEGER class, partial stack words, the bulk
# `rep movsb` threshold) plus a fixed set of aggregates whose *tail eightbyte is
# partial* (SSE and mixed classes).  The same fixture probes mmap guard
# pages, two aggregate arguments, scalar/F64 neighbors, regparm -> cdecl,
# fastcall, and over-aligned local sources. Both GCC/LCCC directions matter.
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
# A second fixture, because the sweep below varies `unsigned char[N]` and an
# array of bytes is *always* INTEGER-classified: the SSE and mixed
# ([INTEGER,SSE], [SSE,INTEGER]) register paths would never be touched no matter
# how many sizes were tried.  That blind spot cost a real over-read -- see the
# comment on emit_small_aggregate_eightbyte_load in src/backend/x86/codegen/calls.rs.
SRC_PARTIAL=$ROOT/tests/regression/small_aggregate_partial_eightbyte.c

# This header advertises what the fixture probes -- guard pages, two aggregate
# arguments, scalar/F64 neighbours, regparm -> cdecl, fastcall, over-aligned
# locals -- and a comment that outlives the code it describes is worse than no
# comment: a reader (or a reviewer scoring the ABI work) cannot tell a covered
# case from an aspirational one.  So the claims are checked against the fixture
# here, at the only place that can see both, and the sizes below are what the
# matrix actually sweeps.
for claim in 'regparm(3)' 'fastcall' 'mprotect' 'PROT_NONE' 'aligned(64)' 'double' 'struct blob'; do
    grep -q -- "$claim" "$SRC" || {
        echo "FAIL: $0 advertises '$claim' but $SRC no longer contains it" >&2
        exit 1
    }
done
grep -c 'struct blob' "$SRC" | { read -r n; [ "$n" -ge 4 ] || {
    echo "FAIL: the fixture must keep *two* by-value aggregate arguments on both" >&2
    echo "      sides of the call (saw $n 'struct blob' mentions)" >&2
    exit 1
}; }
INC=$("$GCC" -print-file-name=include)
printf 'int main(void) { return 0; }\n' >"$WORK/preflight.c"
count=0
for bits in -m64 -m32; do
    timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/preflight.c" -o "$WORK/preflight"
    timeout "$TIMEOUT" "$WORK/preflight"
    for opt in -O0 -O1 -O2 -O3 -Os; do
        # Every partial stack-word size plus both sides of the bulk threshold.
        #
        # The 1..15 run is not padding for its own sake: x86-64 SysV passes an
        # aggregate in registers only when it is <= 16 bytes *and* the eightbyte
        # split is legal, and a `unsigned char[N]` struct has alignment 1 -- so the
        # sizes around 8 and 16 are exactly where "same bytes, different
        # classification" would show up as a caller that wrote through registers
        # while the callee read the stack.  lccc and gcc are compiled
        # independently on both sides and paired in all three mixtures, so a
        # disagreement is visible at the boundary instead of only past it.
        for size in 1 7 8 9 15 16 17 18 19 20 21 22 23 31 2047 2048 2049 4097; do
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
                if ! timeout "$TIMEOUT" "$WORK/mixed"; then
                    echo "FAIL $bits $opt size=$size caller=$caller callee=$callee" >&2
                    exit 1
                fi
                count=$((count + 1))
            done
            printf 'PASS %s %s size=%s (three compiler pairs)\n' "$bits" "$opt" "$size"
        done
    done

    # Partial tail eightbytes, per class.  The callee folds *exactly* sizeof bytes
    # of each aggregate and neighbouring scalars ride between them, so a wrong
    # class, a shifted register or a skipped argument changes the digest; each
    # object also sits flush against a PROT_NONE page, so reading a rounded-up
    # eightbyte is a fault rather than a silent neighbour read.
    for opt in -O0 -O2; do
        for key in gcc lccc; do
            cc=$GCC; [[ $key != lccc ]] || cc=$CCC
            timeout "$TIMEOUT" "$cc" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                -DCALLER_ONLY -c "$SRC_PARTIAL" -o "$WORK/p-$key-caller.o"
            timeout "$TIMEOUT" "$cc" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                -DCALLEE_ONLY -c "$SRC_PARTIAL" -o "$WORK/p-$key-callee.o"
        done
        for pair in lccc:lccc lccc:gcc gcc:lccc; do
            caller=${pair%:*}; callee=${pair#*:}
            timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/p-$caller-caller.o" \
                "$WORK/p-$callee-callee.o" -o "$WORK/p-mixed"
            if ! timeout "$TIMEOUT" "$WORK/p-mixed"; then
                echo "FAIL partial-eightbyte $bits $opt caller=$caller callee=$callee" >&2
                exit 1
            fi
            count=$((count + 1))
        done
        printf 'PASS %s %s partial-eightbyte classes (three compiler pairs)\n' "$bits" "$opt"
    done
done
printf 'aggregate-call ABI: %d native executions passed\n' "$count"
