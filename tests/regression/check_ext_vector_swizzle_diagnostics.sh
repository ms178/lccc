#!/usr/bin/env bash
# Clang ext_vector_type component access ("swizzle") diagnostics.
#
# `v.x` on an ext vector used to resolve like a missing struct field (offset
# 0, type int, no diagnostic). The component grammar is now checked in sema
# and the assignable forms are lowered lane by lane; every form that Clang
# rejects must be rejected HERE with a `file:line:col: error:` diagnostic
# that names the violation -- never accepted and miscompiled, never an ICE.
#
# GCC ignores `ext_vector_type` (it has no component access), so the
# rejections are checked against Clang's documented messages instead of a
# running oracle; the one GCC-expressible case (assigning a 16-byte vector
# to an 8-byte one) is oracle-checked when gcc is present. The positive
# control compiles and RUNS the accepted forms, so the gate cannot pass by
# rejecting everything.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "FAIL: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-swizzle-diag.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail() { echo "FAIL: $*" >&2; exit 1; }

hdr='typedef float F4 __attribute__((ext_vector_type(4)));
typedef float F2 __attribute__((ext_vector_type(2)));
typedef long double L3 __attribute__((ext_vector_type(3)));'

# case <file> <body statement(s)>
mk() { printf '%s\nvoid f(F4 v, F2 w, L3 l) { %s }\n' "$hdr" "$2" >"$1"; }
mk badname.c 'float x = v.q;'
mk mixedsets.c 'float x = v.xr;'
mk range.c 'float x = v.s4;'
mk range_chain.c 'F2 r = v.zw.z;'
mk range_hi.c 'float r = w.hi.y;'
mk dup.c 'v.xx = w;'
mk compound.c 'v.xy += w;'
mk inner_rvalue.c 'v.lo.x = 1;'
mk width.c 'v.xy = v;'
mk width_ld.c 'l.xy = l;'
mk nolanes.c 'F2 r = v.s;'

# expect_rejected <file> <message fragment>
expect_rejected() {
    local file=$1 want=$2
    if "$CCC" -O0 -S "$file" -o "$file.s" 2>"$file.err"; then
        fail "$file was accepted; it must be diagnosed"
    fi
    ! grep -qE "panicked at|internal compiler error|RUST_BACKTRACE" "$file.err" \
        || { cat "$file.err" >&2; fail "$file ICEd instead of diagnosing"; }
    grep -q "$want" "$file.err" \
        || { cat "$file.err" >&2; fail "$file: diagnostic does not mention '$want'"; }
    grep -qE "^${file}:[0-9]+:[0-9]+: error:" "$file.err" \
        || { cat "$file.err" >&2; fail "$file: diagnostic has no file:line:col prefix"; }
    echo "ok: $file -> $(head -1 "$file.err")"
}

expect_rejected badname.c "illegal vector component name 'q'"
expect_rejected mixedsets.c "illegal vector component name 'xr'"
expect_rejected nolanes.c "illegal vector component name 's'"
expect_rejected range.c "vector component access 's4' exceeds type"
expect_rejected range_chain.c "vector component access 'z' exceeds type"
# `w.hi` on a two-lane vector is the scalar lane itself (Clang: "member
# reference base type 'float' is not a structure or union").
expect_rejected range_hi.c "request for member 'y' in something not a structure or union"
mk scalar.c 'float x = 1; float y = x.y;'
expect_rejected scalar.c "request for member 'y' in something not a structure or union"
expect_rejected dup.c "duplicate components 'xx'"
expect_rejected compound.c "compound assignment to a multi-component vector swizzle"
expect_rejected inner_rvalue.c "expression is not assignable"
expect_rejected width.c "incompatible types"
expect_rejected width_ld.c "incompatible types"

# --- oracle for the one GCC-expressible rejection -----------------------------
cat >gnu_width.c <<'C'
typedef float V4 __attribute__((vector_size(16)));
typedef float V2 __attribute__((vector_size(8)));
void f(V4 v, V2 w) { w = v; }
C
expect_rejected gnu_width.c "incompatible types"
if command -v gcc >/dev/null 2>&1; then
    if gcc -O0 -S gnu_width.c -o /dev/null 2>gcc-gnu_width.err; then
        fail "oracle disagreement: gcc accepted gnu_width.c"
    fi
    grep -q "incompatible types" gcc-gnu_width.err \
        || { cat gcc-gnu_width.err >&2; fail "gcc rejects gnu_width.c for another reason"; }
    echo "ok: gcc rejects gnu_width.c too (verdict is oracle-checked)"
fi

# --- positive control: the accepted forms compile AND run ---------------------
cat >valid.c <<'C'
typedef float F4 __attribute__((ext_vector_type(4)));
typedef float F2 __attribute__((ext_vector_type(2)));
typedef unsigned U2 __attribute__((vector_size(8)));
typedef int I2 __attribute__((vector_size(8)));
int main(void) {
    F4 v = {1, 2, 3, 4};
    F2 w = v.hi;          /* halving names */
    v.xy = w;             /* same-width swizzle store */
    v.zw = v.yx;          /* swizzle from swizzle (reads before writes) */
    v.w = 9;              /* single component, scalar */
    v.lo = 5;             /* scalar splat over a swizzle (Clang) */
    F2 r = (v.xy = w.yx); /* the assignment's value */
    I2 a = {1, 2};
    U2 b = {3, 4};
    a = b;                /* same-size lax vector conversion: still accepted */
    return !(v.x == 4 && v.y == 3 && v.z == 4 && v.w == 9 && r.x == 4 && r.y == 3 && a[1] == 4);
}
C
"$CCC" -O2 valid.c -o valid 2>valid.err || { cat valid.err >&2; fail "positive control did not compile"; }
./valid || fail "positive control computed the wrong lanes"
echo "ok: positive control compiles and runs"
echo "PASS: ext-vector swizzle diagnostics ($(ls ./*.c | wc -l) fixtures)"
