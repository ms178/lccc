# vec-chain-exit-phi: the hoisted FMA A-factor must be re-established on every entry edge

Session 2026-10-01 (session 2). Scope: the `tests/regression/matmul_chain_exit_phi.c`
corpus reconstructed for the ZERO-REM-2 / chained-exit-phi work — it was failing
before this fix, and the failure turned out **not** to be the remainders it was
written for.

## Symptom

`matmul_chain_exit_phi.c` (10 `DEFINE_MM` shapes, whole-row compare including a
16-element padding sentinel, reference built through `volatile` stores) failed on
6 of 10 shapes. Pre-fix, all six reported the same value for `C[0][0]`
(1030 vs 1003 for `mm_256_4`), and the shapes that failed were exactly the ones
whose k-loop is unrolled into **chained** j-loops.

Isolation ladder (each step cheap, each step shrinking the hypothesis):

| step | command | result |
|---|---|---|
| vectorizer off | `CCC_NO_MAP_VEC=1` | **still fails** → not (only) the vectorizer |
| opt level | `-O0 -O1 -O2 -O3` | `-O0/-O1` exact, `-O2/-O3` fail → a middle-end/backend transform |
| direct vs indirect call | `run_shape("mm_256_4", fn, …)` vs `mm_256_4()` | direct call: exact; indirect: fails |
| symbol table | `nm` | direct call **does not emit** `mm_256_4` (inlined away); the indirect call emits it as a standalone function — the standalone copy is the miscompile |
| object code | `objdump` | the standalone copy has **one** `vbroadcastsd` feeding four packed k-loops |
| IR | `CCC_DUMP_EACH_PASS=1` | after `vectorize`, four chained loops but only one `BroadcastLoadF64` (in the first loop's preheader) |
| source | `src/backend/x86/codegen/intrinsics.rs` | `BroadcastLoadF64` sets **%ymm1**; `FmaF64x4HoistedSIB` / `FmaF64x2Hoisted` read it — no SSA edge between them |

## Root cause

`BroadcastLoadF64` carries the A factor in a FIXED register (%ymm1; the AArch64
backend keeps the same implicit carrier in v15). The hoist in
`transform_to_fma_f64x2` / `transform_to_fma_f64x4` looked for the preheader as

```rust
func.blocks.iter().enumerate().find_map(|(idx, block)| {
    if pattern.loop_blocks.contains(&idx) { return None; }
    matches!(block.terminator, Terminator::Branch(label) if label == header).then_some(idx)
})
```

— an **unconditional** `Branch` into the header. The k-unroll chains the
j-loops: the exit block of j-loop N *is* the header of j-loop N+1, and that
header is entered on the **false arm of a `CondBranch`** in j-loop N's header.
`find_map` therefore returned `None` for loops 2..K … and the transform emitted
the hoisted FMAs anyway (step 3 was outside the `if let`). Those loops
multiplied by whatever %ymm1 held — loop 1's `A[i][0]` — so every later k
contributed the wrong product. Pre-fix the corpus's `C[0][0] = 1030` is the
"no A contribution at all" shape of the same bug (measured on the state before
the ZERO-REM-2/exit-phi fragments landed, where the remainder restarted at
element 0 and overwrote the vector work).

This is a class of bug — *implicit-register intrinsics with no SSA edge* — so the
invariant is stated as a contract, not a patch: **a loop whose hoisted FMAs are
emitted must re-establish the carrier on every entry edge.**

## Fix (`src/passes/vectorize.rs`)

* `fma_broadcast_entry_edges(func, pattern) -> Option<Vec<(usize, bool)>>`
  — every edge into the header from outside the loop, with `needs_split` set
  when it arrives through one arm of a `CondBranch`. `None` = no isolatable
  entry edge (no outside predecessor, or a `Switch`/`IndirectBranch` entry).
* `split_fma_entry_edge(func, pred_idx, header_idx, next_label)`
  — carves a conditional entry edge into its own block: `[rematerialised A
  chain]; BroadcastLoadF64; Branch(header)`. The header's phis are relabelled
  from `pred`'s label onto the new block (a stale incoming label is exactly the
  phi-elimination hazard the exit-phi retarget guards against). The label cursor
  is defensively re-based on the highest present label — allocating below an
  existing label corrupts the CFG, and it did here: the first draft used
  `func.next_label` directly and tripped
  `src/backend/generation.rs`'s dual-layout `permutation covers every block`
  assertion (duplicate labels ⇒ duplicated permutation entries).
* `hoist_fma_a_broadcast(func, pattern, next_val_id, next_label, debug)`
  — materialises the broadcast on **every** entry edge, splitting as required;
  returns `false` when no entry edge is isolatable.
