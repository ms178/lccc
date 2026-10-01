# FOLLOWUP 2026-10-01 — ZERO-REM-2 + chained exit phis: the %ymm1 carrier contract

**Status:** fixed, gated, mutation-tested, CI-fast green.
**Priority:** was blocker-level (wrong code); residual items are hardening.
**Evidence:** `engineering/evidence/VEC-CHAIN-EXIT-PHI/carrier-contract.md`.

## What landed

Three things, in dependency order:

1. **Carrier contract fix (the actual blocker).** The hoisted FMA form keeps the
   A factor in **%ymm1** (`BroadcastLoadF64` → `FmaF64x4HoistedSIB` /
   `FmaF64x2Hoisted`, no SSA edge; AArch64 the same in v15). The hoist looked
   only for an unconditional `Branch` preheader, which chained k-unrolled
   j-loops do not have (their next header is entered on a `CondBranch` false
   arm), and then emitted the FMAs anyway → later k-loops multiplied by
   `A[i][0]`. Now: hoist on **every** entry edge, splitting a conditional edge
   into its own block, and **refuse the transform** when no entry edge can be
   isolated. `src/passes/vectorize.rs`:
   `fma_broadcast_entry_edges`, `split_fma_entry_edge`,
   `hoist_fma_a_broadcast`, plus the pre-mutation gate in both transforms.
2. **ZERO-REM-2 elision** (reconstructed fragment): `fma_remainder_is_dead`
   omits the scalar remainder when a constant trip count divides 16 exactly and
   the loop is closed (with the pre-mutation `fma_exit_phis_loop_invariant`
   check). Fires 21× over the new corpus.
3. **Chained exit-phi retarget** (reconstructed fragment): the exit block of
   loop N is the header of loop N+1, so `insert_remainder_loop` relabels the
   phi incoming edges onto the remainder header — without it, phi elimination
   drops the `j = 0` copy and the next loop reads an uninitialised register.

Measured along the way, in the order the work happened (useful history: the
first two fragments were necessary but not sufficient, and their counters being
non-zero while the corpus stayed red is what forced the IR-level hunt):

| state | corpus result | debug counters |
| --- | --- | --- |
| fragments absent | 6/10 shapes fail, all `C[0][0] = 1030` (no A contribution) | — |
| fragments present, carrier bug unfixed | same 6 fail, `C[0][0] ∈ {1006, 982, 1000}` (only the boundary k contributes) | 46 matched / 21 omitted / 15 retargeted / **20 hoisted** |
| carrier fix landed | **all 10 exact** | 46 matched / 46 hoisted / 0 refused / 21 omitted |

## Gate

`tests/regression/check_vec_chain_exit_phi.sh` + `matmul_chain_exit_phi.c`
(+ `.flags`: `-O2 -march=x86-64-v3`), wired into `scripts/ci_local.sh --fast`
and `.github/workflows/ci.yml` (parity check green). Three contracts:
oracle parity (incl. `LCCC_FORCE_SSE2=1`, `CCC_NO_MAP_VEC=1`,
`CCC_NO_MAP_ZERO_REM=1`), the object-code carrier ratio
(`fma == 4*bc` quad, `fma == bc` two-wide), and broadcast-or-refuse under
`LCCC_DEBUG_VECTORIZE=1`. Mutation-tested against the pre-fix compiler: all
three contracts fail.

## Open items (for the next session)

* **Audit the other implicit-register intrinsics in this pass.** `VecMulI64x2`
  and `Pblendvb128` withhold a fixed register from the allocator instead; the
  question to answer is the same for each: *is there a CFG shape where the setup
  intrinsic is not emitted on some path?* The carrier gate's shape (a fixed
  register + no SSA edge) is the generalisation worth writing down once.
* **Refusal observability.** Today a refusal is a debug line and (on this
  corpus) a gate failure. If a real workload starts refusing, nothing in
  `--fast` notices. Consider a counter in the pass summary line that the
  benchmark harness can diff.
* **AArch64 parity.** The same implicit carrier exists in v15, so the fix is
  shared, but the AArch64 lowering was not benchmarked here (no AArch64 corpus
  in this environment). One A/B on the qemu cross path would close it.
* **Perf.** The split adds one block + one `jmp` per *outer* iteration per
  chained loop; the packed inner loop is untouched (same FMA/broadcast ratio,
  verified structurally). A callgrind A/B on a matmul-heavy workload is the
  natural confirmation that the standalone-shape wins now show up (the shapes
  were miscompiled before, so no trustworthy baseline exists yet).
