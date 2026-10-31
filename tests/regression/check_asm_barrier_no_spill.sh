#!/usr/bin/env bash
# An inline-asm block whose template is empty assembles to no instructions, so
# it must also stage no operands. `asm volatile("" : "+m"(x))` is the
# operand-carrying form of the compiler barrier: its operand list describes a
# memory effect to the optimiser, but the assembler never sees an instruction,
# so there is no scratch register to reserve, no input to load and no output to
# write back.
#
# Before this gate existed, lccc resolved the memory operand's address into a
# scratch register anyway. When the busy set left no free scratch (a loop with
# several live values), the resolver spilled that address to a frame slot and
# reloaded it after the block: a store/load pair in the *loop body*.
# zlib_ng_slide_hash's `ref_slide_chain` carried exactly that pair through
# 25M iterations.
#
# Parts:
#   A. shape: no frame slot may be written and re-read inside the barrier loop,
#      and the loop must not be materially longer than the reference compiler's.
#   B. behaviour: the runnable fixture's output must match gcc-16's. The barrier
#      is a real semantic barrier — the store has to survive it.
#   C. self-test: the shape checker must fire on a spill pair and stay quiet on
#      a clean loop, so a checker that silently matches nothing cannot pass.
set -uo pipefail

CC="${CCC:-./target/fastbuild/lccc}"
GCC_BIN="${GCC_BIN:-gcc-16}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
pass=0
fail=0

note_pass() { echo "  ok   $1"; pass=$((pass + 1)); }
note_fail() { echo "  FAIL $1"; fail=$((fail + 1)); }

# Two functions, no `main`: at -O2 a static helper would be inlined into the
# caller and the loop shape this gate inspects would disappear.
cat >"$tmp/kernels.c" <<'EOF'
#include <stdint.h>

void chain(uint16_t *t, unsigned n, unsigned w) {
    for (unsigned i = 0; i < n; i++) {
        uint32_t v = t[i];
        t[i] = (v >= w) ? (uint16_t)(v - w) : 0U;
        __asm__ volatile("" : "+m"(t[i]));
    }
}

unsigned sum_with_barrier(const uint16_t *t, unsigned n) {
    unsigned s = 0;
    for (unsigned i = 0; i < n; i++) {
        s += t[i];
        __asm__ volatile("" : : "m"(t[i]));
    }
    return s;
}
EOF

cat >"$tmp/run.c" <<'EOF'
#include <stdint.h>
#include <stdio.h>

static void chain(uint16_t *t, unsigned n, unsigned w) {
    for (unsigned i = 0; i < n; i++) {
        uint32_t v = t[i];
        t[i] = (v >= w) ? (uint16_t)(v - w) : 0U;
        __asm__ volatile("" : "+m"(t[i]));
    }
}

static unsigned sum_with_barrier(const uint16_t *t, unsigned n) {
    unsigned s = 0;
    for (unsigned i = 0; i < n; i++) {
        s += t[i];
        __asm__ volatile("" : : "m"(t[i]));
    }
    return s;
}

#define N 64
int main(void) {
    uint16_t a[N];
    for (unsigned i = 0; i < N; i++)
        a[i] = (uint16_t)(i * 977u);
    chain(a, N, 300u);
    chain(a, N, 17u);
    unsigned s = sum_with_barrier(a, N);
    printf("%u %u %u\n", (unsigned)a[0], (unsigned)a[N - 1], s);
    return 0;
}
EOF

# Print the body of one function from an AT&T assembly listing.
fn_body() {
    python3 -c '
import re, sys
name, path = sys.argv[1], sys.argv[2]
lines = [l.rstrip("\n") for l in open(path, errors="replace")]
out, inside = [], False
for line in lines:
    stripped = line.strip()
    if not inside:
        if re.match(r"^%s:" % re.escape(name), stripped):
            inside = True
        continue
    if re.match(r"^\.size\s+%s\b" % re.escape(name), stripped):
        break
    out.append(line)
print("\n".join(out))
' "$1" "$2"
}

# The loop body: the labelled block that holds the narrow load/store pair and
# ends in a *backward* branch.
loop_body() {
    python3 -c '
import re, sys
lines = [l.rstrip("\n") for l in open(sys.argv[1], errors="replace")]
blocks, cur, label = [], [], None
for line in lines:
    m = re.match(r"^([.\w$]+):$", line)
    if m:
        if label is not None:
            blocks.append((label, cur))
        label, cur = m.group(1), []
    elif label is not None:
        cur.append(line)
if label is not None:
    blocks.append((label, cur))
seen = set()
for label, body in blocks:
    seen.add(label)
    if not body:
        continue
    m = re.match(r"^j[a-z]+\s+([.\w$]+)", body[-1].strip())
    if not m or m.group(1) not in seen:
        continue
    if not any(re.search(r"\bmov(zwl|w)\b.*\(%", l) for l in body):
        continue
    print("\n".join(body))
    raise SystemExit(0)
raise SystemExit(1)
' "$1"
}

