#!/usr/bin/env bash
# Runtime and codegen regressions for GNU asm-goto outputs, including flag
# outputs on both fall-through and implicit label edges.
#
# Three independent layers, because they fail in different ways:
#   1. runtime  -- every @cc suffix on both edges, multi-output, tied "+",
#                  numeric labels, shared targets, volatile outputs;
#   2. codegen  -- the x86-64 fusion must REMOVE the setcc/store/reload round
#                  trip (a positive assertion: an optimisation that silently
#                  stops applying is a regression even when results stay right);
#   3. contract -- the accepted @cc suffix set and the multi-digit %l<N> label
#                  grammar, both measured against gcc (see the tables in
#                  src/common/asm_constraints.rs and
#                  src/frontend/parser/asm_validate.rs).
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
src=$root/tests/regression/asm_goto_outputs.c
tmp=${TMPDIR:-/tmp}/lccc-asm-goto-outputs.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

run_one() {
    local bits=$1 opt=$2
    "$CCC" -m"$bits" -std=gnu11 -O"$opt" -Wall -Wextra -Werror "$src" -o "$tmp/test"
    local interp
    interp=$(readelf -l "$tmp/test" | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
    if [[ -n $interp && ! -e $interp ]]; then
        echo "SKIP-RUN: -m$bits -O$opt requires missing $interp"
        return
    fi
    local got
    got=$("$tmp/test")
    [[ $got == "PASS asm_goto_outputs" ]]
    echo "PASS -m$bits -O$opt $got"
}

run_one 64 0
run_one 64 2
run_one 32 0
run_one 32 2

# ── lccc extension: outputs are deterministic on taken edges ─────────────────
# No reference compiler defines an asm-goto output on a taken edge -- measured,
# gcc 12 and clang 22 each disagree with themselves between -O0 and -O2, and
# clang returns 101 vs 244 for the same flag read. lccc materialises outputs on
# every edge; this file pins that stronger guarantee, which is why it is a
# separate translation unit from the portable runtime test.
ext=$root/tests/regression/asm_goto_outputs_ext.c
for bits in 64 32; do
    "$CCC" -m"$bits" -std=gnu11 -O2 -Wall -Wextra -Werror "$ext" -o "$tmp/ext"
    interp=$(readelf -l "$tmp/ext" | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
    if [[ -n $interp && ! -e $interp ]]; then
        echo "SKIP-RUN: -m$bits taken-edge extension requires missing $interp"
        continue
    fi
    got=$("$tmp/ext")
    [[ $got == "PASS asm_goto_outputs_ext" ]]
    echo "PASS -m$bits $got"
done

# ── Codegen: the fused branch must not touch memory at all ───────────────────
# Guard the x86-64 -O2 win that motivates the direct-flags path. Slots are
# rbp-relative unless the frame pointer was omitted, so both bases are checked:
# an earlier version matched only (%rsp) and passed while reloading -12(%rbp).
"$CCC" -m64 -O2 -S "$src" -o "$tmp/test.s"
func=$(sed -n '/^asm_cc_output_branch:/,/^\.size asm_cc_output_branch/p' "$tmp/test.s")
[[ -n $func ]] || { echo "FAIL: asm_cc_output_branch missing from -O2 output" >&2; exit 1; }
grep -Eq '^[[:space:]]*j[a-z]+[[:space:]]' <<<"$func" \
    || { echo "FAIL: fused branch emitted no conditional jump" >&2; echo "$func" >&2; exit 1; }
if grep -Eq 'movzbl[[:space:]]+[^,]*\(%r(bp|sp)\)' <<<"$func"; then
    echo "FAIL: asm-goto flag-output branch reloads its stack bool" >&2
    echo "$func" >&2
    exit 1
fi
if grep -Eq '^[[:space:]]*set[abglenopsz][[:space:]]' <<<"$func"; then
    echo "FAIL: fused asm-goto flag output still materialises a setcc" >&2
    echo "$func" >&2
    exit 1
fi
echo "PASS x86-64 direct condition-code branches (no setcc, no stack bool reload)"

# ── Contract: the accepted @cc suffix set ────────────────────────────────────
# Measured: gcc accepts exactly these 28 suffixes and rejects everything else
# with `unknown 'asm' flag output`. The table shared by the parser, setcc
# emission and the fusion analysis must agree with that.
cc_suffixes="e z ne nz s ns o no c nc a nbe ae nb b nae be na g nle ge nl l nge le ng p np"
{
    echo '#include <stdbool.h>'
    i=0
    for cc in $cc_suffixes; do
        printf 'int g%d(int x){ _Bool o=0; asm goto("testl %%1,%%1; jz %%l[L]" : "=@cc%s"(o) : "r"(x) :: L); return o; L: return o+1; }\n' \
            "$i" "$cc"
        i=$((i + 1))
    done
} >"$tmp/suffixes.c"
"$CCC" -m64 -std=gnu11 -O2 -c "$tmp/suffixes.c" -o "$tmp/suffixes.o"
echo "PASS all 28 @cc suffixes accepted"

expect_diagnostic() {
    local label=$1 source=$2 fragment=$3
    printf '%s\n' "$source" >"$tmp/diag.c"
    local out
    if out=$("$CCC" -m64 -std=gnu11 -O2 -c "$tmp/diag.c" -o "$tmp/diag.o" 2>&1); then
        echo "FAIL: $label compiled, expected diagnostic: $fragment" >&2
        exit 1
    fi
    grep -qF -- "$fragment" <<<"$out" || {
        echo "FAIL: $label produced the wrong diagnostic (want: $fragment)" >&2
        echo "$out" >&2
        exit 1
    }
    echo "PASS $label"
}

# gcc: `unknown 'asm' flag output '<constraint>'` for each of these.
expect_diagnostic "@cc without a suffix" \
    'int f(void){ _Bool o=0; asm("" : "=@cc"(o)); return o; }' \
    "unknown 'asm' flag output"
expect_diagnostic "@cc with junk suffix" \
    'int f(void){ _Bool o=0; asm("" : "=@ccxx"(o)); return o; }' \
    "unknown 'asm' flag output"
expect_diagnostic "@ccpe is not a gcc suffix" \
    'int f(void){ _Bool o=0; asm("" : "=@ccpe"(o)); return o; }' \
    "unknown 'asm' flag output"
expect_diagnostic "@cc as an input operand" \
    'int f(int x){ int o; asm("" : "=r"(o) : "@ccz"(x)); return o; }' \
    "invalid punctuation '@' in constraint"

# ── Contract: label indices are multi-digit, operand indices are not ─────────
# Measured: with 13 operands gcc accepts `%l13` and rejects `%l12`/`%l14`.
inputs=$(for _ in $(seq 13); do printf '"r"(v),'; done)
inputs=${inputs%,}
asmgoto() { printf 'int f(int v){ asm goto("testl %%1,%%1; jz %s" :: %s :: L); return 0; L: return -1; }\n' "$1" "$inputs"; }

asmgoto '%l13' >"$tmp/l13.c"
"$CCC" -m64 -std=gnu11 -O2 -c "$tmp/l13.c" -o "$tmp/l13.o"
echo "PASS %l13 names the first label of a 13-operand asm goto"
expect_diagnostic "%l12 is below the first label" "$(asmgoto '%l12')" "operand number out of range"
expect_diagnostic "%l14 is past the last label" "$(asmgoto '%l14')" "operand number out of range"

# A `+` output occupies two operand slots in GCC's label numbering, so the first
# label of `"+r"(v) : "r"(x)` is %l3, not %l2.
printf 'int f(int x){ int v=0; asm goto("testl %%1,%%1; jz %%l3" : "+r"(v) : "r"(x) :: L); return v; L: return -1; }\n' >"$tmp/plus3.c"
"$CCC" -m64 -std=gnu11 -O2 -c "$tmp/plus3.c" -o "$tmp/plus3.o"
echo "PASS %l3 counts the implicit read-write input slot"
expect_diagnostic "%l2 ignores the read-write slot" \
    'int f(int x){ int v=0; asm goto("testl %1,%1; jz %l2" : "+r"(v) : "r"(x) :: L); return v; L: return -1; }' \
    "operand number out of range"

# ── Contract: a plain `%N` also names the goto label ─────────────────────────
# Measured (gcc 16.2, `-S`): with one flag output and two inputs, the label is
# operand 3; `%3` prints `$.L3` (the address) and `%l3` prints `.L3`.  LCCC used
# to reject the plain spelling with "operand number out of range", so real gcc
# programs that take a goto label's address did not compile at all.
printf 'int f(int a, int b){ unsigned char o; asm goto("cmpl %%1, %%2\\n\\t# plain=%%3 named=%%l3" : "=@ccne"(o) : "r"(a), "r"(b) :: L); return o; L: return -1; }\n' >"$tmp/plain.c"
"$CCC" -m64 -std=gnu11 -O2 -S "$tmp/plain.c" -o "$tmp/plain.s"
grep -Eq '# plain=\$\.L[0-9A-Za-z_.]+ named=\.L[0-9A-Za-z_.]+' "$tmp/plain.s" \
    || { echo "FAIL: plain %N did not print the label address" >&2; cat "$tmp/plain.s" >&2; exit 1; }
plain_sym=$(sed -n 's/.*# plain=\$\([^ ]*\) named=\([^ ]*\).*/\1 \2/p' "$tmp/plain.s" | head -1)
set -- $plain_sym
[[ $1 == "$2" ]] || { echo "FAIL: plain %N and %lN name different labels ($1 vs $2)" >&2; exit 1; }
echo "PASS plain %N prints the label address (same symbol as %lN)"
expect_diagnostic "plain %N past the label slots" \
    'int f(int a){ unsigned char o; asm goto("cmpl %1,%1\n\t# %3" : "=@ccne"(o) : "r"(a) :: L); return o; L: return -1; }' \
    "operand number out of range"

# ── Runtime: two flag outputs in one statement ───────────────────────────────
# gcc 16.2 accepts `"=@ccne"(n), "=@ccl"(l)` and answers 0/1/1, 0/1/1, 0/1/1
# for (1,2), (2,1), (3,3) at -O0 and -O2.  The fusion analysis proves single
# output / single use only, so this shape must stay on the materialised path.
cat >"$tmp/two_cc.c" <<'CEOF'
#include <stdio.h>
static int f(int a, int b, int *ne, int *lt) {
    unsigned char n, l;
    __asm__ goto("cmpl %4, %3" : "=@ccne"(n), "=@ccl"(l) : "r"(a), "r"(b) : "cc" : out);
    *ne = n; *lt = l;
    return 0;
out:
    *ne = n; *lt = l;
    return 1;
}
int main(void) {
    int ne, lt, r1 = f(1, 2, &ne, &lt), a = ne, b = lt;
    int r2 = f(2, 1, &ne, &lt), c = ne, d = lt;
    printf("%d %d%d %d %d%d\n", r1, a, b, r2, c, d);
    return 0;
}
CEOF
"$CCC" -m64 -std=gnu11 -O2 "$tmp/two_cc.c" -o "$tmp/two_cc"
[[ $("$tmp/two_cc") == "0 11 0 11" ]] \
    || { echo "FAIL: two flag outputs disagree with gcc ($("$tmp/two_cc"))" >&2; exit 1; }
echo "PASS two @cc outputs in one statement (gcc-measured values)"

# ── Contract: outputs are defined on every edge at every level ───────────────
# gcc and clang leave an asm-goto output undefined on a taken edge and disagree
# with themselves between -O0 and -O2, so the extension is pinned against the
# -O0 answer instead of an oracle.  Freestanding, so the 32-bit leg needs no
# multilib headers and runs on any host.
cat >"$tmp/edges.c" <<'CEOF'
static void exit_with(int code) {
#if defined(__x86_64__)
    __asm__ volatile("syscall" :: "a"(60), "D"(code) : "rcx", "r11", "memory");
#else
    __asm__ volatile("int $0x80" :: "a"(1), "b"(code) : "memory");
#endif
    __builtin_unreachable();
}
__attribute__((noinline)) static int pick(int c, int *p) {
    int r = 0;
    __asm__ goto("movl $7, %0\n\ttestl %2, %2\n\tjnz %l3"
                 : "=r"(r) : "r"(c), "r"(c) : "cc" : taken);
    *p = r;
    return *p + 10;
taken:
    *p = r;
    return *p + 20;
}
void _start(void) {
    int v = 0;
    exit_with((pick(0, &v) * 8 + pick(1, &v)) & 0xff);
}
CEOF
for arch in m64 m32; do
    want=""
    for opt in 0 1 2 3 s; do
        "$CCC" "-$arch" -O"$opt" -ffreestanding -nostdlib -static -fno-pie -no-pie \
            "$tmp/edges.c" -o "$tmp/edges"
        got=0
        "$tmp/edges" || got=$?
        if [[ -z $want ]]; then want=$got; fi
        [[ $got == "$want" ]] \
            || { echo "FAIL: -$arch -O$opt edge value $got != $want at -O0" >&2; exit 1; }
    done
    echo "PASS -$arch asm-goto outputs identical at -O0/-O1/-O2/-O3/-Os ($want)"
done
