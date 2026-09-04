# Session follow-up: unroll + vectorization red-team merge (ms178-1 × ms178-3 × PR #374)

Date: 2026-09-03 (session 09-03c)
Base: `82d7009` (upstream ms178/lccc main, contains PR #374 merge `2d3d5d7`)
Deliverable: `/home/user/ms178-1.patch` (+ `artifacts/ms178-1.Sxx-*.patch`, `lccc-src.tar.gz`, `lccc.bundle`)
Binary: `artifacts/bin/lccc-merged` (== `lccc/target/fastbuild/lccc`, md5 `e3a32010098cf83ab6213e6c4fb8ec74`)

---

## 1. What was asked and what was delivered

The user mandated a deep red-team audit of the two competing revisions of the
unroll/vectorization work — `ms178-1.patch` (Fable) and `ms178-3.patch`
(Agent B) — plus a full audit of upstream PR #374 (touched `loop_unroll.rs`),
the production of a merged "godlike" patch rebased on latest main, and proof
of the choice by real testing. The outcome is a single merged patch that takes
the validated insights of both revisions, repairs the CFG/trip-count bugs both
missed, and re-grafts PR #374's `do_unroll` foreign-predecessor fix that the
merge process had initially lost.

## 2. Red-team audit results (empirical matrix)

Corpora: `A_unsigned.c` (unsigned trip domains, near-wrap bounds), `B_carried.c`
(carried dependencies across unrolled iterations, multi-block loops),
`C_vector.c` (conditional vector reduction with a runtime guard), all compared
against `gcc -O0` refs on x86-64. Flags: A/B `-O3`, C `-O3 -march=x86-64-v3`.

| Corpus | Baseline 82d7009 | w1 (ms178-1) | w3 (ms178-3) | **MERGED** |
|---|---|---|---|---|
| A_unsigned (g1, h1) | 2 fail (4294967287 / 18446744073709551607 vs 0) | **ALL PASS** | 2 fail | **ALL PASS** |
| B_carried (first_const4, first_param4, stride_carried, header_effect) | 4 fail (+ new header_effect1 regression) | 4 fail + header_effect1 fail | **ALL PASS** | **ALL PASS** |
| C_vector (guard_nat, guard_swapped) | 2 fail (612 vs 651; wrong guard handling) | 2 fail | 2 fail | **ALL PASS** |

Conclusions:
- **w1 is right about the unsigned trip domain** (the dominant bug class: g1/h1
  are 64-bit unsigned near-wrap loops that the baseline and w3 mis-unroll).
- **w3 is right about the two-block/carried CFG shape** (exit phis, latch
  removal, header-extra conditional evaluation) — w1's single-trip path is
  *incorrect* (it drops the second condition evaluation) and was not used.
- **Neither w1 nor w3 handles the vector guard**: both silently dropped the
  Select that narrows the result to in-range lanes (guard_nat 612 insns, wrong
  value). The merge introduces proper masked-vector lowering instead.
- Baseline also mis-handled `do_unroll` exit phi predecessors; PR #374's fix
  got lost during the splice and was re-grafted from `afd6c0f→2d3d5d7`.

## 3. The merge (what the final patch contains)

### 3.1 Loop unroller (`src/passes/loop_unroll.rs`) — G1–G3, T1a/T1b, PR #374
- `try_complete_unroll_general(func, lp, cfg, trip_range)` + closed-form
  `complete_unroll_trip(iv_init, limit_n, cmp_op, iv_step, iv_ty)`;
  `canonical_continue_cmp(raw_cmp_op, iv_is_lhs, exit_pos)`; `cmp_ty != iv_ty`
  rejected (structured, typed trip math).
- Resolved constants via `resolve_const_operand`; unsigned init/limit kept as
  `IrConst::from_i64(iv_ty)` so `to_i64()` round-trips at the IV's own type
  (U8/U16/U32 stay zero-extended).
- `signed_step` / IV detection split (see §3.3): the EXTENDED detector
  `find_iv_in_loop_ext` (w1's domain reasoning: `Sub(phi,k)` countdowns +
  sign-reinterpreted unsigned constants, steps computed in `i128`) is used
  ONLY by the complete unrollers, which re-verify the step in closed form;
  `analyze_loop` keeps the strict Add-only `find_iv_in_loop` so `do_unroll`
  sees exactly the acceptance set it was written for (fail-closed).
- Two-block loops: `latch → Unreachable`, exit phis relabeled with the
  last-iteration header value, `foreign_preds` of the exit block get an
  incoming value for the merged edge (PR #374 `do_unroll` repair).
- General `trip == 1`: environment for the single iteration is
  `model.env_next(trip, env_entry, empty_vmap)` when no plans remain; the latch
  is replaced by `final_guard.unwrap_or(exit_target)` so *both* condition
  evaluations still happen (w3's semantics; w1's `1..=1` shortcut was wrong).
- Not-unrollable shapes fail closed (no partial deopt): `unroll_unsigned_domain_trip`,
  `unroll_header_extra_trip1`, `k30_unroll_shapes` regression tests.

### 3.2 Vectorizer + masked reduction (`vectorize.rs`, `intrinsics.rs`,
`regalloc.rs`, `x86/codegen/intrinsics.rs`)
- New `IntrinsicOp::VecMaskedAddI32x8` with args
  `[acc I32x8, base, byte_offset, guard_rhs]` — a conditional 8×i32
  vector-sum whose per-lane mask comes from `vpcmpgtd`.
- x86 lowering: load base+offset (`vmovdqu` → ymm0), broadcast rhs (const 0 →
  `vpxor`; const n → `vmovd` + `vpbroadcastd`; runtime value → `mov` + `vmovd`
  + `vpbroadcastd`), `vpcmpgtd %ymm1,%ymm0,%ymm1`, `vpand`, `vpaddd` into the
  accumulator. Scratch confined to ymm0/ymm1 so the accumulator may live in
  ymm2..ymm15.
- Regalloc: `class-of = Some(5)` with a `legal_consumer` whitelist so the
  new op participates in width partitioning without risking illegal temps.
- Vectorizer emits the masked intrinsic whenever the conditional-sum has a
  guard condition and splices out the (previously dropped!) Select; non-I32
  guarded sums are rejected fail-closed.

### 3.3 Red-team round 2 — `unroll_stress.py` found two more miscompiles (fixed)

The exhaustive differential (4000 enumerated loop shapes, lccc -O3 vs
lccc -O3+`CCC_DISABLE_PASSES=unroll` vs gcc -O0) found **2 real miscompiles**
(the other 37 "fails" were all-three-compiler RUN TIMEOUTs of slow programs):

| cfg | loop | correct | baseline | w1 | w3 | merged (before) |
|---|---|---|---|---|---|---|
| 1765 | `long i=5; i <= lim; i += 7`, nested body | 30 | **0** | 30 | 30 | **0** |
| 3169 | `unsigned long lim != i; i -= 1`, nested body | 126 | 126 | 126 | 126 | **252** |

Root causes (diagnosed with `CCC_TRACE_UNROLL` instrumentation + per-pass IR
dumps on instrumented baseline/w1/w3 builds):

1. **cfg 1765 — `do_unroll` exit-value bug (BASELINE bug, inherited by the
   merge).** The loop's body is a straight-line chain left behind by
   complete-unrolling the inner loop; the RETURN block uses the header phi
   directly (no exit-block phi). Step 5b correctly INSERTED an exit phi with
   per-edge values but the reader-rewrite covered only *instructions* — the
   **terminator** (`Return(v27)`) kept the header phi's name, which on the new
   exit-check edges holds its PREHEADER value → returned 0 instead of 30.
   Fix: also `subst_value_in_terminator` in the Step 5b rewrite walk. (w1/w3
   only *accidentally* avoided the bug: their invocation-1 CFG kept the dead
   inner latch inside the outer body, so `analyze_loop` rejected the shape.)
2. **cfg 3169 — merge-specific acceptance leak.** The extended
   `find_iv_in_loop` (Sub-form countdown + unsigned sign-reinterpretation)
   reached `analyze_loop`, so `do_unroll` accepted a countdown IV whose step
   semantics its guard arithmetic cannot express → 4x clone chain whose early
   exit paths fell through with the previous iteration's accumulator (252).
   Fix: split detectors (see §3.1) — strict Add-only for the partial path.
3. **Fail-closed hardening added to `analyze_loop`:** (5b) body-connectivity
   — every body block must be reachable from the header without the back edge
   (rejects detached clone cycles); (5c) body_work must be a simple linear
   chain header→work→latch with exactly one chain tail, no CondBranch, no
   merges — the exact precondition `do_unroll`'s guard arithmetic was derived
   for. Anything else stays rolled (correctness over unrolling).

Validation of the fixes: cfg 1765 and 3169 now return the correct values; a
trip-range differential (lims 0/4/5/11/12/18/19/100/1000000 for the Add form,
lims 0–6 for the countdown form) matches gcc -O0 exactly. Both shapes are
pinned as `tests/regression/unroll_chain_body_exit_phi.c` and
`unroll_countdown_chain_body.c`.

## 4. Validation evidence (all against the deliverable binary)

| Check | Result |
|---|---|
| `cargo test --lib` (clean-room, AFTER the round-2 fixes) | 1698 passed / 0 failed / 6 ignored |
| full regression suite (`run_regression_suite.sh`, incl. the 2 new round-2 tests) | **PASS=595 FAIL=0 SKIP=15** (AB-diff 0) |
| A/B/C red-team corpora + guard.c | PASS (A -O3; B -O3; C -O3 -march=x86-64-v3) |
| `vector_guard_sum` / `unroll_header_extra_trip1` oracles | PASS (guard_nat=651, trip==1 evaluated twice) |
| masked codegen (`guard.c -S`) | `vpcmpgtd`/`vpand`/`vpaddd` confirmed |
| `unroll_stress.py` (exhaustive differential vs gcc -O0, 4000 configs, post-fix) | **0 mismatches** (38 RUN-TIMEOUTs, all-compiler harness noise; round-1 had 2 real miscompiles which are now fixed) |
| codegen oracle `k30_unroll_shapes.c` | lccc 361 insns, best of 5 (gcc 434 / clang 363 / icc 370 / icx 384) |
| codegen oracle k01–k15 (`-O3 -march=x86-64-v3`) | lccc 306 total insns — best aggregate (icx 457, icc 519, clang 610, gcc 945); 5/15 per-file best |

Oracle aggregate per file (lccc / 2nd best | winner):

| file | lccc | best | winner |
|---|---|---|---|
| k01_adler | 70 | 65 | icx |
| k02_sum8 | 14 | 14 | lccc |
| k03_crc | 26 | 17 | gcc |
| k04_strlen | 10 | 1 | clang |
| k05_max | 21 | 21 | lccc |
| k06_dot | 14 | 14 | lccc |
| k07_bswap | 13 | 12 | icc |
| k08_bcopy | 6 | 6 | lccc (tie) |
| k09_clz | 3 | 2 | clang |
| k10_ffs | 20 | 11 | gcc |
| k11_swp | 8 | 7 | tie |
| k12_hash | 16 | 15 | gcc |
| k13_strcmp | 25 | 12 | gcc |
| k14_isort | 41 | 19 | icx |
| k15_bytemask | 19 | 19 | lccc |

## 5. Files added/changed in this session (all in the patch)

- `src/passes/loop_unroll.rs`, `src/passes/vectorize.rs` (M)
- `src/ir/intrinsics.rs`, `src/backend/regalloc.rs`,
  `src/backend/x86/codegen/intrinsics.rs` (M)
- `tests/regression/{vector_guard_sum,unroll_header_extra_trip1,unroll_unsigned_domain_trip,loop_unroll_redteam,unroll_chain_body_exit_phi,unroll_countdown_chain_body}.c(+.flags)`
- `tests/benchmark/kernel_corpus/k30_unroll_shapes.c`
- `scripts/unroll_stress.py`, `scripts/lccc-harness-snapshot.sh` (new)
- `scripts/codegen_oracle.py` (+ `--totals`: cross-source per-compiler aggregate
  of insns/loads/stores/spills/branches/vectors + per-source bests; the summary
  table in §4 was produced with it)
- `tests/linker/setup_oracles.sh` (removed stale `bench_linker.py` references)
- goldens: gzip/zlib-ng/expat/SQLite/linux/glibc workload kernels (earlier
  session, provenance in `tests/benchmark/WORKLOAD_PROVENANCE.md`)

## 6. To-do (ranked, next session)

1. **Codegen gaps visible in the oracle sweep** (biggest wins first):
   - `k03_crc` (26 vs gcc 17): scalar CRC recurrence is 1.5x larger — look at
     the shift/xor chain; consider `crc32` instruction only if semantics allow.
   - `k13_strcmp` (25 vs gcc 12): byte-compare loop — investigate
     word-at-a-time + `pmovmskb`/`bsf` pattern (lccc has `strlen`-family work
     in k04 but it falls back to a libc call for 1-insn string functions).
   - `k10_ffs` (20 vs 11), `k14_isort` (41 vs icx 19), `k01_adler` (70 vs 65).
   - `k04_strlen`: 10 insns incl. a store vs clang 1 (pattern-recognition
     into `strlen` call/builtin). Not a real perf loss; low priority.
   - k12_hash + k15_bytemask are close (16 vs 15, 19 vs 19 tie) — micro.
2. **Run the linker oracle battery** (`tests/linker/run_linker_tests.py`,
   `real_workloads.py`) with `setup_oracles.sh` — not re-run this session;
   oracle policy (git-HEAD mold/wild, bfd 2.47, `MOLD_TARGETS=X86_64;I386`)
   was verified only statically this session.
3. **Extended differential fuzzing**: run `unroll_stress.py` with a bigger
   `--limit` / fresh seeds, and `csmith_diff.py` / `yarpgen_diff.py` slices
   focused on header-condition + multi-exit loops.
4. **Benchmark on real hardware when available**: k01–k15 + golden workloads
   through `run_benchmarks.py`; VM has no PMU so use closed-form + assembly.
5. **Re-check the merge against next upstream head** (rebase workflow:
   `lccc-snapshot.sh` before every upstream pull).

## 7. Risks / known limitations

- `VecMaskedAddI32x8` handles **I32 only** by design; I64/I16 guarded
  conditional sums are rejected during analysis (fail closed). An I64 variant
  (`vpcmpeqq`-style widening or 2×I32 lanes) is the natural next step if a
  golden workload needs it.
- Trip-count closed form uses i128 intermediates; loops whose iteration count
  exceeds 2^127 are not representable (unreachable in practice for counted
  loops with 64-bit IVs).
- The `trip == 1` path now goes through the general model (env_next with an
  empty plan list); it is covered by `unroll_header_extra_trip1` but a
  dedicated `trip==1` × every cmp-op × signed/unsigned sweep is still missing
  (see to-do 3).
