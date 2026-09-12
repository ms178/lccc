#!/usr/bin/env bash
# Structural hot-loop ("tight loop") alignment policy — end-to-end pinning
# of the two-layer GCC 16.2 `ix86_align_loops` mirror:
#
#   layer 1 (passes/loop_align.rs, structural audit): only an innermost,
#     contiguously laid-out dynamic loop with no call/asm/indirect
#     transfer and <=1 conditional branch before the latch is even a
#     candidate (CCC_DUMP_ALIGN exposes tight=yes/no + reason);
#   layer 2 (integrated assembler, .lccc_tight_loop): the exact ENCODED
#     header->latch span is bucketed ceil(log2(span)) clamped to 16/32/64,
#     and bodies larger than one 64-byte cache line fail closed to the
#     ordinary bounded cascade (CCC_DEBUG_TIGHT exposes the decision).
#
# Checks (positive, near-miss and adversarial):
#   1. good  — dynamic scalar loop <=64B: tight=yes, header lands on the
#      bucket boundary in the object, program output matches GCC;
#   2. caller— a call in the body rejects at layer 1, cascade survives;
#   3. huge  — layer 1 accepts a large dynamic body, layer 2 rejects
#      span>64 and pads only with the cascade, program output matches GCC;
#   4. tiny  — a provably 3-trip loop receives NO loop alignment at all;
#   5. -S    — the private directive never leaves the process, and the
#      CCC_LOOP_ALIGN_HOT A/B ladders show exactly the documented groups;
#   6. -Os   — policy is fully inert (zero .p2align, zero tight lines);
#   7. parser— a malformed .lccc_tight_loop argument is a hard error, a
#      well-formed one assembles;
#   8. i686  — the cross compiler runs the same two-layer decision.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
CCC32=${CCC32:-./target/fastbuild/lccc-i686}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
fail=0

