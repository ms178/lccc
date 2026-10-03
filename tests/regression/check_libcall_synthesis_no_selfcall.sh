#!/usr/bin/env bash
# A13 / A14 structural gate: which library calls an optimisation may create.
#
# The loop-idiom pass (`loop_idiom`, `loop_memset`) and the stdio fold
# (`fortify_fold`) replace user code with library calls.  Before this gate
# existed, lccc emitted:
#   * `call memmove@PLT` inside a TU-defined `memcpy` (unbounded recursion on
#     any libc whose memmove calls memcpy),
#   * `call memcpy@PLT` in a TU that defines its own `memcpy` (a program's
#     definition preempts the shared library's, so the copy silently changed
#     meaning),
#   * `call memset@PLT` inside a TU-defined `memset` with a constant fill,
#   * a self-recursive `call puts@PLT` inside a TU-defined `puts` via the
#     printf->puts fold (GCC 16.2, Clang 23.1 and ICX 2025 still do this one;
#     GCC 14.2 SIGSEGVs), and
#   * a constant-size `memset`/`memcpy` call expanded inline with the
#     *library* semantics even when the TU defines the callee (A14b: the
#     pre-fix code stored `0x0707...07` where GCC called the TU's function).
#
# Checks, on the emitted assembly plus runtime rows:
#   1. refusals   — a TU that defines memcpy/memmove/memset/puts gets no
#                   synthesised call to those symbols (-O2, -O3, -Os);
#   2. controls   — a TU that does *not* define them still gets the
#                   synthesised calls (the optimisation must not be lost);
#   3. flags      — -fno-builtin / -ffreestanding / -fno-builtin-<fn> stop
#                   synthesis, -fbuiltin / -fhosted restore it, and a
#                   per-name withdrawal stays per-name (GCC 16.2 contract);
#   4. expansion  — constant-size memset/memcpy calls are expanded by default
#                   and kept as calls under the withdrawal flags and in a TU
#                   that defines the callee; the `__*_chk` fortified forms
#                   stay inlined under every withdrawal (both oracles);
#   5. runtime    — the refusing programs run and produce the C semantics,
#                   including a forced (non-foldable) call to a TU-defined
#                   memset and the original puts shape.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-libcall-gate.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fails=0

