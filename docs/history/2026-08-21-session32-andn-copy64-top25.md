# 2026-08-21 session 32 — Top-25 continuation: ANDN, 64-byte YMM, and blocker spikes

**Base:** latest `ms178/lccc/main` at
`4b2effebcd568fe913970a2f1757cd13625d6024` (PR #156).

## Completed treatments

### IS-12 — BMI1 ANDN fusion

The target feature contract now carries BMI1 into x86 codegen. The shared
instruction walk recognizes an adjacent, single-use `not` followed by `and`;
x86 emits `andn{l,q}` using the operands' assigned physical registers rather
than staging both through fixed scratch registers. Baseline x86 emits the
original NOT+AND pair, so no unsupported instruction can leak without
`-mbmi`/an enabling `-march`.

Linux `find_next_andnot_bit`, paired VM screening (15 rounds):

- treatment vs `CCC_NO_ANDN_FUSION`: **0.965**, about **4% faster**;
- treatment vs GCC 14: 1.421 (remaining ffs/cmov lowering gap is explicit).

A direct-return fast path writes `%rax`, matching GCC/Clang/ICX at two
instructions (`andn; ret`; ICC uses four). The regression checks result
semantics, ANDN presence under x86-64-v3, NOT elimination, and absence of ANDN
on baseline x86-64.

### IS-03 — 64-byte AVX2 assignment

A target-gated 64-byte copy now emits:

```asm
vmovdqu   0(src), ymm0
vmovdqu   ymm0, 0(dst)
vmovdqu  32(src), ymm1
vmovdqu   ymm1, 32(dst)
vzeroupper
```

With memcpy ParamRefs kept in their incoming ABI homes, the isolated function
shrinks **55 -> 38 bytes (-17 from YMM selection, -27 vs the original 65-byte
path)**. Memcpy ParamRefs remain eligible for their incoming ABI homes, removing
the destination entry spill/reload. Compiler Explorer counts LCCC 9
instructions, ICC 9, and GCC/Clang/ICX 6; the remaining three are parameter
setup, not copy instructions. Baseline targets stay XMM-only. The threshold is
evidence-based: enabling YMM for the benchmark's 48-byte Particle copies made
it **25.7% slower** than the XMM path, so 32/48 B remain unchanged. Whole `struct_copy` currently measures LCCC 83.32 ms vs GCC
23.25 ms (3.58x); its dominant gap is aggregate scalar replacement and loop
structure, not the now-optimal isolated 64-byte primitive.

### Pre-existing IS-28 revalidated

`bit_idioms.rs` already recognizes the canonical 32-bit SWAR popcount and
lowers it to `UnaryOp::Popcount`; current x86-64-v3 output contains `popcntl`.
The backlog is corrected to DONE rather than asking for a duplicate emitter.

## Follow-up spikes and decisions

### OP-02 aggregate copy-out

A proposed loop-shape relaxation was tested and reverted. `CCC_SROA_COPYOUT=1`
did not transform the current `struct_copy` shape (`CCC_DEBUG_SROA` reported no
changes, +2.2% timing noise) and made `structs_bitfields` fail at its by-value
return check (exit 15), although `simd_sse_float` passed. The upstream proof is
left unchanged/default-off; a real fix needs dominance plus ABI return/copy
reasoning, not removal of the cycle guard.

### RA-24 explicit register locations

A `SlotAddr::Reg(PhysReg)` migration spike exposed **47 exhaustive match sites**
across shared traits, F128, x86, i686, AArch64 and RISC-V memory paths. The
spike was rolled back rather than filling those sites with dummy/unreachable
arms. Correct completion requires each backend to consume the physical home,
not merely replace `StackSlot(0)` with a panic. The inventory is retained as
implementation evidence for the next session.

### OP-04 vectorize gate

The backlog's `vectorize_gate` function is currently unreferenced; production
vectorization is selected directly in `passes/mod.rs` and already skips
`-Os/-Oz`. A useful PGO gate must be connected at per-loop granularity, not
implemented in the dead function. This is documented rather than claimed done.

## Top-25 status after this session

Completed/validated from the current priority chain now include P0-01a,
P0-02 residual treatment, P0-03, P0-04/OP-01, RA-04, RA-13, RA-26, IS-01,
IS-03, IS-12, and IS-28. P0-01b remains correctly blocked on RA-23. RA-24 has
an exhaustive migration inventory. OP-02 and OP-04 have measured blockers.
The still-open high-impact work is RA-01/02/03/05/06/23/24, IS-02/06,
OP-04/05/06, P0-05, and RA-25.

## Validation

- warning-free fastbuild after removing the pre-existing magic-div warning;
- 968 unit tests, 6 ignored;
- 50/50 correctness;
- 365/365 lccc-only regression passes; comparison run retains the same six documented GCC-oracle mismatches;
- 600/600 phi-CFG differential;
- 540/540 alias differential (`O0/O2/Os`);
- ANDN structural/runtime regression;
- 64-byte YMM structural/runtime regression;
- baseline ISA checks for both new instruction selections;
- graph coloring remains default-off; segment/CRC/LICM treatments from PR #156
  remain enabled and unchanged.

## Next best actions

1. RA-23: make accumulator residency an explicit location so Copy/Tier-2
   coloring can be enabled safely.
2. RA-24: migrate the 47 inventoried matches backend-by-backend with real
   register consumption and architecture tests.
3. Struct SROA: model store-built aggregates consumed through dynamic nested
   GEPs; this is the real 3.58x `struct_copy` gap.
4. Find-bit: recognize the portable ffs decision tree into a target-neutral
   operation, then select the GCC-like CMOV chain.
5. Connect PGO vectorization profitability per loop; do not wire the currently
   dead function at function granularity.
