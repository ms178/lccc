# Pre-existing -O2 miscompile found by extending run_slot_stress beyond seeds 1..20

**Status: RESOLVED (2026-09-03).** Root cause found and fixed in the MachInst
emitter — commit `b10ef6c0` (narrow stores of large immediates). A second,
independent soundness finding from the PR #359 red-team (FP-fold liveness
refinement ignoring control flow) is fixed in `722f7c3c`. Full resolution
evidence below; historical pre-fix analysis is preserved further down.

Original finding (kept for the record): CONFIRMED PRE-EXISTING on upstream main
`4762fea1` (and present on our rebased Tier-2 tree). NOT a regression from the
PR #358 audit / Tier-2 re-enable: reproduces identically with
`CCC_NO_TIER2_GRAPH=1` + `CCC_NO_SMALL_SLOTS=1` (fully conservative layout), so
it is layout-independent.

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

## Root cause & fix (2026-09-03) — MachInst store width contract

The IR that triggered seed 32 / `o2_vla_fill.c` is a **narrow store of a
large unsigned immediate**:

```
Store { val: Const(I64(3041712678)), ty: U32 }      ; b[i] = 6u*2654435761u + i
```

On the MachInst path this lowers to `Mov { src: Imm(v), dst: mem, size: S32 }`
(VLA element fill over a pointer, not a fixed stack slot, so the copy/phi slot
width logic of #358/#362 is not involved — which is exactly why the bug was
layout-independent and survived all four slot-layout factorial cells).

**The bug:** the emitter's large-immediate branch ignored `size` and always
emitted

```
movabsq $3041712678, %rax
movq    %rax, (%rcx)        ; an 8-byte store into a 4-byte element
```

Every element of a 4-byte-strided VLA write overran its element by 4 bytes.
With the 8-byte `unsigned long long a[]` VLA placed above `b[]`, the *last*
`b[]` element's overrun zeroed `a[0]`'s low word → first-accumulator wrong by
~5e9 (`a[0]` lost its low 32 bits). In a single-VLA frame the overrun reached
the saved VLA base pointer and the program SIGSEGV'd. `-O1`/`-Os` were correct
because the *classic* path sizes the store from the memory type; only the
MachInst relay ignored it — explaining the exact opt signature (-O2/-O3 wrong,
-O0/-O1/-Os right).

**Fix (`b10ef6c0`):** for sizes S8/S16/S32 the `mov{b,w,l}` immediate field is
a RAW {8,16,32}-bit value — unlike the sign-extended imm32 of the ALU forms —
so the constant truncates to the destination width and a single sized move is
emitted:

```
movl $3041712678, (%rax)     ; memory / stack-slot dest  (raw 32-bit imm field)
movl $3041712678, %eax       ; register dest: movl zero-extends into %rax
```

64-bit stores of >i32 immediates keep the `movabsq %rax` + `movq` relay
(`movq` imm32 is sign-extended and cannot encode them). Seven golden MachInst
emitter tests pin the width contract across S8/S16/S32/S64 for memory,
stack-slot and register destinations (incl. the register zero-extension
semantics), plus end-to-end `tests/regression/vla_largeconst_u32_fill.c`
(gcc-oracle, byte-exact across -O0..-O3/-Os) and the minimal reproducer
`tests/bugs/o2_vla_fill.c`.

Oracle lines (post-fix, match gcc byte-for-byte):
`vla_largeconst acc=13608415301065385508 a0=1663341875487337584 b3=3041712681`;
`o2_vla_fill` → `g=112725871847838 b0=3041712678 b2=3041712680 b3=3041712681`.