note() { printf '  %s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*" >&2; fails=$((fails + 1)); }

# ── fixtures ────────────────────────────────────────────────────────────────
cat >"$tmp/def_memcpy.c" <<'C'
typedef __SIZE_TYPE__ size_t;
void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *p = d; const unsigned char *q = s;
    for (size_t i = 0; i < n; i++) p[i] = q[i];
    return d;
}
C
cat >"$tmp/def_memmove.c" <<'C'
typedef __SIZE_TYPE__ size_t;
void *memmove(void *d, const void *s, size_t n) {
    unsigned char *p = d; const unsigned char *q = s;
    if (p < q) for (size_t i = 0; i < n; i++) p[i] = q[i];
    else for (size_t i = n; i-- > 0;) p[i] = q[i];
    return d;
}
C
cat >"$tmp/def_memset.c" <<'C'
typedef __SIZE_TYPE__ size_t;
void *memset(void *d, int c, size_t n) {
    unsigned char *p = d;
    for (size_t i = 0; i < n; i++) p[i] = 0; /* constant fill: pass target */
    return d;
}
C
cat >"$tmp/def_puts.c" <<'C'
#include <stdio.h>
int puts(const char *s) { printf("hello\n"); return s != 0 ? 0 : -1; }
C
cat >"$tmp/free_copy.c" <<'C'
typedef __SIZE_TYPE__ size_t;
void cp(unsigned char *restrict d, const unsigned char *restrict s, size_t n) {
    for (size_t i = 0; i < n; i++) d[i] = s[i];
}
C
cat >"$tmp/free_fill.c" <<'C'
typedef __SIZE_TYPE__ size_t;
void zf(unsigned char *restrict d, size_t n) {
    for (size_t i = 0; i < n; i++) d[i] = 0;
}
C
cat >"$tmp/free_puts.c" <<'C'
#include <stdio.h>
int g(void) { printf("hello\n"); return 0; }
C
# A TU-local memcpy (static) plus an ordinary copy loop: the loop must stay.
cat >"$tmp/static_memcpy.c" <<'C'
typedef __SIZE_TYPE__ size_t;
static void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *p = d; const unsigned char *q = s;
    while (n--) p[n] = q[n];
    return d;
}
void cp(unsigned char *restrict d, const unsigned char *restrict s, size_t n) {
    for (size_t i = 0; i < n; i++) d[i] = s[i];
}
C
# Constant-size calls for the A14 expansion rows (declarations only: these
# TUs do not define the callees).
cat >"$tmp/const_calls.c" <<'C'
typedef __SIZE_TYPE__ size_t;
void *memset(void *d, int c, size_t n);
void *memcpy(void *d, const void *s, size_t n);
int f(void) { char b[8]; memset(b, 7, 8); return b[0]; }
int g(void) { char b[4]; memcpy(b, "abcd", 4); return b[0]; }
C
cat >"$tmp/fortify_call.c" <<'C'
#include <string.h>
int f(void) { char b[8]; memset(b, 7, 8); return b[0]; }
C
cat >"$tmp/tu_def_const_call.c" <<'C'
typedef __SIZE_TYPE__ size_t;
void *memset(void *d, int c, size_t n) {   /* ignores c */
    unsigned char *p = d;
    for (size_t i = 0; i < n; i++) p[i] = 0;
    return d;
}
int f(void) { char b[8]; memset(b, 7, 8); return b[0]; }
C
cat >"$tmp/tu_static_const_call.c" <<'C'
typedef __SIZE_TYPE__ size_t;
static void *memset(void *d, int c, size_t n) {   /* static: TU-local symbol */
    unsigned char *p = d;
    for (size_t i = 0; i < n; i++) p[i] = 0;
    return d;
}
int f(void) { char b[8]; memset(b, 7, 8); return b[0]; }
C
cat >"$tmp/runtime.c" <<'C'
#include <stdio.h>
typedef __SIZE_TYPE__ size_t;
static unsigned hits;
void *memset(void *d, int c, size_t n) { /* ignores c, fills zero */
    unsigned char *p = d; hits++;
    for (size_t i = 0; i < n; i++) p[i] = 0;
    return d;
}
void *memcpy(void *d, const void *s, size_t n) { /* deliberately reverse */
    unsigned char *p = d; const unsigned char *q = s; hits++;
    while (n--) p[n] = q[n];
    return d;
}
void forward_copy(unsigned char *restrict d, const unsigned char *restrict s, unsigned n) {
    for (unsigned i = 0; i < n; i++) d[i] = s[i];
}
int main(void) {
    unsigned char b[8];
    const unsigned char s[4] = {'a', 'b', 'c', 'd'};
    forward_copy(b, s, 4);
    if (b[0] != 'a' || b[3] != 'd') { puts("copy reversed"); return 1; }
    memset(b, 7, 8); /* constant size + TU definition: must stay a call */
    if (b[0] != 0 || b[7] != 0) { puts("memset contract assumed"); return 1; }
    if (hits != 1) { puts("unexpected libcall traffic"); return 1; }
    puts("ok");
    return 0;
}
C

# ── helpers ─────────────────────────────────────────────────────────────────
asm_of() { # <cfile> <flags...>  -> stdout asm (fails the gate on compiler error)
    local src=$1; shift
    if ! "$CCC" "$@" -S -x c "$src" -o "$tmp/out.s" 2>"$tmp/err"; then
        bad "compile failed: $(basename "$src") $*"
        sed -n '1,3p' "$tmp/err" >&2
        printf '\n'
        return 0
    fi
    cat "$tmp/out.s"
}
# Number of `call` instructions inside the body of function $2 of asm on stdin.
calls_in() {
    awk -v fn="$2" '
        index($0, fn ":") == 1 { in_fn = 1; next }
        in_fn && /^\.size/ { in_fn = 0 }
        in_fn && /call/ { n++ }
        END { print n + 0 }'
}
has_fn() { grep -q "^$2:" <<<"$1"; }

