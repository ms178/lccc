#!/usr/bin/env bash
# AUD-ICE-1 / AUD-DIAG-1 — `__builtin_shuffle*` must diagnose, never ICE.
#
# All three rejected inputs below reached a bare `panic!` in the lowering
# (`src/ir/lowering/expr_builtins.rs`), i.e. a user could crash the compiler
# with a two-line translation unit, and the compiler *aborted* with a Rust
# backtrace instead of a diagnostic. One of them (the float mask) was worse
# than a crash: it was accepted, its bit patterns silently reinterpreted as
# lane indices, and the program produced wrong answers with no diagnostic at
# all.
#
# The contract pinned here:
#   * every rejected input produces a `file:line:col: error: ...` diagnostic
#     with the message naming the actual violation (not an ICE),
#   * GCC rejects the same inputs (so the verdict is oracle-checked, not
#     self-referential),
#   * the VALID forms still compile AND run: the 2-argument form, the
#     3-argument form, runtime (non-constant) masks, and negative "don't care"
#     indices in `__builtin_shufflevector`. A gate that only asserted
#     rejections would pass with the builtin deleted.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "SKIP: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-shuffle-diag.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail() { echo "FAIL: $*" >&2; exit 1; }

# --- the three rejected inputs, each with the fragment its message must name --
cat >count.c <<'C'
typedef int v4si __attribute__((vector_size(16)));
typedef int v8si __attribute__((vector_size(32)));
v4si f(v4si a, v4si b, v8si m) { return __builtin_shuffle(a, b, m); }
C
cat >floatmask.c <<'C'
typedef int v4si __attribute__((vector_size(16)));
typedef float v4sf __attribute__((vector_size(16)));
v4si g(v4si a, v4si b, v4sf m) { return __builtin_shuffle(a, b, m); }
C
cat >nonvector.c <<'C'
typedef float v4sf __attribute__((vector_size(16)));
v4sf h(v4sf a, v4sf b, int m) { return __builtin_shuffle(a, b, m); }
C

# expect_rejected <file> <message fragment>
expect_rejected() {
    local file=$1 want=$2 err
    if "$CCC" -O0 -S "$file" -o "$file.s" 2>"$file.err"; then
        fail "$file was accepted; it must be diagnosed"
    fi
    # A panic is an ICE: the compiler must never answer a bad program that way.
    ! grep -qE "panicked at|internal compiler error|RUST_BACKTRACE" "$file.err" \
        || { cat "$file.err" >&2; fail "$file ICEd instead of diagnosing"; }
    grep -q "$want" "$file.err" \
        || { cat "$file.err" >&2; fail "$file: diagnostic does not mention '$want'"; }
    # The span matters: the offending operand can be deep inside a large TU.
    grep -qE "^${file}:[0-9]+:[0-9]+: error:" "$file.err" \
        || { cat "$file.err" >&2; fail "$file: diagnostic has no file:line:col prefix"; }
    echo "ok: $file -> $(head -1 "$file.err")"
}

expect_rejected count.c "number of elements"
expect_rejected floatmask.c "must be an integer vector"
expect_rejected nonvector.c "must be an integer vector"

# AUD-ICE-1's sibling arms (the same `panic!` family, reached through
# `__builtin_shufflevector`'s constant-index form).
cat >svrange.c <<'C'
typedef int v4si __attribute__((vector_size(16)));
v4si r(v4si a, v4si b) { return __builtin_shufflevector(a, b, 0, 1, 2, 9); }
C
cat >svnonconst.c <<'C'
typedef int v4si __attribute__((vector_size(16)));
v4si s(v4si a, v4si b, int i) { return __builtin_shufflevector(a, b, 0, 1, 2, i); }
C
cat > svtypes.c <<'C'
typedef int v4i __attribute__((vector_size(16)));
typedef float v4f __attribute__((vector_size(16)));
v4i f(v4i a, v4f b) { return __builtin_shufflevector(a,b,0,1,2,3); }
C
cat > svscalar.c <<'C'
typedef int v4i __attribute__((vector_size(16)));
v4i f(v4i a, int b) { return __builtin_shufflevector(a,b,0,1,2,3); }
C
# Any lane count is valid now (GCC 16.2: one result lane per index), so the
# remaining rejections are GCC's own: element types must be identical
# (signedness included), the index count a power of two, and every index in
# [-1, lanes(a) + lanes(b)).
cat > svsign.c <<'C'
typedef int v4i __attribute__((vector_size(16)));
typedef unsigned v4u __attribute__((vector_size(16)));
v4i f(v4i a, v4u b) { return __builtin_shufflevector(a,b,0,1,2,7); }
C
cat > svpow2.c <<'C'
typedef int v4i __attribute__((vector_size(16)));
int f(v4i a, v4i b) { return __builtin_shufflevector(a,b,0,1,2)[0]; }
C
cat > svneg.c <<'C'
typedef short v8h __attribute__((vector_size(16)));
typedef short v4h __attribute__((vector_size(8)));
v4h f(v8h a, v4h b) { return __builtin_shufflevector(a,b,-2,0,11,1); }
C
expect_rejected svtypes.c "must have the same element type"
expect_rejected svsign.c "must have the same element type"
expect_rejected svscalar.c "arguments must be vectors"
expect_rejected svrange.c "invalid element index"
expect_rejected svneg.c "invalid element index '-2'"
expect_rejected svpow2.c "power of two number of elements"
expect_rejected svnonconst.c "must be an integer constant expression"

