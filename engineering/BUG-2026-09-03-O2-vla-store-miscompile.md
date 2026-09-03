# Pre-existing -O2 miscompile found by extending run_slot_stress beyond seeds 1..20

**Status:** CONFIRMED PRE-EXISTING on upstream main `4762fea1` (and present on
our rebased Tier-2 tree). NOT a regression from the PR #358 audit / Tier-2
re-enable: reproduces identically with `CCC_NO_TIER2_GRAPH=1` +
`CCC_NO_SMALL_SLOTS=1` (fully conservative layout), so it is layout-independent.

## Symptom

`scripts/run_slot_stress.sh 21 45 -O0 -O1 -O2 -O3 -Os`: seed 32 fails at -O2
and -O3 in **all four** layout configurations; -O0/-O1/-Os pass.

```
lccc -O2 = 13301157349257968134        (also -O3)
gcc  -O2 = 10994965426039773268        (gcc -O3 identical)
```

Only seed 32 fails across seeds 21..45. Seeds 1..20 (the upstream-committed
range) pass 400/400 including the four-way factorial.

## Small reproducer (own construct, same opt signature)

`tests/bugs/o2_vla_fill.c` (or /tmp/vla1.c):
```c
#include <stdio.h>
#include <stdint.h>
__attribute__((noinline)) void barrier(void){}
unsigned long long g;
int main(void){
  unsigned n = 4;
  unsigned long long a[n];
  unsigned int b[n];
  for (unsigned i = 0; i < n; i++) { a[i] = 5ull*0x9e3779b97f4a7c15ULL + 7ULL + i;
                                     b[i] = 6u*2654435761u + i; }
  barrier();
  for (unsigned i = 0; i < n; i++) g = g*33 + (a[i] + b[i]);
  printf("%llu\n", g);
  return 0;
}
```
lccc: -O0/-O1/-Os correct; -O2 and -O3 wrong by ~5e9 in the first element
(12877992986365140340 vs 12878068022990762980). gcc agrees across all levels.
NOT fixed by CCC_NO_DSE / CCC_NO_LOOP_INVERT / CCC_NO_MERGE_BLOCKS (which *do*
mask seed 32), nor by CCC_NO_IV_WIDEN / CCC_NO_LOOP_ROTATE / CCC_NO_IVSR /
CCC_NO_MAP_VEC / CCC_NO_STENCIL_VEC / CCC_NO_FIXED_SLP / CCC_NO_SIMPLIFY /
CCC_NO_BIT_IDIOMS / CCC_NO_IF_CONVERT.

## Pass-level observations (seed 32)

- -O2 vs -Os pass sets differ by exactly: `vectorize`, `loop_unroll_post_vec`,
  `iv_widen`, `loop_rotate`, extra `simplify`/`bit_idioms`, extra
  `if_convert`, `dce-post-dse`, and a second cleanup cycle (`iter=2`).
- -O2 vs -O3: only extra pass is `loop_unroll` (seed 32 -O2 and -O3 produce
  the *same* wrong value, so the unroller is not the trigger).
- First IR divergence in -O3 dumps is at `after iter=0 vectorize` (unrolled
  straight-line VLA-init body).
- -O2 DSE removes 57 stores at iter=0 including `Store ptr: Value(0)` (g_sink)
  stores — the mix chain makes those legitimately dead — plus
  `Const(I8(0))/Const(I64(0))` stores whose targets (Value 123/248/251/252)
  need alias follow-up. No single removed store has been conclusively pinned
  yet.

## Recommended next steps

1. Dump -O2 IR immediately around `iter=1`/`iter=2` `simplify`/`bit_idioms`
   for vla1.c and diff against the -Os pipeline output to find the first
   transformed store.
2. Suspect: an index/fold interaction in `iv_widen`/`iv_strength_reduce` on a
   VLA GEP (base `Value(60)`/`Value(65)` in seed 32), or DSE misjudging a
   store dead across the `barrier()` boundary because the barrier is a
   noinline call and alias analysis over-approximates/under-approximates the
   VLA alloca set. Test a `static` fixed array (`a[4]`) variant: if that is
   correct, scope is VLA-typed allocas specifically.
3. Do not add the failing cases to tests/regression until fixed (they are
   intentionally absent). Keep the run_slot_stress default range at 1..20
   (upstream's committed gate), which passes 400/400.

## RESOLUTION (2026-09-03, red-team audit session)

FIXED — and the root cause is NOT in the layout, contrary to the "candidate
defects" above. A/B builds of the exact upstream commits isolate it:

- `c8683317` (PR #362) still reproduces both failures at -O2/-O3.
- upstream + the S26 MachInst commit (wide-imm memory-relay operand-size
  hardening) passes both, matching gcc.

Root cause: a 32-bit constant outside the signed imm32 window (b[3] =
3041712681) is staged with `movabsq $imm, %rax` and stored by the wide-imm
memory relay at its DEFAULT S64 width (`movq`), writing 4 bytes past the
16-byte VLA — exactly a[0]'s low half on the shipped stack ordering. The
checksum chain amplifies the erased 32 bits by 33^3, matching the observed
delta (75045033622640 / 33^3 = 2.088e9 = a[0]'s lost low half). This is the
integer twin of the S26 float-relay defect; the shared-relay hardening fixes
both. Layout knobs could never mask it (over-wide STORE, not a sharing
collision); no IR pass knob can mask it (the wrong instruction is emitted for
-O2-unrolled straight-line VLA stores).

Closed: tests/regression/vla_narrow_const_store_family.c (verified failing on
c8683317, passing on the fixed tree), slot-stress default range extended to
1..45 (720/720 four-way). Full analysis:
engineering/AUDIT-2026-09-03-pr359-362-redteam.md.
