# FOLLOWUP 2026-10-01E — post-rebase reconstruction, red-team audit, and the first hard-data performance baseline

Status: **rebased reconstruction, all targeted gates green, audit fixes in**. No
performance claim in this document rests on wall-clock: every number is either a
Godbolt-oracle static instruction count or a Callgrind `Ir` count, produced by
`scripts/codegen_oracle.py` and `scripts/callgrind_ab.py` on the tree described
in §1.

## 1. Tree

* Recovered from `artifacts/lccc.bundle` after the harness wiped `.git` (the
  second time). The bundle carried **more** than the pre-session audit believed:
  the session-3.5 commits `3ea25c31` (ZERO-ROT-AFFINE: `canonicalise_affine_exit_cmps`,
  region-relative `FileLiveness`, fusion accepts the rotated latch) and
  `145c1bac` (ZERO-REM-2 elision, chained exit-phi retarget, `%ymm1` carrier
  contract) are **git objects**, not just worktree content. Both were rebased
  onto upstream main along with the salvage and reconstruction commits.
* Upstream main had been force-updated to `419307ca` (PR #710, `d6e49e2b`):
  the byte-compare window phase (+955 lines of `vectorize.rs`, +22 of
  `mod.rs`). The rebase of the four local commits onto it was **conflict-free**,
  which is worth recording because it is not obvious: the salvage commit's
  `vectorize.rs` content happened to align with the already-rebased session-3.5
  work, so git could replay the changes without a merge conflict, and the
  bytecmp code survived intact (16 `bytecmp` hits in `vectorize.rs`, 2 in
  `mod.rs`, and the upstream test suite still passes — see §3).
* Reconstructed on top, and now committed:
  * `vectorize.rs` — general `header_entry_edges(func, loop_blocks, header_idx)`,
    `Switch`/`IndirectBranch` entries into a loop header refused, the FMA
    broadcast hoist takes the validated edge set (session-4 hardening), plus
    `mod entry_edge_tests` (7 tests).
  * `loop_rotate.rs` — the AFFOLD phase: `fold_affine_exit_compares`,
    `loop_iv_shape`, `loop_exit_threshold`, `fold_loop`, `plan_affine_fold`.
  * `passes/mod.rs` — the `affold` phase, `iter == 0`, after rotation, default-on
    at `-O2`+ and on the size pipelines, kill switch `CCC_NO_AFFINE_EXIT_FOLD`,
    report `CCC_DEBUG_AFFINE_FOLD`.
  * `.github/workflows/ci.yml` — the three gate steps the wipe dropped from the
    workflow file (`check_rmw_sib_folds`, `check_select_from_compare`,
    `check_affine_loop_fold`); `scripts/ci_local.sh` had kept them.

## 2. Red-team audit of the reconstructed fold — two premises made explicit

Auditing my own reconstruction with the written proof next to the code found two
premises that the proof **relied on but never checked**. Both are fixed; the
argument for fixing rather than documenting follows each.

### 2.1 Branch polarity (`loop_exit_threshold`)

The threshold `T` is derived from the loop's one exiting `CondBranch` under an
assumed polarity — "continue while the test is `true`" for `<`/`<=`, "continue
while it is `false`" for `>`/`>=`. Those two pairings are exactly the ones whose
continuation set is a **prefix** of the IV sequence, which is what makes
`L = ceil((T - start)/step)` the iteration count and `last = start + L*step` the
last IV value the loop can observe. The other two pairings (`continue on true`
with `>`, `continue on false` with `<`) continue while the IV is **above** the
bound: an upper interval, about which the trip-count obligations say nothing —
`T` would be the bound of the *complement*.

The guard now reads which successor stays inside the loop and admits only the
matching operator pair; the upper-interval pairings are refused. This is
**latent, not reachable** through the current C frontend, and I want that on the
record rather than dressed up: in the acceptance set the "wrong" pairing also
makes the *first* test fail, so the loop leaves before the mis-bounded range is
reached, and I could not construct a miscompile from it. It is still a defect: a
correctness argument that survives only through an incidental second effect of
another guard is one relaxed guard away from being a miscompile, and the check
costs one comparison against `cfg` data the function had already computed.

### 2.2 Phi edge side (`loop_iv_shape`)

The seed/step classification looked at operand *kind* (a constant anywhere is
the seed, a `phi + C` anywhere is the step) and threw away the edge labels the
phi hands it. A constant arriving on the **back** edge is then taken as the
seed: `start` becomes a value the first iteration never takes, and the no-wrap
obligation covers the wrong range. Classification is now by side — outside the
body = entry edge, must be a loop-invariant constant (written directly or
through a `Copy` defined outside the body); inside = back edge, must be
`phi + Const`. Ambiguous phis (not exactly two incomings, or both on one side)
produce no shape and hence no fold.

### 2.3 Cost of the guards: zero

`check_affine_loop_fold.sh` reports the same fold counts before and after
(**26** folds default, **6** with `CCC_LOOP_ROTATE=1`, **0** under
`CCC_NO_AFFINE_EXIT_FOLD`), and the unit test module went from 13 to 19 tests
(4 new: both refusals, the non-constant entry operand, and the exit-on-true/`>`
orientation pair, which documents that the refusal there is the orientation
rule and not the polarity rule).

## 3. Gates

| gate | result |
|---|---|
| `check_affine_loop_fold.sh` | PASS — parity vs GCC across 7 configurations, 26/6/0 folds, `cmpq $4092` + `movabsq $9223372036854775803` pins, refusals pinned |
| `check_affine_exit_compare.sh` (session-3.5) | PASS |
| `check_vec_chain_exit_phi.sh` (session-3.5) | PASS |
| `check_rmw_sib_folds.sh` | PASS |
| `check_select_from_compare.sh` | PASS |
| `cargo test --lib` (targeted modules) | loop_rotate 19/19, `vectorize::entry_edge_tests` 7/7 |
| `scripts/ci_local.sh --fast` | see the snapshot ledger for the stamped result of this tree |

## 4. Their choices: the bytecmp phase (PR #710)

Asked to review upstream's work rather than only my own. **I agree with it**, and
the agreement is concrete, not polite — three properties are the ones I would
have demanded:

1. **The entry-edge hazard is already covered.** The phase declines unless
   `cfg.preds` for the header is *exactly* `{preheader, latch}`
   (`"header predecessors are not exactly {{preheader, latch}}"`). That is the
   same class of check as the session-4 `header_entry_edges` helper, done
   directly on the predecessor set; a `Switch`/`IndirectBranch` into the header
   is caught by it.
2. **Exactness is positional, not relational.** The `ctz(~mask)` reduction hands
   the scalar header the *exact* first-mismatch pointer pair, and the module
   comment records that the earlier window-start handoff was rejected because it
   re-scans up to `WIDTH-1` bytes — the shape that dominates short matches in
   real compressors.
3. **The q window's read-ahead is bounded by a page rule, not by a size
   assumption**, and the residual (object-granular instruments can flag the
   read) is documented with a kill switch (`LCCC_NO_BYTECMP_VEC=1`) instead of
   being left for a bug report to discover.

One consistency note, not a defect: the repo now has two mechanisms for "which
edges enter this header" (`header_entry_edges` and the bytecmp `cfg.preds`
check). They agree; unifying them is churn for no behaviour change, so it is
recorded here rather than done.

