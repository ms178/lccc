# Follow-up: Agent B patch rebase onto main, vectorizer dispatcher guards, and the loop-latch LEA fold

Session date: 2026-09-02 (second session of the day; supersedes nothing — this
is the continuation of `FOLLOWUP-2026-09-02-iv-widen-agentb-audit.md`)
Base: `ms178/lccc` main @ `dcf673db` (Agent B synthesis merge, PR #349, `4c7e534`)
Deliverable: `ms178-1.patch` (workspace root); snapshots `S01`+ with the
machine-readable ledger under `/home/user/artifacts/SNAPSHOT_LEDGER.md`.

---

## 1. Situation: the attached patch vs. main

The attached `ms178-1.patch` (Agent B, second revision — the "g4 battery"
work) was written against the tree BEFORE PR #349. Meanwhile main had already
merged the synthesis (`4c7e534`), which independently adopted large parts of
Agent B's revision in a different shape. The rebase was therefore a SEMANTIC
diff, not a textual one. Item-by-item verdict:

| Agent B patch item | Status on main @ dcf673d | Action taken |
|---|---|---|
| Narrow constant-count shifts as closure members | Already in main as `MemberKind::ShiftConst` (count left narrow — strictly better than Agent B's widened-count `BinOpConst`) | none (main wins) |
| `escape_read_needs_narrow(Copy) = true` | already in main | none |
| Candidacy restricted to `I32`/`U32` | already in main | none |
| `FxHashMap` use map | already in main | none |
| `binop_range` `Shl` arm | main computes the shifted range inline in the shift arm (count < 32, so no i128 overflow — equivalent) | none |
| **`CrossCast` (cross-sign widening casts)** | **MISSING** — `U32→I64` fell into the `NarrowCast` arm, whose dest is never enqueued, so the wide `Shl`+GEP below the cast were never scanned, `has_addressing` stayed false, and every `unsigned i; a[i>>1]` loop declined (verified: 32-bit counter survived at -O2) | **ported** (§2) |
| **Unsigned IVs with RUNTIME trip bounds** | the counted-bound proof (`prove_counted_bound`) already accepted loop-invariant runtime bounds — but was unreachable for the cross-sign shapes above | unlocked by the CrossCast port; pinned by tests (§3) |
| **Vectorizer narrow-IV guard** | **MISSING — bug reproduced on main**: `unsigned char i; for (i=0;i<120;i++) s+=a[i]` vectorized into a U8 counter stepping 16 toward byte-limit 480 → wraps at 256 → **infinite loop** (GCC: 7140) | **ported** (§4) |
| **Vectorizer const-zero-init guard** | **MISSING — bug reproduced on main TWICE**: `for (i=5;i<60;i++) s+=a[i]` returned 31708938240 (int/AVX2 path) and 30702305280 (long/SSE2 path); GCC: 1760 both. Agent B's guard sat ONLY in `transform_reduction_avx2` — the SSE2 path stayed broken | **ported and moved into the DISPATCHER** (both transforms gated; both are called only from there) (§4) |
| `tests/regression/iv_widen_derived_closure.c` | already on main (identical minus 3 header lines) | none |
| `tests/regression/iv_widen_latch_bound_soundness.c` | main's twin `iv_widen_latch_and_bound.c` had the **out-of-bounds defect Agent B's header called out**: `runtime_derived(a, N - 1)` with `N=400` reads `a[i*3]` up to `a[1194]` on a 400-element array — UB, GCC differential vacuous | **fixed in place** (`N=420`, call `N/3`, max index 417) |
| `tests/regression/iv_widen_unsigned_runtime_bound.c` | missing | **added** |
| `tests/regression/vectorize_narrow_iv_reduction.c` | missing | **added, extended** with the non-zero-start int/long/dot cases the SSE2 repro exposed |
| Unit tests (cross-sign chain, shl-wrap bail, foreign-sign bail, runtime-bound accum) | missing | **ported** (4 tests, adapted to the synthesis architecture) |

## 2. `MemberKind::CrossCast` (src/passes/iv_widen.rs)

* Admission is EXACT-pair: `(U32→I64) | (I32→U64)`. Members are always 32-bit,
  so these are the only cross-sign widening integer casts; the exact match
  keeps float casts (`U32→F64`, handled by the same-sign/`NarrowCast` arms
  with source-signedness extension semantics) and pointer casts out of the
  arm by construction. (Agent B's `size>=8 && !same_sign` condition would
  also have captured `U32→F64`; that happens to be value-correct under the
  retype, but exactness removes the argument entirely.)
* The dest is enqueued as a chain value (`queue.push((dest, false))`), which
  is the actual fix: the wide scaling ops and the GEP that set
  `has_addressing` live BELOW the cast.
* Apply retypes `from_ty` to the wide type — a same-size `U64→I64` /
  `I64→U64` reinterpretation, a backend no-op.
* Soundness: a cast extends per its 32-bit source's signedness, which is
  exactly the member's `ext`; the no-wrap domain (signed-overflow-is-UB, or
  the counted bound for unsigned) makes the wide member bit-identical to the
  narrow one on every executed iteration.

Result on the oracle shape (`unsigned i; s += a[i>>1]`, `-O3 -march=x86-64-v3`):
the 32-bit counter with per-iteration re-extension became a clean 64-bit
loop; instruction count **17 → 16 = GCC 16.2 parity** (clang23.1: 46,
icc 2021.10: 30, icx-latest: 63).

## 3. Runtime-bound unsigned IVs — pinned

`prove_counted_bound` already accepted runtime invariant bounds (unit step +
strict `Ult`/`Slt` exit ⇒ body ≤ n-1, latch ≤ n — no wrap by construction,
any seed). With CrossCast in place, the full g4 shape (`unsigned i; i<n;
s += a[i>>1]` with runtime `n`) widens. Pinned by
`tests/regression/iv_widen_unsigned_runtime_bound.c` (differential vs GCC,
bit-exact: 7383068127934110950) and unit test
`test_runtime_bound_cross_cast_accum`.

## 4. Vectorizer dispatcher guards (src/passes/vectorize.rs)

One block in `vectorize_with_analysis_mode`, ahead of both
`transform_reduction_sse2` and `transform_reduction_avx2` (the only two call
sites), rejecting:

1. **narrow IVs** (`I8/U8/I16/U16` induction phis) — both addressing schemes
   rescale the counter's unit past a narrow domain (byte-stride stepping and
   byte-scaled limits wrap the counter; the remainder resume
   `iv_final >> log2(elem)` starts wrong);
