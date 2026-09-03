# Typed 128-bit pair store/load — the Store(other)/Load(other) census classes closed

Session 2026-09-03 (follow-up round).  Census at the tip ranked `Store(other) 79`
(i128/F128 pair stores on x86-64) as the largest MachInst-actionable class after
the S26/S27 work; `Load(other) 7` mirrored it.  This note documents the typed
route that closes both to zero and the two soundness gates the route needed.

## The shapes (oracle-measured)

The mature text path materialized the value into the accumulator pair, SPILLED
the pair (emit_save_acc_pair) and stored both halves from the spill slots:

  store_i128 (value):  7 instrs + 16B dead frame traffic   gcc: 2 (2x movq)
  store_zero:          4                                    gcc: 2 (pxor+movaps)
  store_const:         4                                    gcc: 2 (pool movdqa+movaps)

## The route

Three typed forms, cheapest first (isel Store/Load arms, `ty.is_128bit()`):

* Const: two S64 Movs at dst and dst+8 — the halves ARE the data; each is an
  imm32-window `movq $imm` or the emitter's hardened wide-imm relay (which
  stores exactly 8 bytes at the operand's own size; S27's width contract).
  No staging register, no spill, no pool entry.
* Value (slot-homed — the universal i128 home): the new atomic `Mov128` —
  `movdqu src, %xmm0` + `movdqu %xmm0, dst`.  Both loads semantically precede
  both stores (the pair is data-dependent), so overlapping regions stay
  correct.  `movdqu`, never `movdqa`: frame slots sit at 8 mod 16 and pointer
  targets may be packed-member aligned (packed128 probe verified) — on modern
  cores unaligned costs nothing at aligned addresses.
* xmm0-scratch live (an incoming float param home at PhysReg 18/19): the GPR
  fallback — four S64 Movs through the reserved rax/rdx, both loads before
  either store.

`movdqu` vs the oracles' `movaps`: gcc/clang/icc prove alignment from their
IR; lccc's frame layout puts i128 slots at 8 mod 16 (measured) and its IR
does not track packed-member alignment (packed probe), so unaligned-safe is
the only sound spelling.  Instruction counts match the oracles regardless
(zero: 2 vs 2; const: 2-3 vs 2+pool entry).

## Gate 1: genuine i128 carriers, not the I128-typed vector alias

lccc's IR reuses I128 as the 16-byte VECTOR carrier type (v4sf/v4si).  A
vector value's spill slot is written lazily by the vec machinery
(flush_pending_vec_store), so reading it as a 16-byte home loads stale bytes:
`builtin_ia32_vector_value` (the kernel NAP-governor shape) printed zeroed
upper lanes.  Gate: `state.is_i128_value(v)` — the state's i128 registry is
the type truth.  Suite-caught, suite-pinned.

## Gate 2: coherent slot homes (def-site fixpoint)

A value computed into the accumulator pair has a slot whose only writer would
be the very store being lowered; the mature path stores the pair directly,
the typed route would read the never-written slot.  The route therefore
requires the val's home to be COHERENT — written eagerly at definition:

* seed: the ParamRef of an i128 parameter (prologue pre-store spills the
  incoming pair) and the result of a call returning a 128-bit aggregate
  (the call-result machinery writes the pair);
* propagate to a fixpoint through Copy-from-coherent (the mature i128 copy
  materializes the pair into the dest slot) and through i128 Load dests
  (the load itself is the defining write).

The Load arm needs no coherence for its dest (the load IS the defining
write); it keeps the genuine-carrier gate.  Volatile, segment overrides,
over-aligned allocas, folded addresses and indexed addressing stay on the
mature path (volatile access granularity stays the oracle-matching two
8-byte halves).

## Scoreboard (live godbolt oracle, -O2)

  store_val:  gcc 15.2/16.2, clang 23.1.0, icc 2021.10.0, icx latest — all
              2 direct stores; lccc store proper = the atomic Mov128 pair;
              the residual 2 instrs are the param-home materialization
              (the ParamRef(needs-code) census class, a separate layer).
  store_zero: 2 vs 2 (no xmm clobber on lccc's path).
  store_const: 2-3 vs 2+rodata entry (no pool, no load port traffic).
  Census: Store(other) 79 -> 0, Load(other) 7 -> 0; coverage 48.85% -> 49.06%.

## Tests

machinst_tests.rs: Mov128 golden emission (slot<->pointer, both directions),
assembler-corpus coverage (4 memory-operand combinations through the real
assembler), const-store two-Mov halves with exact immediate spellings,
value-store Mov128 shape, scratch-unsafe GPR fallback ordering (reads
strictly before writes), load mirror, volatile refusal, register-homed-value
refusal.  tests/regression/i128_pair_store_load.c: runtime oracle for const/
zero/value/packed/array/member-copy/volatile/global shapes, self-checking
plus suite AB-diff.

Validation: units 1677/0/6; suite PASS=571 FAIL=0 SKIP=15 AB-diff 0
(includes the new regression); vector_value regression PASS; i128 runtime
probes byte-identical to gcc -O2.