## 5. Hard data I — Godbolt oracle, whole corpus

`scripts/codegen_oracle.py --rank tests/benchmark/programs/*.c` with
`-O2 -march=x86-64-v3` against `gcc16.2, clang, icc, icx`
(artifacts: `engineering/evidence/ORACLE-S05/rank-postrebase.{json,md}`).

* total instruction gap to best-of-oracles: **1897** over 101 compared functions;
  **76 behind, 3 tied, 22 ahead**.
* 16 of the 76 "behind" rows compare functions whose **call counts differ**, so
  their gap measures an inlining decision and not code generation. They are
  listed by the tool and are excluded from the ranking below.
* Worst *comparable* gaps: `moving_stats` main **+86** (217 vs 131, gcc16.2),
  `zstd_count` main **+76**, `linux_rbtree` main **+64**, `strlen_bench` main
  **+62**, `lz4_compress` main **+59**, `expat_xml_scan` main **+58**,
  `sha256_transform` main **+53**, `loop_patterns` main **+52**,
  `conv_u8_3x3` main **+46**, `double_reduction` main **+43**,
  `ring_fifo` main **+43**, `chacha20_block:chacha20_core` **+38**.

## 6. Hard data II — Callgrind A/B against GCC 14.2

`scripts/callgrind_ab.py target/fastbuild/lccc /usr/bin/gcc "-O2 -march=x86-64-v3"`
on the 31-program fast corpus (artifact:
`engineering/evidence/ORACLE-S05/callgrind-o2-v3.tsv`; sample parity is checked
by the script before every measurement, so every ratio below is instruction-level
and output-identical).