2. **non-const-zero IV starts** — the limit rescale assumes an element-0
   start; `c != 0` (or dynamic) starts silently drop/misread elements. The
   SSE2 reproduction is why this lives in the dispatcher and not, as in
   Agent B's patch, only inside the AVX2 transform.

These loops stay scalar — correct and tight for their (small) trip counts.
Re-enabling vectorization for them is follow-up work (§6.3/§6.4).

## 5. NEW: `fold_copy_into_lea_base` — the loop-latch coalescing fold

Closes item 4 of the previous audit (`leaq -1(%rbx),%r10; movq %r10,%rbx`
surviving regalloc). Neither existing relay pass can take this shape:

* `eliminate_move_relays` rewrites uses of the copy DEST — live across the
  back edge, never dead;
* `retarget_producer_into_copy` needs the producer reg dead after the copy —
  it feeds the exit `cmp`.

The new pass (src/backend/x86/codegen/peephole/passes/relay_and_lea.rs,
gated `CCC_PEEPHOLE_SKIP=lea_base_fold`) folds

```text
    leaq D(%rA), %rB        leaq D(%rA), %rA
    movq %rB, %rA      ->
    <reads of %rB>          <same reads, renamed to %rA>
```

Soundness contract (all three are enforced, or the candidate rolls back):

1. x86 `lea` reads all sources before writing dest — aliasing dest with the
   base is a well-defined increment; flags untouched.
2. Rename window: reads of `%rB` are renamed to `%rA` up to the first write
   of either family or a barrier; inside the window `%rA == %rB ==` producer
   value, so the rename is value-preserving (including RMW-of-`%rA` lines).
   RMW of `%rB`, memory-operand mentions of `%rB`, and implicit-usage lines
   abort the candidate.
3. `provably_dead_lv` (FileLiveness dataflow ∪ two syntactic proofs) must
   show `%rB` dead after the fold on the REWRITTEN text; failure restores
   every original line.

**Trap found during bring-up** (pinned by
`latch_fold_settled_by_dataflow_when_family_reused_in_loop`): the deadness
query must be anchored at the LEA line, not at the copy — the copy is
NOP-marked by then, and `FileLiveness` only marks real instructions as
`known`, so a query at the NOP index answers `None` and silently degrades
the proof to its syntactic fallbacks (which fail whenever the producer
family is reused elsewhere in the loop — the common `movq %rbx,%r11; shrq`
shift-copy shape).

Measured (this sandbox, no PMU — static counts + same-window timing):

* Static: **−16 instructions across 9 of 33 corpus kernels**
  (double_reduction −4, loop_patterns −3, reduction_vecreg −2,
  vector_remainder −2, gzip_crc32/histogram/nbody/sieve/global_addr_pressure
  −1 each); zero kernels grew (the fold only ever deletes the mov). The
  deleted instruction is the loop-latch increment — hot by construction.
* Runtime (same-window median-of-9, taskset-pinned, output-equal):
  double_reduction 0.989, gzip_crc32 0.994, nbody 0.995, reduction_vecreg
  0.999, everything else 1.000 — flat-to-slightly-better, no regression;
  on this 2-core VM the per-iteration save is sub-noise for most kernels.
* `shift_half` oracle: 17 → **16 = GCC 16.2 parity**.