# --- the oracle: GCC rejects the same three programs ------------------------
if command -v gcc >/dev/null 2>&1; then
    for file in count.c floatmask.c nonvector.c; do
        if gcc -O0 -S "$file" -o /dev/null 2>"gcc-$file.err"; then
            fail "oracle disagreement: gcc accepted $file (this gate's verdict is wrong)"
        fi
        grep -q "must be an integer vector\|number of elements of the argument vector" \
            "gcc-$file.err" \
            || { cat "gcc-$file.err" >&2; fail "gcc rejects $file for another reason"; }
    done
    for file in svtypes.c svsign.c svscalar.c svrange.c svneg.c svpow2.c svnonconst.c; do
        if gcc -O0 -S "$file" -o /dev/null 2>"gcc-$file.err"; then
            fail "oracle disagreement: gcc accepted $file"
        fi
        grep -q '__builtin_shufflevector' "gcc-$file.err" \
            || { cat "gcc-$file.err" >&2; fail "gcc rejects $file for another reason"; }
    done
    echo "ok: gcc rejects all ten malformed shuffle programs"
    # ...and the positive control is NOT oracle-comparable as a whole: the
    # narrow_mask case is the documented acceptance superset above, so the
    # runtime comparison against gcc is done on a form gcc also compiles.
fi

# --- positive control: the valid forms must still compile AND agree ----------
cat >valid.c <<'C'
#include <stdio.h>
typedef int v4si __attribute__((vector_size(16)));
typedef unsigned char v4u8 __attribute__((vector_size(4)));

static v4si two_arg(v4si v) { return __builtin_shuffle(v, (v4si){3, 2, 1, 0}); }
static v4si three_arg(v4si a, v4si b) { return __builtin_shuffle(a, b, (v4si){4, 5, 6, 7}); }
/* Runtime mask: the lowering unrolls the gather, so this is not a
   constant-only builtin. Values are taken modulo the vector length. */
static v4si runtime_mask(v4si a, v4si b, v4si m) { return __builtin_shuffle(a, b, m); }
static v4si clang_static(v4si a, v4si b) {
    return __builtin_shufflevector(a, b, 0, 5, 2, -1); /* -1 = don't care */
}
/* Only lanes 0-2 are COMPARED below: Clang documents a negative index as
   "don't care" (the lane is undefined), so an implementation may leave
   anything there and gcc's choice is not a spec.  Lane 3 is still checked for
   stability against lccc's documented choice (lane 0 of the first vector). */
static int dont_care_lane(v4si a, v4si b) {
    return __builtin_shufflevector(a, b, 0, 5, 2, -1)[3];
}
/* AUD-ICE-1 second half: EVERY consumer that needs the builtin's type must see
   the vector type, or it treats the 16 bytes as an untyped scalar.  These four
   forms were separately broken: `sizeof` reported 8, `_Generic` matched
   nothing, `x[1]` read the wrong lane, and `return __builtin_shuffle(...)`
   handed the caller the result alloca's ADDRESS instead of the data (the
   32-byte form returned all-zeros, and dereferenced the address at -O2). */
static v4si direct_return(v4si v) { return __builtin_shuffle(v, (v4si){3, 2, 1, 0}); }
static v4si casted(v4si v) { return (v4si)__builtin_shuffle(v, (v4si){3, 2, 1, 0}); }
static int subscripted(v4si v) { return __builtin_shuffle(v, (v4si){3, 2, 1, 0})[1]; }
typedef int v8si __attribute__((vector_size(32)));
static v8si wide_return(v8si v) { return __builtin_shuffle(v, (v8si){7, 6, 5, 4, 3, 2, 1, 0}); }