spill_stores() {
    grep -cE '^[[:space:]]*movq[[:space:]]+%r[0-9a-z]+, *[0-9]+\(%rsp\)' "$1" 2>/dev/null || true
}
spill_loads() {
    grep -cE '^[[:space:]]*movq[[:space:]]+[0-9]+\(%rsp\), *%r[0-9a-z]' "$1" 2>/dev/null || true
}
insn_count() { grep -cE '^[[:space:]]+[a-z]' "$1" 2>/dev/null || true; }

# ---- A. shape ------------------------------------------------------------
if ! "$CC" -O2 -S "$tmp/kernels.c" -o "$tmp/kernels_lccc.s" 2>"$tmp/err.txt"; then
    note_fail "lccc compiles the barrier kernels: $(tail -1 "$tmp/err.txt")"
else
    fn_body chain "$tmp/kernels_lccc.s" >"$tmp/lc_fn.s"
    loop_body "$tmp/lc_fn.s" >"$tmp/lc_body.s"
    if [ ! -s "$tmp/lc_body.s" ]; then
        note_fail "the barrier loop is identifiable in lccc output"
    else
        stores=$(spill_stores "$tmp/lc_body.s")
        reloads=$(spill_loads "$tmp/lc_body.s")
        if [ "${stores:-0}" -eq 0 ] && [ "${reloads:-0}" -eq 0 ]; then
            note_pass "the barrier loop neither spills to nor reloads from the frame"
        else
            note_fail "the barrier loop spills $stores and reloads $reloads register(s)"
            sed 's/^/       /' "$tmp/lc_body.s"
        fi
        lc_insns=$(insn_count "$tmp/lc_body.s")
        if command -v "$GCC_BIN" >/dev/null 2>&1; then
            "$GCC_BIN" -O2 -S "$tmp/kernels.c" -o "$tmp/kernels_gcc.s" 2>/dev/null
            fn_body chain "$tmp/kernels_gcc.s" >"$tmp/gcc_fn.s"
            loop_body "$tmp/gcc_fn.s" >"$tmp/gcc_body.s"
            gcc_insns=$(insn_count "$tmp/gcc_body.s")
            # The lccc loop keeps the same load/compare/cmov/store shape; gcc's
            # differs by a couple of register moves. Anything beyond that comes
            # from operand staging, not from codegen.
            if [ "${gcc_insns:-0}" -gt 0 ] && [ "$lc_insns" -le $((gcc_insns + 4)) ]; then
                note_pass "barrier loop is $lc_insns instructions (gcc-16: $gcc_insns)"
            else
                note_fail "barrier loop is $lc_insns instructions, gcc-16 emits $gcc_insns"
            fi
        fi
    fi
fi

# ---- B. behaviour --------------------------------------------------------
if "$CC" -O2 "$tmp/run.c" -o "$tmp/lc.bin" 2>"$tmp/err.txt" &&
    "$GCC_BIN" -O2 "$tmp/run.c" -o "$tmp/gcc.bin" 2>"$tmp/gerr.txt"; then
    lc_out=$("$tmp/lc.bin")
    gcc_out=$("$tmp/gcc.bin")
    if [ "$lc_out" = "$gcc_out" ]; then
        note_pass "barrier fixture output matches gcc-16 ($lc_out)"
    else
        note_fail "barrier fixture output differs (lccc '$lc_out', gcc '$gcc_out')"
    fi
else
    note_fail "barrier fixture fails to compile"
fi

# ---- C. self-test -------------------------------------------------------
cat >"$tmp/spilled.s" <<'EOF'
.LBB2:
    leaq (%rdi, %rsi), %rcx
    movq %rcx, 8(%rsp)
    movzwl (%rcx), %edi
    movw %di, (%rcx)
    movq 8(%rsp), %rcx
    addq $1, %rsi
    cmpq %r8, %rsi
jb .LBB2
EOF
loop_body "$tmp/spilled.s" >"$tmp/self_body.s"
if [ -s "$tmp/self_body.s" ] && [ "$(spill_stores "$tmp/self_body.s")" -eq 1 ] &&
    [ "$(spill_loads "$tmp/self_body.s")" -eq 1 ]; then
    note_pass "self-test: the checker recognises a spill pair"
else
    note_fail "self-test: the checker misses a spill pair"
fi
cat >"$tmp/clean.s" <<'EOF'
.LBB2:
    leaq (%rdi, %rsi), %rcx
    movzwl (%rcx), %edi
    movw %di, (%rcx)
    addq $1, %rsi
    cmpq %r8, %rsi
jb .LBB2
EOF
loop_body "$tmp/clean.s" >"$tmp/clean_body.s"
if [ -s "$tmp/clean_body.s" ] && [ "$(spill_stores "$tmp/clean_body.s")" -eq 0 ] &&
    [ "$(spill_loads "$tmp/clean_body.s")" -eq 0 ]; then
    note_pass "self-test: the checker accepts a spill-free loop"
else
    note_fail "self-test: the checker flags a spill-free loop"
fi

echo
if [ "$fail" -eq 0 ]; then
    echo "asm barrier staging: PASS ($pass checks)"
    exit 0
fi
echo "asm barrier staging: FAIL ($fail of $((pass + fail)) checks)"
exit 1