**Read the geomean with suspicion**: `0.42783` is carried by three benchmarks
where LCCC evaluates the whole fixed-input computation at compile time
(`fib` 2.84G → 118,813; `ackermann` 1.04G → 118,416; `constant_recursion` 1.04G
→ 118,408). Those are legitimate for a benchmark with a constant input, and
useless as a performance claim. The honest reading is per benchmark:

| slower than GCC (ratio mine/ref) | faster than GCC |
|---|---|
| `ring_fifo` 2.735 | `chacha20_block` 0.412 |
| `histogram` 1.955 | `matmul` 0.670 |
| `ascii_case_fold` 1.859 | `arith_loop` 0.779 |
| `binary_search` 1.443 | `struct_copy` 0.869 |
| `linux_rbtree` 1.442 | `bitops` 0.887 |
| `loop_patterns` 1.320 | `sha256_transform` 0.952 |
| `switch_dispatch` 1.302 | `tce_sum` 0.976 |
| `zstd_count` 1.290 | `gzip_crc32` 1.000 |
| `expat_xml_scan` 1.169 | |
| `double_reduction` 1.151 | |
| `linux_find_bit` 1.112, `lz4_compress` 1.090, `zlib_ng_adler32` 1.075, | |
| `sieve` 1.070, `libm_round_family` 1.050, `glibc_memcmp` 1.051, `qsort` 1.042, `sqlite_varint` 1.032, `tls_seg_access` 1.021 | |

## 7. Hard data III — what the affine fold is worth, post-rebase

Fold on vs `CCC_NO_AFFINE_EXIT_FOLD=1`, same binary, Callgrind `Ir`, stdout and
the fixture's shape hashes identical in both arms:

| workload | fold on | fold off | ratio |
|---|---:|---:|---:|
| `tests/benchmark/programs/affine_countdown.c` | 95,730,850 | 114,826,850 | **1.1995×** |
| `tests/regression/affine_loop_fold.c` (all 37 shapes) | 1,261,435 | 1,359,847 | **1.078×** |

## 8. Gap classification — which of these are codegen, and which are not

This is the part of the baseline that changes what to do next, so it is stated
per item with the evidence that decided the classification.

