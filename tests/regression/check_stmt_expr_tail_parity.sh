#!/usr/bin/env bash
# GCC differential: the value of a GNU statement expression, tail item by tail
# item, and the diagnostics a used-but-void statement expression must produce.
#
# Why this exists: the parser queues a struct member `_Static_assert` as a block
# item after the very statement that must supply a statement expression's value
# (the kernel's `sizeof(struct { _Static_assert(...); })` idiom). An earlier
# arrangement tracked which assertion came from a member list and hid those from
# the tail search, which is not GCC's rule and left the ordering fragile. GCC's
# actual rule (16.2, re-derived here on every run) is that `_Static_assert`, the
# null statement and every declaration that reserves no block storage are
# transparent, and anything else makes the statement expression `void`.
#
# The fixture tests/regression/stmt_expr_tail_parity.c pins the same matrix as
# compile-only assertions so the corpus covers it; this script adds the three
# things a fixture cannot: the oracle re-derivation, the *values* the lowering
# must deliver, and the void-value diagnostics.
#
# Oracle: GCC 16.2 (this project's reference). Clang 23.1.4 answers `void` for
# every transparent tail; that divergence is documented in the fixture and
# asserted below only when a Clang oracle is installed, so a future Clang that
# changes its mind is reported instead of silently contradicting the comment.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
CLANG=${CLANG:-}
command -v "$GCC" >/dev/null || { echo "missing oracle: $GCC" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
fail() { echo "FAIL $*" >&2; exit 1; }

# type_of <compiler> <bits> <tail>: the type of `({ <tail> })`, spelled as one of
# the candidates below or `none` when it is something else entirely. Probed with
# __builtin_types_compatible_p so no diagnostic text is parsed.
type_of() {
    local compiler=$1 bits=$2 tail=$3 candidate
    for candidate in int void 'long' 'unsigned char' 'char *'; do
        printf 'void probe(void) { _Static_assert(__builtin_types_compatible_p(typeof(({%s})), %s), "t"); }\n' \
            "$tail" "$candidate" >"$tmp/probe.c"
        if "$compiler" -std=gnu11 -m"$bits" -fsyntax-only "$tmp/probe.c" >/dev/null 2>&1; then
            printf '%s' "$candidate"
            return 0
        fi
    done
    printf 'none'
}

# expect_type <expected> <tail>: the oracle and this compiler must both answer
# `expected` in both target modes. Checking the oracle too is deliberate: if a
# future GCC changes the rule, this gate fails and the pin gets re-measured
# instead of quietly redefining "correct".
expect_type() {
    local expected=$1 tail=$2 bits oracle actual
    for bits in 32 64; do
        oracle=$(type_of "$GCC" "$bits" "$tail")
        [[ $oracle == "$expected" ]] ||
            fail "oracle drift: ($GCC -m$bits) types ({ $tail }) as $oracle, pinned $expected"
        actual=$(type_of "$CCC" "$bits" "$tail")
        [[ $actual == "$expected" ]] ||
            fail "($CCC -m$bits) types ({ $tail }) as $actual, GCC says $expected"
    done
}

# Runtime cases need startup files the type matrix does not. Probe the oracle
# once and drop to 64-bit only where the image has no i386 multilib (GitHub CI
# provisions it, so the full matrix runs there and the reduction is announced
# here rather than silently shrinking what a local run proves).
run_bits=(32 64)
if ! printf 'int main(void) { return 0; }\n' >"$tmp/m32.c" ||
    ! "$GCC" -m32 "$tmp/m32.c" -o "$tmp/m32.bin" >/dev/null 2>&1; then
    run_bits=(64)
    echo "note: $GCC cannot link -m32 here; runtime cases run 64-bit only" >&2
fi

# case_run <name> <expected-exit> <body>: both compilers build it, both binaries
# must produce the same exit status.
case_run() {
    local name=$1 expected=$2 body=$3 bits compiler rc
    printf 'int main(void) { %s }\n' "$body" >"$tmp/run.c"
    for bits in "${run_bits[@]}"; do
        for compiler in "$GCC" "$CCC"; do
            "$compiler" -std=gnu11 -m"$bits" -O2 "$tmp/run.c" -o "$tmp/run.bin" >"$tmp/out" 2>&1 ||
                { echo "FAIL $name: $compiler -m$bits could not build it" >&2; cat "$tmp/out" >&2; exit 1; }
            rc=0
            "$tmp/run.bin" || rc=$?
            [[ $rc == "$expected" ]] ||
                fail "$name: $compiler -m$bits exited $rc, expected $expected"
        done
    done
}

# case_reject <name> <body>: a used void value is an error for both compilers,
# with GCC's wording for the context.
case_reject() {
    local name=$1 body=$2 compiler rc
    printf '%s\n' "$body" >"$tmp/reject.c"
    for compiler in "$GCC" "$CCC"; do
        rc=0
        "$compiler" -std=gnu11 -fsyntax-only "$tmp/reject.c" >"$tmp/out" 2>&1 || rc=$?
        [[ $rc != 0 ]] || fail "$name: $compiler accepted a used void statement expression"
        grep -qiE 'void value not ignored|invalid use of void expression' "$tmp/out" ||
            { echo "FAIL $name: $compiler rejected it for another reason" >&2; cat "$tmp/out" >&2; exit 1; }
    done
}

# --- transparent tails: the value is the last expression statement ----------
expect_type int '1;'
expect_type int '1; _Static_assert(1, "a");'
expect_type int '1; _Static_assert(1, "a"); _Static_assert(1, "b");'
expect_type int '_Static_assert(1, "a"); 2;'
expect_type int '1; ;'
expect_type int '1; ; ;'
expect_type int '1; struct S { int a; };'
expect_type int '1; enum E { A };'
expect_type int '1; typedef int T;'
expect_type int '1; extern int o;'
expect_type int '1; int f(void);'
expect_type int '1; inline int g(void);'
expect_type int '1; lbl: 2;'

# --- opaque tails: the statement expression is void -------------------------
expect_type void '1; int y;'
expect_type void '1; static int q;'
expect_type void '1; int y = 2;'
expect_type void '1; { 2; }'
expect_type void '1; if (1) ;'
expect_type void '1; do { 2; } while (0);'
expect_type void '1; lbl: ;'
expect_type void '_Static_assert(1, "a");'
expect_type void ';'
expect_type void ''

# --- the values the lowering must deliver ----------------------------------
case_run assert-tail 1 'int v = ({ 1; _Static_assert(1, "a"); }); return v;'
case_run null-tail 1 'int v = ({ 1; ; }); return v;'
case_run typedef-tail 1 'int v = ({ 1; typedef int T; }); return v;'
case_run prototype-tail 1 'int v = ({ 1; int f(void); }); return v;'
case_run label-tail 2 'int v = ({ 1; lbl: 2; }); return v;'
case_run local-tail 5 'int v = ({ int t = 5; t; }); return v;'
case_run kernel-member-assert 1 'struct n { int v; } a[2];
    int v = ({ (typeof(a[sizeof(struct { _Static_assert(1, "m"); })]) *)0 == 0; });
    return v;'
case_run discarded-void 0 '({ (void)2; }); return 0;'

# --- a used void statement expression is an error in every context ---------
case_reject init-void 'int main(void) { int v = ({ (void)2; }); return v; }'
case_reject init-decl-tail 'int main(void) { int v = ({ 1; int y; }); return v; }'
case_reject init-empty 'int main(void) { int v = ({ ; }); return v; }'
case_reject init-tail-only 'int main(void) { int v = ({ if (1) ; }); return v; }'
case_reject return-void 'int main(void) { return ({ (void)2; }); }'
case_reject assign-void 'int main(void) { int v; v = ({ (void)2; }); return v; }'
case_reject argument-void 'int f(int); int main(void) { return f(({ if (1) ; })); }'

# --- documented Clang divergence, asserted when an oracle is available ------
if [[ -n $CLANG ]] && command -v "$CLANG" >/dev/null; then
    for tail in '1; _Static_assert(1, "a");' '1; ;' '1; typedef int T;'; do
        actual=$(type_of "$CLANG" 64 "$tail")
        [[ $actual == void ]] ||
            fail "clang divergence changed: $CLANG types ({ $tail }) as $actual, not void"
    done
fi

echo "statement expression tails: PASS (GCC differential, 32/64-bit, values and void diagnostics)"