**Why slot-width fixes (#358/#362) could not catch it:** those protect the
*reload* side (a 4-byte slot must not be reloaded at 8 bytes) and the slot
*sizing* side. Here the slot/width bookkeeping was internally consistent (the
element genuinely is 4 bytes and is reloaded at 4 bytes) — the *write itself*
stored 8 bytes. The store width must equal the IR store width, always. That is
the invariant `b10ef6c0` restores at the single emission point.

**Second independent finding from the PR #359 red-team (`722f7c3c`):** the
FP-fold liveness refinement (accept "first later mention is a pure xmm
overwrite" as a value kill) was control-flow blind — a branch between the
consumer and the overwrite lets the taken path read the loaded value at the
merge while the fold deletes the defining load. Fixed by requiring a
control-flow-free straight-line stretch between consumer and kill in all three
affected passes (`fold_fp_register_loads`, `fold_fma_memory_src2`,
`fold_zero_addend_fma213_to_132`), with three diamond-shape regression tests
that misfold pre-fix. Not the VLA bug, but the same class (a "dead" value
proven by textual order that execution order contradicts), so it is recorded
here alongside.

## Resolution evidence (post-fix, commits `b10ef6c0` + `722f7c3c` on da964ed4)

| Gate | Result |
|---|---|
| `cargo test --profile fastbuild --lib` (full unit suite) | 1636 passed / 0 failed / 6 ignored |
| `scripts/run_regression_suite.sh` | PASS=576 FAIL=0 SKIP=15 (AB-diff failures: 0) |
| `scripts/run_slot_stress.sh 1 45 -O0 -O1 -O2 -O3 -Os` | PASS=900 FAIL=0 (seed 32 in all four layout cells now matches gcc) |
| FP differential corpus (PR #359 audit), seeds 1..60 x {-O1,-O2,-O3} | PASS=180 FAIL=0 |
| `tests/bugs/o2_vla_fill.c` + `vla_largeconst_u32_fill.c` at -O2/-O3 | byte-identical to gcc |

The run_slot_stress default range may now be widened to 1..45; the failing
cases are in the suite (`tests/regression/vla_largeconst_u32_fill.c`,
`tests/bugs/o2_vla_fill.c`) and upstream's committed gate range stays valid.

## RESOLUTION — the two independent fixes, merged (2026-09-03)

This defect was found and fixed **independently in two places**; after the
rebase onto upstream main `e2d5ef66` both fixes coexist in one tree:

1. **This branch, commit `b10ef6c0` — direct sized immediate (primary).** A
   narrow store (`size` S8/S16/S32) of a constant outside the signed-32-bit
   range truncates to the destination width and emits a SINGLE sized move:
   `movl $3041712678, (%rax)` (memory/stack-slot dest) or `movl $3041712678,
   %eax` (register dest; `movl` zero-extends). This is sound because the
   mov{b,w,l} immediate field is a RAW {8,16,32}-bit value encoding the full
   unsigned range (unlike the sign-extended imm32 of the ALU ops) — one
   instruction, no `%rax` staging, no temporary clobber.
2. **Upstream PR #363 (merged `e2d5ef66`) — sized relay (defense in depth).**
   The wide-immediate memory relay now stores at the operand's own width
   (`movabsq $imm, %rax` + `mov{suffix} %{rax_sized}, dst`), fixing the same
   VLA overrun and its float twin (an S32 hi-half store with the sign bit set,
   0xE0000000, that crashed fp_arith at -O2). The relay remains reachable for
   S64 >i32 destinations (which cannot take a direct imm) and stays as a
   safety net beneath the narrow fast path.

Both agree on the root cause: **an over-wide STORE, not a layout or IR-pass
defect** — which is why no `CCC_*` layout knob or IR-pass kill switch could
mask it and the original bisection stalled across all four "fully
conservative" configurations. The checksum chain amplifies the erased 32 bits
by 33^3, matching the observed delta exactly
(75045033622640 / 33^3 = 2.088e9 = a[0]'s lost low half).

**The direct form is the better fix** for the narrow case (1 instruction and
no `%rax` dependency vs the relay's 2 instructions + a temporary), and the
registers-destination width contract it pins (`movl` zero-extends; `movb`/
`movw` write the low lane; never `movabsq` leaking a >u{size} constant into a
64-bit register read) is not expressible with the relay alone. The relay fix
is kept as the general S64 path and the emitter-defined fallback.

Closed by: this branch's `tests/regression/vla_largeconst_u32_fill.c` +
`tests/bugs/o2_vla_fill.c` + 7 golden emitter tests (`b10ef6c0`), and
upstream's `tests/regression/vla_narrow_const_store_family.c` + `run_slot_stress`
default range now 1..45 (both upstream and this branch: 720/720 four-way /
900/900 incl. -O0/-Os).

---

## Addendum (same day, after PR #364 merged as `77eb34e1`): relay eliminated from the default emitters

After upstream merged the above two layers (#363 sized relay + #364 MachInst raw-width fast path) as
`77eb34e1`, the **default/mature** emitters still relayed constant stores: on `77eb34e1` a `-O2` build
emitted `movabsq $3041712678, %rax; movl %eax, ga(%rip)` for a global store and, with
`CCC_MI_ALL_CLASSIC=1`, relayed **every** constant store (even `ga[0] = 7`). The relay was correct but
two instructions, clobbered `%rax`, and kept a width hazard alive in the very code that must not have
one. The raw-width principle of the MachInst fast path now also governs the mature emitters (commit
`cd049d7a`):

* `memory.rs` — `pub(super) direct_store_imm(imm, ty)` centralises the raw-field contract; the main
  scalar const-store caller, the const-offset GEP store arm, and `try_emit_const_store` store directly.
* `globals.rs` — direct `movX $imm, sym(%rip)` arm at the head of `emit_global_store_rip_rel_impl`.

Post-fix `-O2` A/B on pristine `77eb34e1` vs this branch (default mode):

```
f(unsigned *p){ p[1]=3041712678u; p[2]=2147483648u; }
  77eb34e1:  movl $4294967295,(%rdi); movabsq $3041712678,%rax; movl %eax,4(%rdi);
             movabsq $2147483648,%rax; movl %eax,8(%rdi)
  branch:    movl $4294967295,(%rdi); movl $3041712678,4(%rdi); movl $2147483648,8(%rdi); ret

g(){ ga[0]=3041712678u; ga[1]=4294967295u; }
  77eb34e1:  movabsq $3041712678,%rax; movl %eax,ga(%rip); movabsq $4294967295,%rax; movl %eax,ga+4(%rip)
  branch:    movl $3041712678,ga(%rip); movl $4294967295,ga+4(%rip); ret

k(){ ga[2]=7; ga[3]=5; }          # even small constants relayed to globals on 77eb34e1
  77eb34e1:  movl $7,%eax; movl %eax,ga+8(%rip); movl $5,%eax; movl %eax,ga+12(%rip)
  branch:    movl $7,ga+8(%rip); movl $5,ga+12(%rip); ret
```

Under `CCC_MI_ALL_CLASSIC=1` upstream relayed even `0xFFFFFFFF` for register-homed stores; the branch
is direct in both text paths. Runtime parity GCC/upstream/branch: identical (`ok=1`). A direct sized
`mov` truncates by hardware to exactly the destination width, so the over-wide-store class cannot
re-enter through these emitters by construction. Pinned by `tests/regression/wide_const_imm_stores.c`
(differential vs GCC) and `tests/regression/check_wide_const_imm_stores.sh` (asserts no `movabsq`
relay, direct `movl $3041712678, ga(%rip)` etc., passes in default and `CCC_MI_ALL_CLASSIC` modes).