* **Store-only maps with an IV-derived value — a real codegen gap, and the
  single biggest one.** `histogram.c`'s first loop
  (`bytes[i] = (u8)((i * 2654435761u) >> 24)`) and `ascii_case_fold.c`'s `init()`
  are maps whose only stream is the induction variable. GCC vectorizes both
  (`vpmulld`/`vpsrld`/`vpbroadcast` in the oracle asm); LCCC runs them scalar,
  because the map parser refuses a load-free tree outright
  (`vectorize.rs`, `"[VEC-MAP] BAIL: no loads"`, with the comment "a pure store
  of a loop invariant is not a map"). The comment is right about invariants and
  wrong about the IV: the tree needed here is *not* a broadcast but an **iota
  leaf**. Design sketch for the next session: add `MapExpr::Iota` (element index
  of each lane + the copy's displacement) and allow a load-free map when the
  tree contains an iota; for the two hot shapes the arithmetic is 32-bit and the
  store is `i8`, so this needs the wide-lane-compute/narrow-store path — the
  byte-lane parser that exists today (`parse_byte_map_expr`) demands
  byte-exactness, which `(i*C)>>24` deliberately is not. Budget: the two shapes
  are worth ~2× on `histogram` (1,668,349 → mine 3,262,250 Ir) and ~1.9× on
  `ascii_case_fold`.
* **`ring_fifo` 2.735× — not codegen.** GCC's asm for the whole `main` is 13
  instructions: it forwarded the store `ring[head & 1023] = seed` to the load
  `sum += ring[tail & 1023]` (the two indices are equal in every iteration of
  this benchmark by construction), scalar-replaced the array away, and collapsed
  the loop. LCCC keeps the array (59 instructions). Closing this needs memory
  GVN with relational index reasoning, not instruction selection; recorded as a
  different workstream so nobody chases it as a codegen item.
* **`binary_search` 1.443× and oracle +20 — isolated, next-session.**
  LCCC's inner-loop update is
  `movl %edx,%edi; movq %rdi,%rcx; ...; cmovll %ecx,%edi`; GCC emits
  `cmovgel %edx,%edi` — the same select with the condition inverted and the
  operands swapped, which retires the `mid-1` value *into* the phi's register
  instead of moving the phi aside first. Two instructions per iteration in a
  ten-instruction loop.
* **`loop_patterns` 1.320 (+52), `zstd_count` 1.290 (+76), `linux_rbtree` 1.442
  (+64), `switch_dispatch` 1.302, `double_reduction` 1.151 (+43),
  `expat_xml_scan` 1.169 (+58), `lz4_compress` 1.090 (+59)** — measured, ranked,
  untouched this round. They are the queue, in that order.

### 8.1 The pattern behind the gap table: it is vectorization, not selection

The oracle's own per-function statistics classify each row (its `vector` column
counts packed operations). Over the compared functions, **27 rows have LCCC
scalar (`vector == 0`/near-zero) while the best oracle vectorizes** — and the
top of the table is dominated by them: `moving_stats` (LCCC 217 insns / 0 vector
vs gcc16.2 131 / 61), `conv_u8_3x3` (137/0 vs 91/29), `expat_xml_scan` (152/0 vs
94/7), `lz4_compress` (351/4 vs 292/51), `linux_rbtree` (237/0 vs 173/3),
`struct_copy` (136/1 vs clang 76/47). Several of those rows are the
call-count-confounded ones listed in §5, so the count is an upper bound — but the
direction is not in doubt: where LCCC loses to GCC on this corpus, it usually
loses because GCC emitted SIMD and LCCC emitted a scalar loop, not because the
scalar loop is worse.

`zstd_count` is the counter-example that deserves its own look: LCCC emits
**more** instructions (167 vs 91) *and* uses vector ops (4 vs 0) — a partial
vectorization that grows the function. That is a cost-model question (remainder
path size against a small trip count), not a missing feature, and it belongs to
the same next-session pass as the iota work.

## 9. Environment notes (repeatable setup)

* No root in this sandbox: `apt` cannot install. Valgrind 3.24.0 was fetched as
  a Debian trixie `.deb` and extracted without root
  (`dpkg-deb -x`, then `VALGRIND_LIB=<prefix>/usr/libexec/valgrind`,
  `PATH=<prefix>/usr/bin:$PATH`); `scripts/callgrind_ab.py` finds it on PATH and
  otherwise fails with an error that looks like a measurement failure.
* 8 GiB swap active (`free` shows it; `swapon` is not installed in the image).
* Builds: `cargo build --profile fastbuild -j2` throughout; the fastbuild preset
  is what keeps a 2 GiB / 2-vCPU sandbox from OOMing during a full rebuild.

## 10. Next steps, in order

1. Iota store-maps (§8, first item) — measured at ~2× on `histogram` and
   `ascii_case_fold`; needs `MapExpr::Iota` + wide-lane/narrow-store emission,
   its own regression gate, and a kill switch.
2. `binary_search` cmov operand order (§8, third item) — small, pinned by the
   oracle gap and the Callgrind ratio.
3. Re-measure with `callgrind_ab.py` after each item, and re-run the oracle rank
   so the gap table stays a fact about the current tree.
4. Memory GVN / store forwarding (`ring_fifo`) as its own workstream.
