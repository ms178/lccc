# Follow-up: MachInst coverage 74.5% → 85.1%, and a guard against erosion

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `7f8628e7`
Snapshots: `S33`, `S34`

---

## 1. Where the previous round left it

`CCC_ISEL_STATS=1` made MachInst coverage measurable and ranked the gap. On the
**full** corpus (562 files, 8942 instructions) the baseline was:

```
MachInst coverage: 6662/8942 = 74.5%
    963  ParamRef        10.8%
    714  Call             8.0%
    133  Store(other)     1.5%
    119  Store(float)     1.3%
     94  Memcpy           1.1%
```

## 2. `ParamRef` — 10.8%, done properly

The previous attempt returned `true` for every `ParamRef` and broke four
parameter-related tests. The lesson recorded then was "model the incoming
argument location", and that is what distinguishes the cases:

`emit_param_ref_impl` has **four** behaviours — pre-stored (nothing to do), a
parameter homed in an alloca slot (a load), an incoming ABI register copied to
a different home, and a stack-passed argument loaded from
`16(%rbp)`/`8(%rsp)`. Only the first emits nothing.

So only the first is lowered. The decision lives in `try_lower_machinst`, where
the codegen state (`param_pre_stored`, `param_alloca_slots`, `param_classes`)
is in scope — isel itself cannot see it. The other three keep the text path,
which already implements the ABI contract including the pinned rule that the
fallback must read the parameter's **incoming** register even when a
caller-saved pre-store of a *different* parameter has aliased that register
name.

**ParamRef rejections: 963 → 96.** The residual is labelled
`ParamRef(needs-code)` rather than hidden, so the next attempt knows exactly
what is left.

The gain is not only the percentage: a `ParamRef` sitting between two lowerable
instructions used to **flush the buffer**, splitting an otherwise contiguous
MachInst run across two code paths.

## 3. Narrow stores — again

The blanket refusal of `I8/U8/I16/U16` stores had returned on the merge. Narrow
stores are ordinary `Mov`s at `OpSize::S8`/`S16` and the emitter's size tables
have always handled them. `Store(other)` rejections **133 → 79**.

This is the byte traffic that dominates gzip / zlib-ng / expat, and the oracle
shows the payoff (§6): on a byte-copy loop lccc emits **14 instructions where
GCC emits 113, Clang 91, ICC 92 and ICX 41** — best of all five.

**Coverage: 74.5% → 85.1%.**

## 4. `Call` — 7.8%, and why it must not be lowered yet

`Call` is now the largest remaining class, and the honest finding is that the
current behaviour is **correct, not defective**.

`MachInst::Call` emits a bare `call target`. The MachInst layer has **no
clobber modelling** — nothing tells the allocator that a call destroys the
caller-saved set. Lowering a real call would therefore let a value live in
`%rdi` across it.

What keeps this sound today is precisely the rejection: returning `false`
triggers `flush_machinst()`, which allocates and emits the buffered run
*before* the call. **The flush boundary is the clobber model.** Verified by
spot-check that a value live across a call survives.

Lowering `Call` requires giving MachInst a per-instruction clobber set and
teaching the allocator to honour it. That is the correct next feature; adding
`MachInst::Call` uses without it would be a miscompile.

## 5. A guard so coverage cannot silently erode

The census made erosion *measurable*. This makes it *enforced*.

`instruction_selection_covers_the_expected_instruction_classes` lowers one
representative of every class the layer claims to own and names any that fall
out. It matters because `lower_instruction_typed` returning `false` falls back
to text emission **with no diagnostic**: a class quietly leaving the typed path
looks exactly like everything being fine — which is how the narrow-store
refusal came back unnoticed on a merge.

Proven non-vacuous: reintroducing that refusal fails with

```
these instruction classes fell OUT of MachInst lowering: ["Store(i8)", "Store(i16)"]
```

A second structural test pins that `GlobalAddr` lowers to `LeaSym` and **not**
to a `Mov` — a `Mov` from a `RipRel` operand loads the symbol's *contents*
while `GlobalAddr` wants its *address*, and both assemble cleanly.

MachInst suite is now **36 tests across seven layers**: table integrity, type
mapping, golden emission, assembler differential (GAS 2.47), randomized
cross-product stress, **execution**, and coverage regression.

## 6. Oracle standing (`-O3 -march=x86-64-v3`, instructions)

| function | lccc | clang 23.1 | icc | icx | gcc 16.2 |
|---|---|---|---|---|---|
| `bytecopy` (narrow stores) | **14** | 91 | 92 | 41 | 113 |
| `match_len` | **15** | 14 | 11 | 31 | 112 |
| `scan` (memchr) | **19** | 10 | 22 | 27 | 65 |

lccc is **best of all five** on the byte-copy loop, and beats GCC and ICX on
all three.

---

## 7. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1476 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=562 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3372 configs |
| MachInst 7-layer suite | 36 tests, green against **GAS 2.47.20260726** |

---

## 8. To do next, in priority order

1. **Clobber modelling in MachInst, then `Call` (7.8%).** Add a per-instruction
   clobber set and teach the allocator to honour it; `Call` lowering follows
   directly. This is the last large coverage class and the only one currently
   blocked by a missing capability rather than by effort.
2. **`Store(float)` / XMM (1.3%).** MachInst has no vector/FP register class.
3. **`ParamRef(needs-code)` (1.1%)** — the alloca-homed and stack-passed cases,
   once the incoming-argument location is modelled.
4. **If-convert the pure SESE test region** — still the biggest *performance*
   item. `namechars` emits an eleven-branch chain per byte; the chain
   `if_convert` → `range_fold` → `set_membership` is starved at the first link.
   `FOLLOWUP-2026-09-01g` documents the critical-edge trap that must be split
   first.
5. Vectorisation breadth; encoding-level differential; def-dominates-use in the
   IR verifier; linker oracle.

---

## 9. Notes for whoever picks this up

- **A rejection is not always a defect.** `Call` looks like an 8% hole; it is
  actually the mechanism that keeps caller-saved values sound in the absence of
  clobber modelling. Check *why* something is rejected before "fixing" it.
- **Split the class, not the difference.** `ParamRef` is neither all-no-op nor
  all-code. Lowering the provably-empty 90% and labelling the rest beat both
  "reject everything" and the earlier "accept everything".
- **A merge can silently undo a fix.** The narrow-store change came back;
  nothing failed, because falling back to text emission is invisible. That is
  what the coverage guard is for.