## 6. Validation matrix (all gates green, fastbuild profile, -j2, 4G swap)

| Gate | Result |
|---|---|
| `cargo build --profile fastbuild` + `-D warnings` | clean |
| `cargo test --lib` | **1517 passed / 0 failed / 6 ignored** (32 iv_widen, 19 relay_and_lea) |
| `run_regression_suite.sh` (GCC differential + small-slots A/B) | **PASS=567 FAIL=0 SKIP=15** |
| `check_benchmark_outputs.sh` (INF-BENCHGATE-1) | **PASS=152 FAIL=0** |
| `ir_verify_sweep.py` | **1134 configs, 0 violations** |
| Agent B batteries vs GCC (`-O2`, runtime output) | 4/4 bit-exact |
| Miscompile repros (narrow-IV hang, start-5 int/long) | fixed, match GCC (7140 / 1760 / 1760) |
| Godbolt oracle `shift_half` (lccc/gcc16.2/clang23.1/icc/icx) | **16 / 16 / 46 / 30 / 63** |
| IV-widen runtime A/B (`bench_iv_widen_ab.sh`, N=9) | sieve **0.755**, tls_seg_access 0.920, arith_loop 0.979, loop_patterns 0.988, histogram/sqlite_varint/adler32 1.000, nbody 1.010 (noise band) — no regression |
| Fold runtime A/B (skip-gate, median-of-9, output-equal) | 0.989–1.000 across the 9 affected kernels — no regression |

## 7. Remaining follow-up (prioritized)

1. **Real i8/i16 IV widening** (item 1 of the previous audit, unchanged):
   needs the narrow-width no-wrap proof (counted bound ≤ 2^(w-1) signed /
   ≤ 2^w−1 unsigned) plus recognition of the promoted latch shape
   `trunc(Add(sext(phi),1):I32)`. Precisely specified; rare in real code —
   medium value, medium cost.
2. **`loop_memory_promote` + TBAA** (previous item 3, unchanged): the
   biggest remaining `stencil`-vs-GCC gap; orthogonal to this session.
3. **Vectorizer: SUPPORT non-zero const starts instead of declining.** The
   Max reduction already carries c-aware remainder math (`max_shift` in
   `insert_reduction_remainder_loop`); generalizing the limit rescale to
   `(n - c)` and the remainder resume to the c-aware form would re-enable
   vectorization for `for (i = c; ...)` sum/dot loops. The dispatcher guard
   is the safe floor; this is the win above it.
4. **Vectorizer: SUPPORT narrow IVs by widening the counter to I32 first**
   (a pre-vectorization IV widen), instead of declining. Small trip counts
   keep the value low; only worth it if a real workload shows the shape.
5. `fold_copy_into_lea_base` extensions (all conservative today):
   rename `%rB` mentions inside memory operands (base/index) in the window;
   handle RMW-of-`%rB` by keeping the result in `%rB` and renaming only the
   source reads; both need the same deadness proof and rollback skeleton
   that already exists.
6. Emitter cosmetics: conditional jumps are emitted at column 0
   (`jae .LBB3`) while instructions carry 4-space indent — harmless, but
   inconsistent with GCC-style output and mildly confusing in diffs.

## 8. Environment notes for the next session

* Swap is MANDATORY and wiped between sessions: `sudo fallocate -l 4G
  /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo
  swapon /swapfile` (or `scripts/ensure_swap.sh`, which the build wrappers
  call).
* Build with `scripts/build_lccc_fast.sh` (fastbuild profile: -O1, no LTO,
  incremental, -j2). rustup 1.98.0 lives under `/home/user/.cargo` /
  `/home/user/.rustup` — re-install after a wipe (`rustup -y --profile
  minimal --default-toolchain stable`; `rust-toolchain.toml` pins 1.98.0).
* Godbolt oracle: `scripts/godbolt.py compare <file> --local
  target/fastbuild/lccc --oracles gcc16.2,clang23.1,icc,icx-latest
  --function <fn>`; cache under `.godbolt-cache`. `DEFAULT_ORACLES` is
  already pinned to the project's version policy (GCC 16.2 = `cg162`,
  Clang 23.1.0 = `cclang2310`, ICC 2021.10 = `cicc2021100`, ICX =
  `cicxlatest` with the resolved id recorded in every manifest) — verified
  this session, no change needed. A dedicated LINKER oracle (lccc-ld vs
  mold 2.42 / lld 23.1 / bfd 2.47 link-time + layout comparison) does not
  exist yet in `scripts/`; neither mold nor lld is installed in this
  sandbox, so it could not be exercised here — build wrapper
  (`build_lccc_fast.sh`) already honors the mold preference when present.
  Candidate for a future session.
* Snapshot after EVERY validated fix: `/home/user/lccc-snapshot.sh <slug>
  "<desc>"` refreshes `/home/user/ms178-1.patch` atomically and verifies it
  applies clean to the recorded base.
