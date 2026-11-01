#!/usr/bin/env bash
# A symbol+index address is computed at FULL register width, whatever the index's type.
#
# x86's SIB byte consumes the index register's whole 64-bit value. lccc therefore
# has to normalise a narrow index before a SIB can use it: sign-extend a signed
# narrow value, zero-extend an unsigned one. Four folds do that, and the law is
# stated once in `ensure_sib_index_form` (`src/backend/x86/codegen/memory.rs`);
# the symbol+index LEA arm used to skip it, so the SAME `base + i` expression
# lowered correctly when the emitter chose a load fold and incorrectly when it
# chose the LEA fold.
#
# A negative index is the cheapest way to see the difference, and it is not a
# corner case: `table[i]` with a signed `int` index is ordinary C. An index of
# -8 whose register holds 0x0000_0000_FFFF_FFF8 (the shape a 32-bit move leaves
# in a 64-bit register) addresses 4 GiB above the table if it is consumed raw
# instead of sign-extended — a wrong answer, or a crash, or a silently different
# array.
#
# So this gate states the property executably, over the three narrow index
# widths a C program actually uses:
#
#   1. a signed `int` index drives a global table through a symbol+index
#      address, including negative indices;
#   2. the same for a signed `short` index (the I16 extension arm) and an
#      unsigned `char` index (the U8 zero-extension arm);
#   3. a 64-bit index still gives exactly the same answer, so the arms agree;
#   4. every level from -O0 to -Os, with and without -g, prints gcc's checksum.
#
# The gate pins VALUES, not a particular emitter arm: which fold the backend
# picks for a given GEP is an internal decision, and a test that asserted one
# arm would fail when a better fold is added. The arm-level mechanism is pinned
# by the unit tests next to `ensure_sib_index_form`.
#
# Usage: bash tests/regression/check_symbol_index_widths.sh
#        CCC=/path/to/lccc GCC_ORACLE=/usr/bin/gcc bash tests/regression/check_symbol_index_widths.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
GCC_ORACLE=${GCC_ORACLE:-gcc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ ! -x "$CCC" ]; then
    echo "FAIL: lccc not found at $CCC" >&2
    exit 1
fi
if ! command -v "$GCC_ORACLE" >/dev/null 2>&1; then
    echo "SKIP: no reference compiler '$GCC_ORACLE' on PATH" >&2
    exit 0
fi

work=$TMP/work
mkdir -p "$work"

cat >"$work/index.c" <<'EOF'
#include <stdio.h>

/* A file-scope table: the address is a symbol, so `table + i` is a
 * symbol+index address and every access is a candidate for the SIB folds. */
static long table[96];

static void fill(void) {
    for (int i = 0; i < 96; i++) table[i] = (long)i * 2654435761L + 7;
}

/* A positive bias so the interesting indices are NEGATIVE while the address
 * stays inside the object: `mid = table + 48` is a symbol+constant base. */
#define MID (table + 48)

__attribute__((noinline)) long via_int(int i) { return MID[i]; }

__attribute__((noinline)) long via_short(short i) { return MID[i]; }

__attribute__((noinline)) long via_uchar(unsigned char i) { return MID[i]; }

__attribute__((noinline)) long via_long(long i) { return MID[i]; }

int main(void) {
    fill();
    unsigned long s = 0;
    /* Negative down to -40 (well inside the table), positive past the bias. */
    for (int i = -40; i <= 47; i++) {
        s = s * 1000003UL + (unsigned long)via_int(i);
        s = s * 1000003UL + (unsigned long)via_short((short)i);
        s = s * 1000003UL + (unsigned long)via_long((long)i);
        if (i >= 0) s = s * 1000003UL + (unsigned long)via_uchar((unsigned char)i);
    }
    printf("%lu\n", s);
    return 0;
}
EOF

note() { printf '  %-8s %s\n' "$1" "$2"; }
fail=0
ok() { note ok "$1"; }
bad() {
    note FAIL "$1"
    fail=1
}

# The reference is gcc's own translation of the same source, so a checksum
# difference is a difference in the compiled program, not in the harness.
"$GCC_ORACLE" -O2 -o "$work/index.ref" "$work/index.c" 2>/dev/null \
    || { echo "FAIL: gcc oracle failed to build the fixture" >&2; exit 1; }
want=$("$work/index.ref") || { echo "FAIL: gcc oracle binary did not run" >&2; exit 1; }

for opt in -O0 -O1 -O2 -O3 -Os; do
    for debug in "" "-g"; do
        tag="$(printf '%s%s' "$opt" "${debug:-}")"
        if ! "$CCC" "$opt" $debug -o "$work/index.bin" "$work/index.c" 2>"$work/err"; then
            bad "$tag: lccc failed to build ($(head -1 "$work/err"))"
            continue
        fi
        if ! got=$("$work/index.bin"); then
            bad "$tag: the program did not run to completion"
            continue
        fi
        if [ "$got" = "$want" ]; then
            ok "$tag: $got"
        else
            bad "$tag: printed $got, gcc prints $want"
        fi
    done
done

if [ "$fail" -ne 0 ]; then
    echo "FAIL: symbol+index addressing at narrow widths disagrees with gcc" >&2
    exit 1
fi
echo "PASS: symbol+index addressing is computed at full register width (narrow, signed and unsigned indices agree with gcc at -O0..-Os)"