* Both transforms now **refuse the whole transform** (return 0, no mutation)
  before touching IR when `fma_broadcast_entry_edges(...).is_none()`
  (log: `[VEC] no isolatable entry edge for the A broadcast; skipping loop`),
  and the hoist is a hard precondition of the FMA step — "broadcast or refuse",
  never "emit and hope". The doubled DOMINANCE PRECONDITION in both transforms
  was collapsed while touching that block.

Cost of the split: one new block per chained entry, executed once per **outer**
iteration (one `jmp`), zero cost inside the packed inner loop.

## Verification

Gate `tests/regression/check_vec_chain_exit_phi.sh` (wired into
`scripts/ci_local.sh --fast` and `.github/workflows/ci.yml`, parity-checked),
fixture `tests/regression/matmul_chain_exit_phi.c` +
`tests/regression/matmul_chain_exit_phi.flags` (`-O2 -march=x86-64-v3`):

1. **Correctness** — GCC-oracle parity (stdout byte-identical + exit 0) at
   `-O0..-O3`, `-Os`, under `LCCC_FORCE_SSE2=1` (two-wide lowering), and against
   the same compiler with `CCC_NO_MAP_VEC=1` and `CCC_NO_MAP_ZERO_REM=1`.
2. **Carrier ratio in the object code** — one packed loop = 1 broadcast + 4 SIB
   chunks, so `fma == 4*bc` (quad) and `fma == bc` (two-wide) must hold for
   every shape that vectorizes at all.
3. **Mechanism** — under `LCCC_DEBUG_VECTORIZE=1` every `Matmul pattern matched`
   must be followed by a `Hoisted BroadcastLoadF64` (or a refusal) before the
   next match; the ZERO-REM-2 elision must fire on the exact-multiple shapes and
   vanish under its kill switch; the chained exit-phi retarget must fire.

Measured, post-fix (whole corpus, one TU):

```
matched=46  hoisted=46  refused=0  remainder-omitted=21  retargeted>0
quad  bc/fma:  mm_256_4 4/16   mm_48_5 5/20   mm_32_2 8/32   mm_16_4 0/0
               mm_64_1  4/16   mm_255_4 4/16  mm_250_3 3/12  mm_17_4 4/16
               mm_33_4  4/16   mm_300_2 8/32
sse2  bc/fma:  identical bc, fma == bc for all nine vectorized shapes
```

**Mutation test** (red-team of the gate): the same gate run against the
pre-fix compiler fails all three contracts —

* `quad-O2: exited 1 … C[0][0] = 1030, expected 1003` (also `-O3`, `sse2`,
  `CCC_NO_MAP_VEC=1`, `CCC_NO_MAP_ZERO_REM=1`),
* `quad mm_256_4: 1 broadcast(s) for 16 packed FMA insns` (and 15 more shape
  lines; pre-fix `mm_32_2`/`mm_300_2` were 4/32),
* `matched=46 hoisted=20 refused=0` + 26 `unmatched: Matmul pattern matched`
  lines, `ZERO-REM-2 elision did not fire`, `retarget never fired`.

Static instruction counts (standalone shape functions, pre-fix → post-fix):
`mm_256_4 112→108`, `mm_48_5 136→135`, `mm_32_2 176→182`, `mm_16_4 135→135`,
`mm_64_1 95→95`, `mm_255_4 219→213`, `mm_250_3 173→163`, `mm_17_4 224→211`,
`mm_33_4 224→211`, `mm_300_2 379→365`. The packed inner loops are unchanged
(same FMA count); the deltas are the omitted remainder scaffolding (ZERO-REM-2)
and, on `mm_32_2`, six instructions of split-block entry code that execute once
per outer iteration.

Existing gates re-run and green: `check_vec_dead_remainder.sh`,
`check_vector_dot_fma_codegen.sh`, `check_ci_gate_parity.py`.

## Notes / residual risks

* The refusal path is now the safety net for every shape this pass cannot model
  (Switch/IndirectBranch entries, no outside predecessor). Refusals are visible
  in the debug stream and in contract 3 of the gate (which asserts zero on this
  corpus), so a future CFG shape that starts refusing shows up as a gate failure
  rather than as a silent scalar loop.
* `mm_16_4` (16 elements) stays scalar below the vectorizer's size floor; the
  gate asserts `fma == 4*bc` only where FMAs exist, so a future fix that
  vectorizes it passes automatically.
* The same implicit-carrier pattern should be audited for other `dest: None`
  intrinsics in this pass (`VecMulI64x2` / `Pblendvb128` withhold %xmm2 in the
  allocator — the same contract expressed differently). Not needed for this
  fix; recorded for the next session.
