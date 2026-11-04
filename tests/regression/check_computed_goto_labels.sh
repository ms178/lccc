#!/usr/bin/env bash
# Computed-goto / label-namespace contract on x86_64 and i686.
#
# This gate exists because THREE distinct defects all surfaced as the same
# symptom -- `ccc: internal assembler error: '.LBBn' is referenced ... but has
# no symbol-table entry` -- and each needed a different fix:
#
#   1. gcc.c-torture/compile/pr17913.c: `&&label` in a `?:` arm the constant
#      fold removed. The label ID was allocated but NO basic block was ever
#      started. Fixed by materialising a fall-off-the-end landing pad in
#      finalize_function/finalize_nested_function.
#   2. gcc.c-torture/compile/pr84136.c: two byte-identical `ret` blocks, one
#      address-taken. The x86 identical-block peephole merged them and deleted
#      the address-taken label. Fixed by protecting every `.LBB` that appears
#      as a DATA operand (not only `.quad`/`.long` jump tables).
#   3. Undefined labels at all: `goto nope`, `&&nope`, `asm goto(::::nope)`,
#      duplicate `a: ; a: ;`. LCCC used to accept them (dangling .LBB, a jump
#      to address 0, or `internal error: permutation covers every block`).
#      GCC 16.2 rejects all of them; a new zero-copy sema pass restores the
#      C23/GNU contract and makes fix (1) unambiguous.
#
# Every expectation below was measured against GCC 16.2 and Clang 23.1.4
# where those compilers support the construct. Nested functions are a GNU
# extension Clang does not implement, so that shape is checked against GCC.
#
# The structural check at the end is the important one: it parses the emitted
# assembly and proves every `.LBBn` named by a data reference has a matching
# label definition. A green "compiles" alone would not catch a dangling
# relocation that an external assembler binds to absolute address 0.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null 2>&1 || GCC=gcc
[ -x "$CCC" ] || { echo "check_computed_goto_labels: SKIP ($CCC not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-computed-goto.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/runtime.c" <<'C'
#include <stdio.h>

static int fails;
#define CHECK(e) do { if (!(e)) { printf("FAIL %d: %s\n", __LINE__, #e); ++fails; } } while (0)

void *g_a;

/* pr84136: two byte-identical `ret` blocks, one of them address-taken.
   The identical-block peephole must NOT delete .LBB for `l:`. */
void foo(void)
{
    if ((g_a = &&l))
        return;
l:
    ;
}

/* pr17913: the label is inside a `?:` arm the constant fold removes.
   &&here is still a real address; landing on it leaves the function. */
int dead_arm(void)
{
    void *p = &&here;
    1 ? (void)0 : ({ here: (void)0; });
    return p == 0;
}

/* Four computed-goto targets, two of which have byte-identical bodies.
   Includes a SCALAR static &&label initializer and a jump table. */
int dispatch(int which)
{
    static void *const table[4] = { &&zero, &&one, &&one_alias, &&two };
    static void *scalar = &&zero;
    if (scalar != &&zero) return 90;
    if (&&one == &&one_alias) return 91;
    if (table[1] != &&one || table[2] != &&one_alias) return 92;
    goto *table[which & 3];
zero:
    return 0;
one:
    return 1;
one_alias:
    return 1;
two:
    return 2;
}

int main(void)
{
    foo();
    CHECK(g_a != 0);
    CHECK(dead_arm() == 0);
    CHECK(dispatch(0) == 0);
    CHECK(dispatch(1) == 1);
    CHECK(dispatch(2) == 1);
    CHECK(dispatch(3) == 2);
    printf(fails ? "FAIL %d\n" : "PASS %d\n", fails);
    return fails != 0;
}
C

# ── Negative diagnostics ──────────────────────────────────────────────────
cat >"$tmp/u_goto.c" <<'C'
void f(void) { goto nope; }
C
cat >"$tmp/u_addr.c" <<'C'
void f(void) { void *p = &&nope; goto *p; }
C
cat >"$tmp/u_asm.c" <<'C'
void f(void) { asm goto ("" : : : : nope); }
C
cat >"$tmp/u_dup.c" <<'C'
void f(void) { a: ; a: ; }
C
cat >"$tmp/u_local.c" <<'C'
void f(void) { a: ; { __label__ a; goto a; } }
C
cat >"$tmp/u_static.c" <<'C'
void f(void) { static void *p = &&nope; goto *p; }
C
cat >"$tmp/u_child.c" <<'C'
void f(void) { void g(void) { b: ; } void *p = &&b; goto *p; }
C

# ── Positive GNU shapes that must stay legal ──────────────────────────────
cat >"$tmp/p_nested.c" <<'C'
/* Non-local goto from a nested function to a __label__-declared parent label. */
int f(void)
{
    __label__ target;
    static void *seen;
    void transfer(void *value) { seen = value; goto target; }
    transfer(&&target);
    return 1;
target:
    return seen == &&target ? 0 : 1;
}
C
cat >"$tmp/p_local.c" <<'C'
void f(void) { __label__ a; a: ; goto a; }
C
cat >"$tmp/p_case.c" <<'C'
void f(void) { switch (1) { case 1: goto c; } c: ; }
C
cat >"$tmp/p_scalar.c" <<'C'
void f(void) { static void *p = &&a; goto *p; a: ; }
C

# Structural proof: no `.LBBn` data reference may name an undefined label.
cat >"$tmp/check_asm.py" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
defined = set(re.findall(r"^\s*(\.LBB\d+):", text, re.M))
referenced = set()
for line in text.splitlines():
    # A label definition is not a reference. Data operands are `leaq .LBBn(%rip)`,
    # `.quad .LBBn`, `.long .LBBn`, plus branch operands for completeness.
    if re.match(r"^\s*\.LBB\d+:", line):
        continue
    referenced.update(re.findall(r"\.LBB\d+", line))
dangling = sorted(referenced - defined)
if dangling:
    print("dangling .LBB references: " + ", ".join(dangling), file=sys.stderr)
    sys.exit(1)
print("asm labels: %d defined, %d referenced, 0 dangling" % (len(defined), len(referenced)))
PY

expect_error() {
    local file=$1 want=$2 out rc=0
    out=$("$compiler" ${mflag:-} -O2 -c "$tmp/$file" -o "$tmp/neg.o" 2>&1) || rc=$?
    [ "$rc" -ne 0 ] || { echo "$label/$file: accepted an invalid program"; return 1; }
    case "$out" in
        *"$want"*) ;;
        *) echo "$label/$file: expected '$want', got: $out"; return 1 ;;
    esac
    echo "$label/$file: rejected ($want)"
}

