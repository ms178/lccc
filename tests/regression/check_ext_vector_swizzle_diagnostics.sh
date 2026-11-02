#!/usr/bin/env bash
# Clang ext_vector_type component access ("swizzles"), checked against the
# Clang 23.1.4 oracle -- never against expectations written by hand.
#
# GCC has no ext_vector_type, so Clang is the only reference. Every verdict
# and every diagnostic below is what that Clang itself produces on the same
# file, at gate run time:
#
#   1. ext_vector_select/errors.c: LCCC's `file:line:col: error: message` list
#      equals Clang's, entry for entry. The one normalisation: a vector type's
#      spelling directly before its "(vector of N 'elem' values)" description
#      becomes T (LCCC types carry no typedef names); the description itself
#      -- lane count and element type -- is still compared verbatim.
#   2. Per-form fixtures (chained / halving / scalar member access, width
#      mismatches, compound and nested swizzle stores): LCCC accepts exactly
#      what Clang accepts; on a rejection the first error is positioned at
#      Clang's line:col and is a diagnostic, not an ICE.
#   3. ext_vector_select/select.c (values, stores, compound assignment,
#      ++/--, nested selection, single evaluation of the base, aliasing, the
#      exact footprint of 10-byte x87 lanes): LCCC -O0/-O1/-O2 on -m64 and
#      -m32 prints byte-for-byte what Clang -O0 prints and exits 0. Clang runs
#      at -O0 only: Clang 23.1.4 miscompiles `<N x x86_fp80>` at -O1+.
#   4. The one GCC-expressible rejection (a 16-byte GNU vector assigned to an
#      8-byte one) is checked against GCC 16 as well.
#
# A missing or different Clang FAILS (tests/regression/lib/clang_oracle.sh):
# a gate that skipped its oracle would report a comparison that never ran.
# The -m32 legs need i686 execution; LCCC_REQUIRE_I686=1 makes it mandatory.
set -euo pipefail

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "FAIL: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")
export LC_ALL=C

# shellcheck source=tests/regression/lib/clang_oracle.sh
. "$root/tests/regression/lib/clang_oracle.sh"
require_clang_oracle check_ext_vector_swizzle_diagnostics || exit 1
CLANG_BIN=$CLANG_ORACLE_BIN
# shellcheck source=tests/regression/i386_exec.sh
source "$root/tests/regression/i386_exec.sh"

fixtures=$root/tests/regression/ext_vector_select
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-swizzle-diag.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail() { echo "FAIL: $*" >&2; exit 1; }
no_ice() { # <stderr file> <what>
    ! grep -qE "panicked at|internal compiler error|RUST_BACKTRACE" "$1" \
        || { cat "$1" >&2; fail "$2 ICEd instead of diagnosing"; }
}

# --- 1. errors.c: the full diagnostic list equals Clang's -------------------
cp "$fixtures/errors.c" errors.c
errors_of() { # <stderr file> -> normalised `errors.c:L:C: error: msg` lines
    grep -E '^errors\.c:[0-9]+:[0-9]+: error: ' "$1" \
        | sed -E "s/'[^']*' (\(vector of [0-9]+ '[^']*' values?\))/T \1/g" || true
}
if "$CLANG_BIN" -fsyntax-only -ferror-limit=0 errors.c 2>clang-errors.err; then
    fail "oracle: Clang accepted errors.c (the fixture must be all errors)"
fi
"$CCC" -O0 -S errors.c -o errors.s 2>lccc-errors.err \
    && fail "LCCC accepted errors.c; Clang rejects it"
no_ice lccc-errors.err errors.c
errors_of clang-errors.err >clang-errors.txt
errors_of lccc-errors.err >lccc-errors.txt
[[ -s clang-errors.txt ]] || fail "oracle: no positioned errors from Clang on errors.c"
if ! diff -u clang-errors.txt lccc-errors.txt >errors.diff; then
    cat errors.diff >&2
    fail "errors.c: LCCC's diagnostics differ from Clang $CLANG_ORACLE_VERSION's (- Clang, + LCCC)"
fi
echo "ok: errors.c -- $(wc -l <clang-errors.txt) diagnostics, line:col and message equal to Clang"

# --- 2. per-form verdicts and positions ---------------------------------------
hdr='typedef float F4 __attribute__((ext_vector_type(4)));
typedef float F2 __attribute__((ext_vector_type(2)));
typedef long double L3 __attribute__((ext_vector_type(3)));'
mk() { printf '%s\nvoid f(F4 v, F2 w, L3 l) { %s }\n' "$hdr" "$2" >"$1"; }
mk range_chain.c 'F2 r = v.zw.z;'      # beyond an inner two-lane selection
mk range_hi.c 'float r = w.hi.y;'      # hi of two lanes is the scalar lane
mk scalar.c 'float x = 1; float y = x.y;'
mk width.c 'v.xy = v;'                 # 4 lanes into a 2-lane swizzle
mk width_ld.c 'l.xy = l;'              # ... on 10-byte x87 lanes
mk compound.c 'v.xy += w;'             # Clang accepts compound swizzle stores
mk inner_lvalue.c 'v.lo.x = 1;'        # ... and stores through nested selections
mk splat.c 'v.lo = 5;'                 # a scalar splats over a swizzle
mk odd_hi.c 'long double t = l.hi.x;'  # odd N: hi = lanes {2, 3}

