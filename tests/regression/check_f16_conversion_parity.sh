#!/usr/bin/env bash
# Gate: `-mf16c` must not change a single binary16 -> binary32 VALUE.
#
# `has_hw_f16_convert()` (ir/lowering/half_float.rs) decides between the single
# `vcvtph2ps` and the branchless software sequence.  Two ways that selector can
# be wrong, and neither is visible from the assembly alone:
#
#   * the hardware path computes a different value (a real wrong-code bug), or
#   * the selector reads a feature set that disagrees with the macro the driver
#     announces.  That was live on i686: `-m32 -mf16c -mavx` defined `__F16C__`
#     and then quietly took the SOFTWARE path, so `-mf16c` bought nothing while
#     claiming the extension.  Caught by comparing the two builds.
#
# So the check is a self-differential: build the same source twice, once with
# the extension and once without, and require the printed float bits to match
# exactly.  A reference compiler is then required to agree with LCCC on the
# extension-free build, which pins the software sequence itself.
#
# Usage:
#   tests/regression/check_f16_conversion_parity.sh [--lccc PATH] [--reference CC]
set -euo pipefail

LCCC=""
REFERENCE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "check_f16_conversion_parity: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$ROOT/tests/regression/f16_conversion_parity.c"
CC="${LCCC:-$ROOT/target/fastbuild/lccc}"
[ -f "$SRC" ] || { echo "FAIL: missing source $SRC" >&2; exit 1; }
[ -x "$CC" ] || { echo "FAIL: lccc binary '$CC' is not executable" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "check_f16_conversion_parity: binary16 -> binary32"

# shellcheck disable=SC2086
$CC -O2 -w "$SRC" -o "$WORK/sw" 2>"$WORK/a.log" \
    || { echo "FAIL: cannot build the software path" >&2; cat "$WORK/a.log" >&2; exit 1; }
$CC -O2 -w -mf16c -mavx "$SRC" -o "$WORK/hw" 2>"$WORK/b.log" \
    || { echo "FAIL: cannot build the F16C path" >&2; cat "$WORK/b.log" >&2; exit 1; }
"$WORK/sw" > "$WORK/sw.txt"
"$WORK/hw" > "$WORK/hw.txt"

rows=$(wc -l < "$WORK/sw.txt")
if [ "$rows" -lt 14 ]; then
    echo "FAIL: expected at least 14 pattern rows, got $rows" >&2
    exit 1
fi
if ! diff -u "$WORK/sw.txt" "$WORK/hw.txt" > "$WORK/d.txt"; then
    echo "FAIL: -mf16c changed the converted values (software vs hardware):" >&2
    cat "$WORK/d.txt" >&2
    exit 1
fi
echo "  ok  $rows patterns identical with and without -mf16c"

# The extension must actually ENGAGE, or the equality above is trivially true
# because both builds took the software path.
# shellcheck disable=SC2086
if $CC -O2 -w -mf16c -mavx -S "$SRC" -o "$WORK/hw.s" 2>/dev/null; then
    if grep -q "vcvtph2ps" "$WORK/hw.s"; then
        echo "  ok  -mf16c emits vcvtph2ps (the hardware path really engages)"
    else
        echo "FAIL: -mf16c is accepted but no vcvtph2ps is emitted: the option" >&2
        echo "      buys nothing while __F16C__ claims the extension" >&2
        exit 1
    fi
fi

if [ -n "$REFERENCE" ]; then
    if ! command -v "$REFERENCE" >/dev/null 2>&1; then
        echo "  note reference '$REFERENCE' not found: SKIP"
    else
        # shellcheck disable=SC2086
        $REFERENCE -O2 -w "$SRC" -o "$WORK/ref" 2>/dev/null \
            && "$WORK/ref" > "$WORK/ref.txt" \
            || { echo "  note reference cannot build the fixture: SKIP"; exit 0; }
        if ! diff -u "$WORK/ref.txt" "$WORK/sw.txt" > "$WORK/r.txt"; then
            echo "FAIL: LCCC's software path disagrees with $REFERENCE:" >&2
            cat "$WORK/r.txt" >&2
            exit 1
        fi
        echo "  ok  LCCC matches $REFERENCE on all $rows patterns"
    fi
fi

# --- i686 leg -------------------------------------------------------------
# The defect this leg exists for was on THIS backend: `-m32 -mf16c -mavx`
# defined `__F16C__` and then silently took the software path, because
# `effective_x86_features()` derived `F16c` from `isa.avx`, which `x86_isa()`
# pins to NONE for i686.  The option bought nothing while claiming the
# extension -- and both paths compute CORRECT values, so only the pair
# (identical output) AND (vcvtph2ps present) catches it.
#
# The hosted fixture needs <stdio.h>, and this host has no 32-bit libc headers,
# so the i686 leg uses the freestanding sibling and runs it natively.
I686_SRC="$ROOT/tests/regression/i686_fma/f16_i686_freestanding.c"
I686_FLAGS="-m32 -O2 -w -ffreestanding -nostdlib -static -fno-pic"
case "$(uname -m)" in
x86_64|amd64)
    if [ -f "$I686_SRC" ]; then
        # shellcheck disable=SC2086
        if $CC $I686_FLAGS "$I686_SRC" -o "$WORK/i686_sw" 2>"$WORK/c.log" \
           && $CC $I686_FLAGS -mf16c -mavx "$I686_SRC" -o "$WORK/i686_hw" 2>>"$WORK/c.log"; then
            "$WORK/i686_sw" > "$WORK/i686_sw.txt"
            "$WORK/i686_hw" > "$WORK/i686_hw.txt"
            if ! diff -u "$WORK/i686_sw.txt" "$WORK/i686_hw.txt" > "$WORK/i686_d.txt"; then
                echo "FAIL: -mf16c changed the i686 converted values:" >&2
                cat "$WORK/i686_d.txt" >&2
                exit 1
            fi
            irows=$(wc -l < "$WORK/i686_sw.txt")
            echo "  ok  i686: $irows patterns identical with and without -mf16c (executed)"
            # shellcheck disable=SC2086
            $CC $I686_FLAGS -mf16c -mavx -S "$I686_SRC" -o "$WORK/i686_hw.s" 2>/dev/null
            if grep -q "vcvtph2ps" "$WORK/i686_hw.s"; then
                echo "  ok  i686: -mf16c emits vcvtph2ps"
            else
                echo "FAIL: i686 accepts -mf16c but emits no vcvtph2ps: __F16C__ is" >&2
                echo "      announced while the option buys nothing" >&2
                exit 1
            fi
        else
            echo "  note i686 leg could not build (no -m32 support here): SKIP"
        fi
    fi
    ;;
*) echo "  note i686 leg needs an x86-64 host to execute: SKIP" ;;
esac

echo "PASS: binary16 conversion is value-identical with and without F16C"
