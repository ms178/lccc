# Register homing follow-up (2026-10-20, on main d8668ee24)

Scope: x86-64 register homing quality: loop-phi coalescing, callee-saved
save-set size, and the peephole fold that homing choices exposed. Every
number is Callgrind `Ir` at `-O2` on `tests/benchmark/programs/*.c` (VM, no
PMU), built with the fastbuild profile. "base" = d8668ee24 (main after
#882), built the same way; both binaries at equal-length paths.

## Findings

| ID | Where | Defect | Origin | Resolution |
|---|---|---|---|---|
| LIVE-FAT-LIVE-IN-BLOCK-END | `backend/liveness.rs` | A value live into a block but not out of it was extended to the block end (fat interval AND segment envelope), so it blocked homes it never occupies | pre-existing (upstream marked it deferred) | FIXED: end = last local event (def or use); the envelope is tightened too, so segments stay inside the fat interval. 3 unit tests, mutation-checked. `CCC_LIVE_IN_BLOCK_END=1` restores the old end (measured worse: adler32 371.2M, zstd 99.4M) |
| PHI-COALESCE-FAT-BLOCKERS | `regalloc.rs` GPR phi coalesce | Blockers were judged by fat intervals, so a co-tenant filled into a segment hole of the phi's register vetoed a legal coalesce (adler32 `v564<-v547`, `v791<-v544`) | missing (the FP path already used segments) | FIXED: segment-aware blocker filter, same contract as `fp_phi_seg_cov` |
| PHI-COALESCE-HOME-CONFLICT | `regalloc.rs` `PhiBlockerRehome` | A real blocker in the phi's register kept a latch copy in the loop | missing | FIXED: rehome blockers into an already-saved callee-saved register, or into a caller-saved register when the blocker is already caller-saved and meets no call point. Coalesce-group members never move. `CCC_NO_PHI_BLOCKER_REHOME` |
| RA-REHOME-CHAIN | `PhiBlockerRehome` occupancy | Blockers already moved were judged against the next blocker INCLUSIVELY. A two-address chain `v816[127,128] v817[128,129] v819[129,130]` (touching at death points, legal in the phi home) needed three targets, so the move failed | mine (this series) | FIXED: moved blockers count as users of the target, not as conflicts. All blockers co-reside in the phi home, so any subset keeps that relation in a common target. adler32 369.1M -> **356.6M** (base 383.8M). Test `phi_blocker_rehome_moves_a_touching_chain_together` (mutant caught) |
| RA-PHASE2C-SAVED-REUSE | `regalloc.rs` Phase 2c | Phase 2c overflow only took callee-saved registers no earlier phase used, so a function could save a fresh register while an already-saved one was free | pre-existing | FIXED, then REVISED 2026-10-22 (see the section at the end): the fresh tier runs first, unchanged; the leftovers fill already-saved registers' stretches hottest web first. `CCC_NO_PHASE2C_REUSE` |
| RA-CALLEE-STICKY | `live_range.rs` `find_free_register` | Round-robin rotation opened fresh callee-saved registers while touched ones were free. Each fresh register costs a push/pop pair per call; under register renaming, rotation's false-dependency benefit is moot | pre-existing | FIXED for Phase 1 (`prefer_touched`, rotation among touched registers first): pushes 333 -> 320, Ir neutral. REJECTED for the Phase 2c fresh tier. That scan cannot evict, so placement order decides which interval spills: sticky spilled zstd's hot v26 where rotation spilled the cold v158 (+4.0% zstd, +3.5% adler32). `CCC_NO_CALLEE_STICKY` |
| RA-SAVE-SET-COMPACT | `regalloc.rs` `compact_callee_saved_homes` | The phases' independent occupancy models leave a register whose holders all fit beside another saved register's (find_bit: v53 [34,35] in fresh %r14 while %rbx's v28 [15,16] was dead) | pre-existing | FIXED: a post-pass before the overlap repair folds a register's WHOLE holder set into another saved register. New co-tenants are judged by fat intervals, inclusively. A holder without an interval pins its register. Pushes 320 -> **297** (base 336); find_bit 181.40M -> 180.88M (base 181.13M). 4 tests, 2 mutants caught. `CCC_NO_SAVE_SET_COMPACT` |
| RA-XMM-CALL-SPANNING-SPILL | `regalloc.rs` `add_cold_call_xmm_saves` | x86-64 has no callee-saved XMM register, so a scalar FP value live across ANY call left the XMM pool and lived on the stack, even when every call it spans is a cold one (the errno slow path of `sqrt`) | missing | FIXED: values whose spanned calls are all cold (hole-aware segments only) keep an XMM home; each cold call gets a save/restore span, never over the call's own result register. A spanned non-cold call is an invariant break: `debug_assert!`, and release fails closed to a save. Unit test `cold_call_xmm_saves_skip_the_result_register`. `CCC_NO_COLD_CALL_SPLIT` keeps the old spill |
| PEEP-SETCC-FOLD-RECYCLED-REG | `flag_peepholes.rs` `fold_setcc_test_cmov` | Any line touching a boolean family between `setCC` and `test` aborted the fold, including a pure redefinition of a family whose boolean copy was already dead (`movl %r14d,%r11d; leal 32(%r9),%r14d; testl %r11d,%r11d; cmovnel`). Homing choices therefore decided whether `setbe; movzbl; test` survived: ascii_case_fold 2.21M -> 2.44M under Phase 2c sticky | pre-existing | FIXED: `retiring_families` retires a family on a full-width, non-RMW write that reads it neither explicitly nor implicitly; the tested register is protected until the test. Built only from the shared acceptance predicates. 3 tests, 2 mutants caught |

## Upstream design verdicts

- **Phase 2c "fresh callee-saved only": DISAGREE.** An already-saved
  register is free capacity. Fixed by tier 1 (seeded, valve preserved).
- **Round-robin rotation for callee-saved pools: DISAGREE for Phase 1,
  AGREE (for now) in the Phase 2c fresh tier.** The tier lacks weight-aware
  eviction, so rotation is the safer placement there. The real fix is
  eviction by spill weight (spill the colder of the current interval and its
  overlapping holders), tracked as RA-PHASE2C-WEIGHTED-EVICTION.
- **FP phi segment-aware blocker filter: AGREE**, extended to GPRs.

## Accepted side effect

- linux_rbtree +16,384 Ir with compaction: `xor %ebp,%ebp` is one byte shorter
  than `xor %r13d,%r13d`, so the 10-byte alignment pad before the loop became
  11 bytes, which GAS emits as two NOPs. They execute once per loop entry; the
  loop body is identical apart from register names.

## Current Ir (this tree vs base, -O2)

`scripts/callgrind_ab.py`, 31 programs, geomean Ir this/base = **0.98429**.
Every program not listed is within 0.01% (most are bit-identical).

| bench | this tree | base | delta |
|---|---|---|---|
| linux_rbtree | 20,770,198 | 22,927,453 | -9.41% |
| switch_dispatch | 971,996,458 | 1,046,995,773 | -7.16% |
| zlib_ng_adler32 | 356,567,046 | 383,776,023 | -7.09% |
| zstd_count | 97,246,277 | 102,620,233 | -5.24% |
| histogram | 1,331,707 | 1,397,249 | -4.69% |
| qsort | 445,524,843 | 464,198,750 | -4.02% |
| ascii_case_fold | 2,165,495 | 2,214,648 | -2.22% |
| sqlite_varint | 342,715,224 | 350,026,138 | -2.09% |
| bitops | 2,750,122,560 | 2,800,122,560 | -1.79% |
| binary_search | 573,561 | 581,754 | -1.41% |
| arith_loop | 1,010,121,313 | 1,020,121,313 | -0.98% |
| expat_xml_scan | 669,406,411 | 673,436,910 | -0.60% |
| lz4_compress | 24,278,164 | 24,423,539 | -0.60% |
| linux_find_bit | 180,845,417 | 181,105,516 | -0.14% |
| tls_seg_access | 131,121,645 | 131,321,645 | -0.15% |
| glibc_memcmp | 88,161,836 | 88,212,024 | -0.06% |
| sha256_transform | 3,359,762,305 | 3,360,810,884 | -0.03% |
| gzip_crc32 | 545,382,426 | 545,382,428 | -2 Ir |
| spectral_norm | 2,240,978,287 | 2,240,950,600 | +27,687 Ir |

- spectral_norm: the program's own Ir is identical (2,240,828,560 on both
  sides). The difference is ld.so loading libm: this tree honours
  `-fmath-errno` with a never-taken `call sqrt` slow path for negative
  inputs (MATH-ERRNO-SQRT, as GCC 16.2 emits it), so the executable NEEDs
  libm. Base dropped `errno` for `sqrt`. One-time startup cost, a
  conformance fix.
- gzip_crc32: neutral now. The earlier +4,086 Ir was DT_NEEDED order
  (libm before libc, exactly GNU ld's order for the same link line). See
  LD-NEEDED-ORDER in FINDINGS.md.

Prologue pushes over all benchmark programs at -O2: base 336, this tree 297.

## 2026-10-22: RA-PHASE2C-SAVED-REUSE revised (zstd decoder regression)

A Callgrind A/B of the zstd 1.5.7 CLI (`scripts/zstd_eval/`, 1.5 MiB input)
against main 32769db4e showed every decompression case flat or worse while
compression improved. Per function, `ZSTD_decompressSequences_bmi2` (81% of
decode Ir) was +3.0% at every level. Instruction-level profiles put the
loss in the per-sequence block, +8 instructions over 113,020 iterations.
The three `nbBits` bytes (three reads each) were spilled and reloaded, and
two table-entry addresses were materialized into a stack slot. Switch
bisection on the one TU found `CCC_NO_PHASE2C_REUSE=1` restored the
function exactly (22,856,263 Ir vs 23,534,434).

Cause: the tier was a start-ordered linear scan over the free stretches of
already-saved registers, run before the fresh scan. Neither scan can evict
across the other, so a stretch went to whichever overflow interval started
first. Here that was a single-use table base and a copy, ahead of the hot
bytes. This is the RA-CALLEE-STICKY failure in another form.

Revision:
1. The fresh scan runs first, unchanged, so the tier only adds homes.
2. The tier is a no-eviction fill ranked as Phase 2f ranks: whole-web
   loop-weighted uses, then the shorter envelope, then the value id
   (`fill_saved_callee_stretches`).
3. Interference uses fat group-merged envelopes with 2f's half-open
   hand-off.

Alternatives measured and rejected:
- Delete the tier (2f covers saved-register holes): 2f skips Copy webs, so
  strlen_bench's outer loop counter round-trips the stack (+675K Ir).
- 2f's hole-aware segments instead of fat envelopes: geomean 0.995 vs
  0.989 on zstd (c-19 +1.4%, c-1 +0.9% vs fat).
- Keep the scan, fresh tier first: no change on the decoder (its fresh pool
  is empty).

zstd Callgrind Ir, final code (byte-identical asm to the measured binary):

| case | main 32769db4e | start-ordered tier | revised | vs main | GCC 16.2 -O2 | revised/GCC |
|---|---:|---:|---:|---:|---:|---:|
| c-1 | 59,501,994 | 54,215,530 | 54,355,307 | -8.65% | 41,522,827 | 1.309 |
| d-1 | 26,963,025 | 25,873,172 | 25,301,975 | -6.16% | 17,665,466 | 1.432 |
| c-3 | 71,539,881 | 66,210,102 | 66,356,383 | -7.25% | 47,256,549 | 1.404 |
| d-3 | 28,617,053 | 28,580,661 | 27,895,679 | -2.52% | 18,977,297 | 1.470 |
| c-19 | 6,733,778,043 | 5,021,838,583 | 5,022,863,239 | -25.41% | 3,385,452,266 | 1.484 |
| d-19 | 24,916,767 | 25,052,314 | 24,468,353 | -1.80% | 16,754,387 | 1.460 |

Every decompression round trip was byte-exact. The zstd validation
(FORCE_INLINE census 0, 24/24 cross-decoding round trips, fuzzer -s7 for
150 s) passed. The same validation fails on main: 7 FORCE_INLINE templates
are outlined.

`scripts/callgrind_ab.py --opt=-O2` (31 bounded benchmarks, revised vs
start-ordered): Ir identical on all 31, including linux_find_bit
(181,365,500), whose fresh-%r15 case the tier was built for. Save-set
compaction folds it.

What the revision costs, measured:
- strlen_bench: +0.06% Ir. Five of the six extra instructions per outer
  iteration are alignment NOPs on entry to the aligned inner loop (the code
  moved by 16 bytes); the sixth is a `jmp` from a layout change. Every loop
  value is in a register in both builds.
- zstd compression: +0.02% to +0.26% vs the start-ordered tier, from
  `HIST_count_parallel_wksp` (+2.6%) and `ZSTD_buildSequencesStatistics`
  (+3.8%). In HIST the fill gives %rbx to four webs that the static
  loop-depth weights rank above the loop pointer `ip+4`, which the
  start-ordered scan had homed. The weights carry no trip counts, the same
  limit Phase 2f has. These functions are each under 5% of compression Ir;
  the decoder is 81% of decompression Ir.

Unit tests:
- `saved_callee_fill_places_the_hottest_web_first`;
- `saved_callee_fill_breaks_ties_by_envelope_then_id_and_tries_each_register`;
- `saved_callee_fill_judges_the_fat_envelope`.

Negative control: the first two fail when the ranking is mutated to start
order.