run_target() {
    local label=$1 compiler=$2 mflag=${3:-}
    local opt
    for opt in -O0 -O2; do
        "$compiler" $mflag "$opt" -c "$tmp/runtime.c" -o "$tmp/runtime.o"
        "$GCC" $mflag "$tmp/runtime.o" -o "$tmp/runtime"
        local out rc=0
        out=$("$tmp/runtime") || rc=$?
        [ "$out" = "PASS 0" ] || { echo "$label $opt runtime mismatch: rc=$rc out='$out'"; return 1; }
        echo "$label $opt runtime: $out"

        "$compiler" $mflag "$opt" -S "$tmp/runtime.c" -o "$tmp/runtime.s"
        python3 "$tmp/check_asm.py" "$tmp/runtime.s" || return 1
    done

    expect_error u_goto.c   "label 'nope' used but not defined"
    expect_error u_addr.c   "label 'nope' used but not defined"
    expect_error u_asm.c    "label 'nope' used but not defined"
    expect_error u_static.c "label 'nope' used but not defined"
    expect_error u_local.c  "label 'a' used but not defined"
    expect_error u_child.c  "label 'b' used but not defined"
    expect_error u_dup.c    "duplicate label 'a'"

    local f
    for f in p_nested.c p_local.c p_case.c p_scalar.c; do
        "$compiler" $mflag -O2 -c "$tmp/$f" -o "$tmp/pos.o" || {
            echo "$label/$f: valid GNU program rejected"; return 1; }
        echo "$label/$f: accepted"
    done
}

run_target x86_64 "$CCC" ""
if [ -x "$CCC_I686" ] && [ "${SKIP_I686:-0}" != 1 ]; then
    run_target i686 "$CCC_I686" "-m32"
else
    echo "i686: SKIP ($CCC_I686 not built)"
fi
echo "computed-goto label contract: PASS"