expect_refused() { # <cfile> <fn> <flags...>
    local src=$1 fn=$2; shift 2
    local asm; asm=$(asm_of "$src" "$@")
    if ! has_fn "$asm" "$fn"; then bad "$(basename "$src") $*: $fn not emitted"; return; fi
    local n; n=$(calls_in "$asm" "$fn")
    if [ "$n" != "0" ]; then
        bad "$(basename "$src") $*: $fn still calls a libcall ($n) -- self-call/aliasing risk"
    else
        note "refused   $(basename "$src") $*  ($fn: 0 calls)"
    fi
}
# No call inside `$fn` may reference symbol `$sym`.  Needed where the body
# legitimately calls something *else*: a TU-defined `puts` whose printf->puts
# fold was refused still ends in `call printf@PLT`, so "no calls at all" is
# the wrong assertion there.
expect_no_call_to() { # <cfile> <fn> <sym> <flags...>
    local src=$1 fn=$2 sym=$3; shift 3
    local asm; asm=$(asm_of "$src" "$@")
    if ! has_fn "$asm" "$fn"; then bad "$(basename "$src") $*: $fn not emitted"; return; fi
    if awk -v fn="$fn" -v sym="$sym" '
            index($0, fn ":") == 1 { in_fn = 1; next }
            in_fn && /^\.size/ { in_fn = 0 }
            in_fn && /call/ && index($0, sym) { found = 1 }
            END { exit found ? 1 : 0 }' <<<"$asm"; then
        note "refused   $(basename "$src") $*  ($fn: no call to $sym)"
    else
        bad "$(basename "$src") $*: $fn still calls $sym -- self-call/aliasing risk"
    fi
}
expect_call() { # <cfile> <fn> <expected-symbol> <flags...>
    local src=$1 fn=$2 sym=$3; shift 3
    local asm; asm=$(asm_of "$src" "$@")
    if ! has_fn "$asm" "$fn"; then bad "$(basename "$src") $*: $fn not emitted"; return; fi
    if awk -v fn="$fn" -v sym="$sym" '
            index($0, fn ":") == 1 { in_fn = 1; next }
            in_fn && /^\.size/ { in_fn = 0 }
            in_fn && /call/ && index($0, sym) { found = 1 }
            END { exit found ? 0 : 1 }' <<<"$asm"; then
        note "synthesis $(basename "$src") $*  ($fn calls $sym)"
    else
        bad "$(basename "$src") $*: $fn does not call $sym -- optimisation lost"
    fi
}
# The function must not assume the *library* implementation's result.  Keeping
# the call is correct (it honours the TU definition), so the assertion is:
# either the call is there, or -- if the body was inlined -- no store of the
# fill immediate the library call would have written may appear.  The store
# form is `movX $7, <mem>`; `movl $7, %esi` (an argument) is not a store.
expect_no_library_fill() { # <cfile> <fn> <sym> <flags...>
    local src=$1 fn=$2 sym=$3; shift 3
    local asm; asm=$(asm_of "$src" "$@")
    if awk -v fn="$fn" -v sym="$sym" '
            index($0, fn ":") == 1 { in_fn = 1; next }
            in_fn && /^\.size/ { in_fn = 0 }
            in_fn && /call/ && index($0, sym) { found = 1 }
            END { exit found ? 0 : 1 }' <<<"$asm"; then
        note "semantics $(basename "$src") $*  ($fn keeps the call to $sym)"
        return
    fi
    if awk -v fn="$fn" '
            index($0, fn ":") == 1 { in_fn = 1; next }
            in_fn && /^\.size/ { in_fn = 0 }
            in_fn && /\$7,[[:space:]]*[^%]*\(/ { found = 1 }
            END { exit found ? 1 : 0 }' <<<"$asm"; then
        note "semantics $(basename "$src") $*  ($fn inlined without the library fill)"
    else
        bad "$(basename "$src") $*: $fn stores the library fill byte -- contract assumed"
    fi
}

expect_no_call() { # <cfile> <flags...>: the whole TU must contain no call
    local src=$1; shift
    local asm; asm=$(asm_of "$src" "$@")
    if grep -q 'call' <<<"$asm"; then
        bad "$(basename "$src") $*: expected no call at all"
    else
        note "no calls  $(basename "$src") $*"
    fi
}

# ── 1. refusals ─────────────────────────────────────────────────────────────
for opt in -O2 -O3 -Os; do
    expect_refused "$tmp/def_memcpy.c"  memcpy  "$opt"
    expect_refused "$tmp/def_memmove.c" memmove "$opt"
    expect_refused "$tmp/def_memset.c"  memset  "$opt"
    # `puts`' body legitimately calls `printf`: assert the *symbol*, not
    # "no calls at all" (the pre-fix bug was `call puts@PLT` inside `puts`).
    expect_no_call_to "$tmp/def_puts.c" puts puts "$opt"
    expect_refused "$tmp/static_memcpy.c" cp    "$opt"   # static definition counts
done

# ── 2. positive controls ────────────────────────────────────────────────────
for opt in -O2 -O3 -Os; do
    expect_call "$tmp/free_copy.c" cp memcpy "$opt"
    expect_call "$tmp/free_fill.c" zf memset "$opt"
    expect_call "$tmp/free_puts.c" g  puts   "$opt"
done

# ── 3. flag contract ────────────────────────────────────────────────────────
expect_no_call "$tmp/free_copy.c" -O2 -fno-builtin
expect_no_call "$tmp/free_copy.c" -O2 -ffreestanding
expect_no_call "$tmp/free_copy.c" -O2 -fno-builtin-memcpy
expect_no_call "$tmp/free_fill.c" -O2 -fno-builtin-memset
expect_call   "$tmp/free_copy.c" cp memcpy -O2 -fno-builtin -fbuiltin
expect_call   "$tmp/free_copy.c" cp memcpy -O2 -ffreestanding -fhosted
# per-name precision: withdrawing memset leaves the copy rewrite alone
expect_call   "$tmp/free_copy.c" cp memcpy -O2 -fno-builtin-memset
expect_call   "$tmp/free_fill.c" zf memset -O2 -fno-builtin-memcpy

# ── 4. constant-size call expansion (A14) ───────────────────────────────────
expect_no_call "$tmp/const_calls.c" -O2                       # expanded inline
expect_call   "$tmp/const_calls.c" f memset -O2 -fno-builtin
expect_call   "$tmp/const_calls.c" g memcpy -O2 -fno-builtin
expect_call   "$tmp/const_calls.c" f memset -O2 -ffreestanding
expect_call   "$tmp/const_calls.c" g memcpy -O2 -fno-builtin-memcpy
expect_call   "$tmp/const_calls.c" f memset -O2 -fno-builtin-memset
# fortified expansion is not gated by the withdrawal flags (both oracles)
expect_no_call "$tmp/fortify_call.c" -O2 -D_FORTIFY_SOURCE=2
expect_no_call "$tmp/fortify_call.c" -O2 -D_FORTIFY_SOURCE=2 -fno-builtin-memcpy
expect_call   "$tmp/fortify_call.c" f memset -O2 -D_FORTIFY_SOURCE=2 -fno-builtin-memset
# a TU *definition* of the callee also stops the expansion (no flag needed).
# The global definition stays a call; a *static* one may legitimately be
# inlined (that honours it), so the assertion there is semantic: no library
# fill byte may appear.
expect_call          "$tmp/tu_def_const_call.c" f memset -O2
expect_no_library_fill "$tmp/tu_static_const_call.c" f memset -O2

# ── 5. runtime ──────────────────────────────────────────────────────────────
if "$CCC" -O2 -o "$tmp/rt" "$tmp/runtime.c" 2>"$tmp/err" && out=$("$tmp/rt"); then
    if [ "$out" = "ok" ]; then
        note "runtime   -O2 program honours the TU's own library functions"
    else
        bad "runtime -O2: unexpected output '$out'"
    fi
else
    bad "runtime -O2: program did not run (a synthesised call reached a TU definition)"
fi

# The original A13 shape: a TU-defined `puts` whose printf->puts fold turned
# the call into `call puts@PLT` inside `puts` (unbounded recursion; GCC 16.2,
# Clang 23.1 and ICX 2025 still emit exactly that, GCC 14.2 crashes here).
cat >"$tmp/rt_puts.c" <<'C'
#include <stdio.h>
int puts(const char *s) { printf("hello\n"); return s != 0 ? 0 : -1; }
int main(void) { return puts("x") != 0; }
C
if "$CCC" -O2 -o "$tmp/rtp" "$tmp/rt_puts.c" 2>"$tmp/err" && out=$("$tmp/rtp"); then
    if [ "$out" = "hello" ]; then
        note "runtime   -O2 TU-defined puts prints once and returns"
    else
        bad "runtime -O2 TU-defined puts: unexpected output '$out'"
    fi
else
    bad "runtime -O2: TU-defined puts did not run (printf->puts self-call)"
fi

# The same shape forced through a volatile pointer, so nothing can fold the
# call away and the *definition* must really run: it must fill zeros (its own
# semantics), not the library's `c`.  GCC 14.2 SIGSEGVs here at -O2 (its own
# compiled `memset` is `call memset@PLT` with the same d and n), which is why
# this row is lccc-only by construction: the oracle is broken.
cat >"$tmp/forced_memset_call.c" <<'C'
#include <stdio.h>
typedef __SIZE_TYPE__ size_t;
void *memset(void *d, int c, size_t n) {
    unsigned char *p = d;
    for (size_t i = 0; i < n; i++) p[i] = 0;
    return d;
}
void *(*volatile p_memset)(void *, int, size_t) = memset;
int main(void) {
    unsigned char b[8];
    p_memset(b, 7, 8);
    return b[0];
}
C
if "$CCC" -O2 -o "$tmp/rtf" "$tmp/forced_memset_call.c" 2>"$tmp/err" && "$tmp/rtf"; then
    note "runtime   -O2 forced call through the TU's memset honours it (exit 0)"
else
    bad "runtime -O2: forced call to the TU's memset returned non-zero / crashed"
fi

if [ "$fails" -ne 0 ]; then
    printf 'libcall-synthesis gate: %d failure(s)\n' "$fails" >&2
    exit 1
fi
printf 'libcall-synthesis gate: all checks passed\n'