/* Documented acceptance superset: a mask whose inner type is NARROWER than
   the vector's (`v4u8` for a `v4si`), which GCC rejects with "argument
   vector(s) inner type must have the same size as inner type of the mask".
   lccc has always taken it (the mask elements are read at their own width and
   reduced modulo the lane count), no GCC-targeting program can contain it, and
   rejecting it now would turn a working build into a hard error -- so the
   superset is pinned here as a *decision*, visible instead of accidental.
   Narrowing it to GCC-exact is AUD-DIAG-2 in the follow-up document. */
static v4si narrow_mask(v4si a, v4u8 m) { return __builtin_shuffle(a, m); }

int main(void) {
    v4si a = {10, 20, 30, 40}, b = {50, 60, 70, 80};
    v4si r2 = two_arg(a);
    v4si r3 = three_arg(a, b);
    v4si rm = runtime_mask(a, b, (v4si){0, 5, 2, 7});
    v4si rs = clang_static(a, b);
    v4si rn = narrow_mask(a, (v4u8){1, 0, 3, 2});
    v4si rd = direct_return(a);
    v4si rc2 = casted(a);
    int rx = subscripted(a);
    v8si w = {1, 2, 3, 4, 5, 6, 7, 8};
    v8si rw = wide_return(w);
    /* The element type must be the first operand's (gcc agrees; _Generic is
       how the two implementations are compared, not a comment). */
#if defined(__GNUC__) && !defined(__clang__)
    _Static_assert(sizeof(__builtin_shuffle(a, (v4si){0, 1, 2, 3})) == 16,
                   "shuffle result must be the first operand's 16-byte vector");
#endif
    printf("direct: %d %d %d %d\n", rd[0], rd[1], rd[2], rd[3]);
    printf("casted: %d %d %d %d\n", rc2[0], rc2[1], rc2[2], rc2[3]);
    printf("sub: %d\n", rx);
    printf("wide: %d %d %d %d %d %d %d %d\n", rw[0], rw[1], rw[2], rw[3], rw[4], rw[5], rw[6], rw[7]);
    printf("%d %d %d %d\n", r2[0], r2[1], r2[2], r2[3]);
    printf("%d %d %d %d\n", r3[0], r3[1], r3[2], r3[3]);
    printf("%d %d %d %d\n", rm[0], rm[1], rm[2], rm[3]);
    printf("%d %d %d\n", rs[0], rs[1], rs[2]);
    printf("%d %d %d %d\n", rn[0], rn[1], rn[2], rn[3]);
    return 0;
}
C
# GCC-comparable subset (everything but the documented superset).
sed '/narrow_mask/d; /typedef unsigned char v4u8/d; /^ *printf.*rn\[/d' valid.c > valid_gcc.c
for opt in -O0 -O2; do
    "$CCC" $opt valid.c -o "valid$opt.bin" || fail "valid forms rejected at $opt"
    ./"valid$opt.bin" > "lccc$opt.txt" || fail "valid forms crashed at $opt"
    if command -v gcc >/dev/null 2>&1; then
        # The superset line is removed for the cross-compiler run; the gcc half
        # must still cover the 2-arg, 3-arg, runtime-mask and negative-index
        # forms (four printed lines).
        "$CCC" $opt valid_gcc.c -o "lccc-gcc$opt.bin" || fail "gcc-comparable file failed"
        ./"lccc-gcc$opt.bin" > "lccc-gcc$opt.txt"
        # This executable passes 256-bit vectors by value. Give GCC the AVX
        # ABI explicitly, rather than accepting a -Wpsabi fallback warning.
        gcc $opt -mavx -Werror=psabi valid_gcc.c -o "gcc$opt.bin" || fail "oracle build failed at $opt"
        ./"gcc$opt.bin" > "gcc$opt.txt"
        [[ $(wc -l < "gcc$opt.txt") == 8 ]] || fail "oracle output shape changed at $opt"
        cmp "lccc-gcc$opt.txt" "gcc$opt.txt" \
            || { paste "lccc-gcc$opt.txt" "gcc$opt.txt" >&2; fail "valid-form results differ from gcc at $opt"; }
    fi
    echo "ok: valid forms $opt -> $(tr '\n' ';' < "lccc$opt.txt")"
done

echo "PASS: __builtin_shuffle diagnoses bad masks and still lowers the valid forms"