note() { printf '%s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

cat >"$td/good.c" <<'EOF'
#include <stdio.h>
__attribute__((noinline))
unsigned long good(unsigned n, unsigned long x){
    unsigned long a = x;
    for (unsigned i = 0; i < n; i++) { a += (unsigned long)i*3u + (a>>3); a ^= a<<7; }
    return a;
}
int main(void){ printf("%lu\n", good(1000, 1)); return 0; }
EOF

cat >"$td/caller.c" <<'EOF'
#include <stdio.h>
__attribute__((noinline)) unsigned long black(unsigned long x){ return x*2654435761u; }
__attribute__((noinline))
unsigned long withcall(unsigned n, unsigned long x){
    unsigned long a = x;
    for (unsigned i = 0; i < n; i++) { a += black(a ^ i); }
    return a;
}
int main(void){ printf("%lu\n", withcall(1000, 7)); return 0; }
EOF

cat >"$td/huge.c" <<'EOF'
#include <stdio.h>
__attribute__((noinline))
unsigned long huge(unsigned n, unsigned long x){
    unsigned long a = x;
    for (unsigned i = 0; i < n; i++) {
        a += (unsigned long)i*3; a ^= a<<7; a += (unsigned long)i*5; a ^= a>>11;
        a += (unsigned long)i*7; a ^= a<<13; a += (unsigned long)i*9; a ^= a>>17;
        a += (unsigned long)i*11; a ^= a<<19; a += (unsigned long)i*13; a ^= a>>23;
    }
    return a;
}
int main(void){ printf("%lu\n", huge(1000, 4)); return 0; }
EOF

cat >"$td/tiny.c" <<'EOF'
static volatile unsigned g;
__attribute__((noinline))
unsigned f(unsigned x){ unsigned a = x;
  for (unsigned i = 0; i < 3; i++) { a += i*7u; g = a; }
  return a; }
int main(void){ return (int)(f(10u) & 255u); }
EOF

# ── 1. good: layer 1 accept + layer 2 bucket + achieved alignment ──
dump=$({ CCC_DUMP_ALIGN=1 CCC_DEBUG_TIGHT=1 "$CCC" -O2 -c "$td/good.c" -o "$td/good.o"; } 2>&1)
echo "$dump" | grep -q "func=good .*tight=yes" || bad "good: structural audit did not accept the loop"
align=$(echo "$dump" | sed -n 's/.*\[TIGHT\].*header=\.LBB\([0-9]*\) span=\([0-9]*\) log2=\([0-9]*\) align=\([0-9]*\).*/\4/p' | head -1)
[ -n "$align" ] || bad "good: assembler did not resolve a tight bucket"
note "good: tight bucket align=$align"
# The first instruction after each executable padding run (a tight header
# or any aligned label) must sit on an address that is a multiple of the
# tight bucket the writer reported.
objdump -d -M att "$td/good.o" | python3 -c '
import sys, re
want = int(sys.argv[1])
in_good = False
prev_was_nop = False
bad = False
for line in sys.stdin:
    if "<good>:" in line:
        in_good = True
        continue
    if re.match(r"[0-9a-f]+ <", line):  # next function
        in_good = False
    if not in_good:
        continue
    m = re.match(r"\s*([0-9a-f]+):(?:\s[0-9a-f]{2})+\s*(.*)$", line.rstrip())
    if not m:
        continue  # raw-byte continuation line of a multi-byte nop
    addr = int(m.group(1), 16)
    mnem = m.group(2)
    if not mnem:
        continue
    is_nop = bool(re.match(r"(data16 |cs |rep[nz]* )*nop", mnem))
    if not is_nop and prev_was_nop:
        if addr % want != 0:
            print(f"unaligned tight header 0x{addr:x} % {want} = {addr % want}")
            bad = True
    prev_was_nop = is_nop
sys.exit(1 if bad else 0)
' "$align" || bad "good: tight header not aligned to $align bytes"
gcc -O2 "$td/good.c" -o "$td/good-gcc"
"$CCC" -O2 "$td/good.c" -o "$td/good-lccc"
[ "$("$td/good-gcc")" = "$("$td/good-lccc")" ] || bad "good: runtime output differs from GCC"

# ── 2. caller: call in body rejects at layer 1, cascade remains ──
dump=$({ CCC_DUMP_ALIGN=1 CCC_DEBUG_TIGHT=1 "$CCC" -O2 -c "$td/caller.c" -o "$td/caller.o"; } 2>&1)
echo "$dump" | grep -q "tight=no (call" || bad "caller: body call did not reject tight promotion"
echo "$dump" | grep -q "\[TIGHT\]" && bad "caller: assembler must not resolve a rejected loop"
"$CCC" -O2 -S "$td/caller.c" -o "$td/caller.s"
grep -qE '\.p2align 4,,10' "$td/caller.s" || bad "caller: ordinary bounded cascade must survive rejection"

# ── 3. huge: layer 2 fail-closed on span>64, still correct output ──
dump=$({ CCC_DUMP_ALIGN=1 CCC_DEBUG_TIGHT=1 "$CCC" -O2 -c "$td/huge.c" -o "$td/huge.o"; } 2>&1)
echo "$dump" | grep -q "func=huge .*tight=yes" || bad "huge: structural audit should accept (encoded size unknown at IR)"
echo "$dump" | grep -q "\[TIGHT\].*reject=span>64" || bad "huge: assembler must reject span>64"
gcc -O2 "$td/huge.c" -o "$td/huge-gcc"
"$CCC" -O2 "$td/huge.c" -o "$td/huge-lccc"
[ "$("$td/huge-gcc")" = "$("$td/huge-lccc")" ] || bad "huge: runtime output differs from GCC"

# ── 4. tiny: provably <=4 trips gets no loop alignment at all ──
"$CCC" -O2 -S "$td/tiny.c" -o "$td/tiny.s"
fbody=$(sed -n '/^f:/,/^[[:space:]]*\.size[[:space:]]*f/p' "$td/tiny.s")
if grep -qE '\.p2align' <<<"$fbody"; then
    echo "$fbody" >&2
    bad "tiny: a <=4-trip loop must receive no .p2align"
fi
{ CCC_DUMP_ALIGN=1 "$CCC" -O2 -c "$td/tiny.c" -o "$td/tiny.o"; } 2>&1 \
    | grep -q "tight=" && bad "tiny: no tight-loop audit line expected"

# ── 5. -S never leaks the private directive; A/B ladders documented ──
"$CCC" -O2 -S "$td/good.c" -o "$td/good.s"
if grep -q "lccc_tight_loop" "$td/good.s"; then
    bad "-S: private .lccc_tight_loop directive must never leave the process"
fi
# Default (gcc tier): portable bounded cascade only.
grep -qE '^\s*\.p2align 4,,10' "$td/good.s" || bad "-S default: scalar cascade missing"
# Forced 32/64 ladders put the unconditional tier in front (even under -S).
CCC_LOOP_ALIGN_HOT=5 "$CCC" -O2 -S "$td/good.c" -o "$td/good5.s"
grep -qE '\.p2align 5' "$td/good5.s" || bad "CCC_LOOP_ALIGN_HOT=5 ladder head missing"
grep -qE '\.p2align 4,,10' "$td/good5.s" || bad "CCC_LOOP_ALIGN_HOT=5 cascade tail missing"
sed -n '/\.p2align 5/,/\.p2align 3/p' "$td/good5.s" \
    | grep -qE '\.p2align 5[[:space:]]*$' || bad "force-32 head must be unconditional (no max-skip)"
CCC_LOOP_ALIGN_HOT=6 "$CCC" -O2 -S "$td/good.c" -o "$td/good6.s"
grep -qE '^\s*\.p2align 6[[:space:]]*$' "$td/good6.s" || bad "CCC_LOOP_ALIGN_HOT=6 ladder head missing"
# Tight-off still keeps the ordinary cascade (only the strong tier is gated).
CCC_LOOP_ALIGN_HOT=off "$CCC" -O2 -S "$td/good.c" -o "$td/goodoff.s"
grep -qE '\.p2align 4,,10' "$td/goodoff.s" || bad "tight=off must retain the scalar cascade"
if grep -qE '^\s*\.p2align [56][[:space:]]*$' "$td/goodoff.s"; then
    bad "tight=off must not emit an unconditional 32/64 tier"
fi

# ── 6. -Os: fully inert ──
{ CCC_DUMP_ALIGN=1 CCC_DEBUG_TIGHT=1 "$CCC" -Os -c "$td/good.c" -o "$td/good-os.o"; } 2>"$td/os.err"
grep -q "tight" "$td/os.err" && bad "-Os: tight audit must be silent"
"$CCC" -Os -S "$td/good.c" -o "$td/good-os.s"
g_body=$(sed -n '/^good:/,/^[[:space:]]*\.size[[:space:]]*good/p' "$td/good-os.s")
grep -qE '\.p2align' <<<"$g_body" && bad "-Os: GCC emits zero .p2align and so must lccc"

# ── 7. parser: malformed marker is a hard error, well-formed assembles ──
printf '.text\n.globl fb\nfb:\n.lccc_tight_loop 123\nret\n' >"$td/bad.s"
if "$CCC" -c "$td/bad.s" -o "$td/bad.o" 2>"$td/bad.err"; then
    bad "malformed .lccc_tight_loop argument must be an error"
fi
grep -q "bad .lccc_tight_loop argument" "$td/bad.err" || bad "malformed marker error message missing"
printf '.text\n.globl fg\nfg:\n.lccc_tight_loop .Lx\n.Lx:\nret\n' >"$td/ok.s"
"$CCC" -c "$td/ok.s" -o "$td/ok.o" || bad "well-formed .lccc_tight_loop failed to assemble"

# ── 8. i686 cross compiler: same two-layer decision ──
if [ -x "$CCC32" ]; then
    dump=$({ CCC_DUMP_ALIGN=1 CCC_DEBUG_TIGHT=1 "$CCC32" -O2 -c "$td/good.c" -o "$td/good32.o"; } 2>&1)
    echo "$dump" | grep -q "func=good .*tight=yes" || bad "i686: structural audit did not accept"
    echo "$dump" | grep -q "\[TIGHT\].*align=" || bad "i686: assembler did not resolve a bucket"
    note "i686: $(echo "$dump" | grep -o '\[TIGHT\].*' | head -1)"
fi

if [ "$fail" -eq 0 ]; then
    note "tight-loop alignment regression: PASS"
else
    exit 1
fi