nforms=0
for f in range_chain range_hi scalar width width_ld compound inner_lvalue splat odd_hi; do
    nforms=$((nforms + 1))
    crc=0; "$CLANG_BIN" -fsyntax-only "$f.c" 2>"$f.clang.err" || crc=$?
    lrc=0; "$CCC" -O0 -S "$f.c" -o "$f.s" 2>"$f.lccc.err" || lrc=$?
    no_ice "$f.lccc.err" "$f.c"
    if [[ $crc -eq 0 ]]; then
        [[ $lrc -eq 0 ]] || { cat "$f.lccc.err" >&2; fail "$f.c: Clang accepts it, LCCC rejects it"; }
        echo "ok: $f.c accepted (as by Clang)"
        continue
    fi
    [[ $lrc -ne 0 ]] || fail "$f.c: Clang rejects it, LCCC accepted it"
    cpos=$(grep -m1 -oE "^$f\.c:[0-9]+:[0-9]+: error:" "$f.clang.err" || true)
    lpos=$(grep -m1 -oE "^$f\.c:[0-9]+:[0-9]+: error:" "$f.lccc.err" || true)
    [[ -n $cpos ]] || fail "oracle: Clang's rejection of $f.c has no positioned error"
    [[ $lpos == "$cpos" ]] \
        || { cat "$f.clang.err" "$f.lccc.err" >&2; fail "$f.c: first error at '${lpos:-none}', Clang's at '$cpos'"; }
    echo "ok: $f.c rejected at ${cpos%: error:} -> $(grep -m1 ': error:' "$f.lccc.err" | sed 's/^[^ ]* //')"
done

# --- 3. select.c: runtime output equals Clang -O0 on every leg ---------------
cp "$fixtures/select.c" select.c
"$CLANG_BIN" -m64 -O0 -w select.c -o clang64 || fail "oracle: Clang -m64 cannot build select.c"
./clang64 >expected.txt || { cat expected.txt >&2; fail "oracle: Clang -m64 select.c reports failures"; }
legs=0
for bits in 64 32; do
    if [[ $bits == 32 ]]; then
        i386_rc=0; i386_run_or_skip "$CLANG_BIN" || i386_rc=$?
        [[ $i386_rc -eq 77 ]] && continue
        [[ $i386_rc -eq 0 ]] || exit 1
        "$CLANG_BIN" -m32 -O0 -w select.c -o clang32 || fail "oracle: Clang -m32 cannot build select.c"
        ./clang32 >clang32.txt || fail "oracle: Clang -m32 select.c reports failures"
        cmp -s expected.txt clang32.txt || fail "oracle: Clang -m32 and -m64 disagree on select.c"
    fi
    for opt in -O0 -O1 -O2; do
        "$CCC" -m$bits $opt -w select.c -o "lccc$bits$opt" 2>build.err \
            || { cat build.err >&2; fail "select.c does not build with LCCC -m$bits $opt"; }
        rc=0; "./lccc$bits$opt" >got.txt || rc=$?
        if ! cmp -s expected.txt got.txt || [[ $rc -ne 0 ]]; then
            diff -u expected.txt got.txt >&2 || true
            fail "select.c: LCCC -m$bits $opt differs from Clang -O0 (exit $rc)"
        fi
        legs=$((legs + 1))
    done
done
echo "ok: select.c -- $(wc -l <expected.txt) lines equal to Clang -O0 on $legs LCCC legs"

# --- 4. the GCC-expressible rejection, against GCC 16 too ---------------------
cat >gnu_width.c <<'C'
typedef float V4 __attribute__((vector_size(16)));
typedef float V2 __attribute__((vector_size(8)));
void f(V4 v, V2 w) { w = v; }
C
"$CCC" -O0 -S gnu_width.c -o gnu_width.s 2>gnu_width.err \
    && fail "gnu_width.c was accepted; GCC and Clang reject it"
no_ice gnu_width.err gnu_width.c
"$CLANG_BIN" -fsyntax-only gnu_width.c 2>/dev/null && fail "oracle: Clang accepted gnu_width.c"
gcc_bin=${GCC:-gcc-16}
if command -v "$gcc_bin" >/dev/null 2>&1; then
    "$gcc_bin" -O0 -S gnu_width.c -o /dev/null 2>/dev/null \
        && fail "oracle disagreement: $gcc_bin accepted gnu_width.c"
    echo "ok: gnu_width.c rejected by LCCC, Clang and $gcc_bin"
else
    echo "ok: gnu_width.c rejected by LCCC and Clang ($gcc_bin absent)"
fi
echo "PASS: ext-vector swizzles match Clang $CLANG_ORACLE_VERSION (errors.c, $nforms forms, select.c x $legs legs)"
