# PR #696 follow-up: audit response, gate restoration, and the cross-vendor oracle campaign

Session date: 2026-09-30 (S04 follow-up on PR #696; base `f9bef39b`, the
S08 deliverable re-based on latest main).

## 1. The Review AI audit — adjudication

The audit's verdict "core engineering right, testing discipline regressed"
was adjudicated finding-by-finding against the code, not the prose. Every
factual claim was independently verified; the two findings that looked like
bugs and weren't were also re-derived (both correct, as the audit said).

| Finding | Verdict | Evidence |
|---|---|---|
| F1 (HIGH): far-local `data16` grow path lost byte-level gating | **AGREE** | `encdiff.py:786` returns `ORACLE-INVALID` at `:794` before any comparison; encoder tests pin only external forms; the `.space 200` rows were removed from `prefix-words.casefile` with nothing replacing them. The near-local rows gate the SHRINK path only. |
| F2 (MED): nocfi gate probes with lccc, so an lccc regression becomes a silent SKIP | **AGREE** | `i386_run_ok()` compiled+ran its probe with `"$CCC"`; the skip decision depended on the artifact under test. The worst version of the mistake. |
| F3 (MED): 136/174 changed lines in one file are tab→space churn | **AGREE** | `git diff -w` was 40 lines vs 176 full. An editor reformat on save. |
| F4 (MED): four probe implementations in the PR that adds the shared one | **AGREE** | `i386_exec.sh`'s own docstring says "Source, don't execute" — three of five call sites carried private copies. |
| F5 (LOW/MED): unconditional `PASS: … (x86-64 + i386)` when i386 was skipped | **AGREE** | Phantom coverage claims in CI-greppable lines. |
| F6 (LOW): execution-independent laws correctly placed | **AGREE** (confirmed) | `copy_alias_law` runs before the exec-gated block; PLT-routing before the run comparison. |
| F7 (LOW): `_MOV_IMM` regex narrowed for no stated reason | **AGREE** | `%(\w+)`→`%([a-z0-9]+)` in 47580b76; no behavioral difference for any real spelling; reverted (T6). |
| F8 (NIT): probe not memoized | **AGREE** | comdat probes per mode-loop iteration. |

The six technical decisions the audit endorsed were re-verified independently
(prefix66 length arithmetic at all six sites — then byte-pinned by test rather
than trusted; `& 0x7` EVEX map fix; `@PLT` accept-set parity; the precedence
claim at `elf_writer.rs:55`; no double-addend). **One instruction was
disagreed with in part:** T1.4 (asmdiff `local expectation` support as the
preferred mechanism) — a writer-level unit test is strictly stronger for the
relaxation engine (no oracle indirection, runs in-process), and a new
corpus-side expectation mechanism would itself need tests. T1.1-3 were
implemented as specified; T1.4 was not.

## 2. The seven fixes (T1–T7), all landed on the PR branch

1. `aa3714f3` (T1) — `data16_far_local_grow_path_is_byte_pinned` (jmp/jcc/call,
   150-byte gaps, full-vector assert with derived displacements 463/306/150,
   zero leftover relocations), `data16_near_local_relaxes_to_gas_short_rows`
   (`66 eb 05`/`66 75 01`), `data16_far_backward_local_settles_long`
   (`66 0f 85`, disp −137, the short-form's-end backward arm). All passed
   first-run, which independently confirms the audit's hand-trace.
2. `811d102d` (T2) — nocfi gate probes host capability via `i386_exec.sh`
   + `$GCC`; the lccc -m32 leg now FAILS on an lccc regression.
3. `cc8d3698` (T3) — notype-code re-landed on the pristine base by exact
   byte-replacement script (the interactive editor expands heredoc tabs —
   that is HOW the churn happened); 26+/7− (was 174), `git diff -w` ≈ full diff.
4. `070d4ebb` (T4) — `i386_exec.sh`: one 3-level memoized `i386_capability`
   (`run|link|none`); `i386_exec_ok` a thin `==run` wrapper; new
   `i386_link_ok`; eh_frame/comdat stop hand-rolling their second link probe;
   copy_alias's private third copy deleted (probe compiler now `$GCC`).
5. `eae56fa1` (T5) — three-state PASS lines claiming exactly what ran +
   machine-readable `SKIP-RUN:` markers (run_regression.py's vocabulary).
6. `b63f35c0` (T6) — `_MOV_IMM` regex restored to the audited baseline.
7. `c0f19f79` (T7) — parens on the `expected_len` grouping (readability;
   behavior pinned unchanged by the casefile).

## 3. The oracle campaign (the user directive: read them all, use them all)

Read + used: `docs/GODBOLT_ORACLE.md`, `tools/oracle/godbolt_oracle.py`(+selftest),
`scripts/godbolt.py`, `godbolt_cache.py`, `codegen_oracle.py` (--rank),
`encdiff.py`, `asmdiff.py`, `test_encdiff.py`, `callgrind_ab.py` (methodology),
`run_regression_suite.sh` (oracle plumbing), `check_benchmark_outputs.sh`.

### 3a. Rank survey (codegen, compiler-side targeting)

`codegen_oracle.py --rank tests/benchmark/programs/*.c` with the s19 baseline
→ `engineering/evidence/godbolt/pr696-rank/` (rank.md + rank.json). Totals:
lccc 7580 insns / 9 bests; gcc16.2 7823/32; clang 10160/3; icx 11626/5;
icc 12988/2 — **lccc smallest overall**, but gcc owns 32/51 per-source bests.
96 functions compared: 75 behind, 3 tied, 18 ahead; total gap 1654.
Worst gaps: nbody 188 (127 loads/31 spills), moving_stats 90, i686_alu_chains 86,
glibc_strstr 70, linux_rbtree 64 (was 492 at s19 — the merged work collapsed it),
matmul 63, strlen_bench 63, struct_copy 60. Three remote compile failures
(vector_remainder, zlib_ng_adler32, zstd_count — all four oracles; reported,
not passed silently). **Next-session targeting:** the dominant class is
constant-trip-count loop unrolling + index constant-folding (nbody/matmul/
ring_fifo/int_alu — the `body_contains_persisting_inner_loop` refusal is the
single identified blocker); second is moving_stats-class RA/remat.

### 3b. Remote-oracle encdiff sweep (assembler, this session's domain)

Whole corpus vs gcc16.2/clang23.1/icx/icc (chunked per casefile — the
single-shot run cannot complete inside foreground windows; the driver +
per-chunk JSONs live in this session's records; chunks whose remotes were
rate-limited were re-queued; three casefiles remain partially covered,
honestly recorded). Findings, in discovery order:

1. **TEST commutativity** (byteregs, 24 rows): ICC emits the commuted modrm
   form (`40 84 c5` for `test %bpl,%al`). TEST's sources are read-only and
   order-independent → sorted-operand canonicalisation (the `_COMMUTATIVE_VEX`
   pattern). No other GPR instruction qualifies (CMP/UCOMISx order is
   observable; every two-operand ALU op writes a destination).
2. **XCHG commutativity** (misc, 7 rows): clang/ICC/ICX commute
   (`40 86 c5` vs `40 86 e8`); the exchange is its own inverse → sorted.
3. **Selector moves/stores, 64-bit** (apx, 3 rows): clang/icx take the
   `REX.W + 8C/8E` and REX.W SLDT/STR rows; GAS/GCC/ICC/lccc take the
   32-bit rows (SDM lists both). In 64-bit mode a 32-bit GPR write
   zero-fills the upper half → identical architectural state; the no-W row
   is also 1B shorter on the legacy path. Mode-aware canonicalisation
   (32-bit view unifies with 64-bit view; 16-bit views and 32-bit mode
   stay distinct — a 16-bit write preserves the upper bits).
4. **The index-only scale-1 fold** (modrm, 88 rows → the big one): lccc
   emitted SIB+disp32 (8B) where ICC folds the index into the base slot
   (3-5B). Adopted, then extended one step further than ICC's own win set
   (see 3c).
5. **The ICC `movq $u32-bit31` miscompile** (misc, 6 rows): ICC's -O0
   encoder emits REX.W imm32 which the hardware sign-extends — a different
   program, objdump-verified. Partitioned like the truncated data16 rows;
   the same bytes from lccc would be reported immediately.

### 3c. The index fold (encoder change, oracle-distilled)

`fold_index_into_base` in `encoder/core.rs` — a pure operand rewrite called
by BOTH `emit_rex_rm` and `encode_modrm_mem` (one decision; the X→B extension
bit moves with the register). Every GPR folds (flat 64-bit segmentation; the
modrm emitter already encodes every based form optimally — byte-probed:
`mov (%r13)` → `49 8b 4d 00`, `mov (%r12)` → `49 8b 0c 24`, EGPR → REX2 with
the bit moved). Guarded to scale==1, no base, INTEGER displacement (any
width: int8 → disp8 base form; wider → mod=10, ICC's exact 7B forms — 28 more
verified rows). Symbol displacements keep the GAS-parity SIB form (folding
them would change the relocation class — unevidenced). VSIB vector indices
never fold (`gp_id` guard).

Corpus migration: 156 rows (121 wave-1 + wave-2 remainder, merged after a
script clobbered the .insn — restored from git) →
`tests/encdiff-corpus/index-fold-64.insn`. Final oracle state on that corpus:
**148 shorter + 8 tie vs clang/GAS/GCC/ICX (−465B), an EXACT tie with ICC
(767B = 767B) on every row** — lccc matches the single best oracle and beats
the other four by 1.6× on this family. Byte-pins: `index_fold_tests` (three
tests, every register class, both int8 boundaries, the 0x67 interplay, every
guard edge, VSIB) — all probed, none derived.

## 4. Measurements (hard data, no guesswork)

* **Callgrind 3.24 A/B** (fold, 400k-instruction stress: 50% foldable
  operands): old 18,521,918,028 Ir, new 18,883,734,414 Ir → **+1.95%** on
  that mix (≈1.8k Ir per firing — two String clones; absent from
  compiler-generated code). The fold fires only on hand-written index-only
  operands; the benchmark + regression outputs are unchanged where it does
  not fire, and the bytes shrink where it does.
* **encdiff whole corpus, offline** (x86-64): 12418 insns, BEATS=124
  ok=12163(+…) ORACLE-INVALID=10 both-reject=19, **zero bad verdicts**, exit 0
  — byte-identical to the pre-session S03 baseline (the T6 revert restored
  the exact audited tool state). i686: 833, BEATS=6 ok=817
  DECLINED-DATA16=7 both-reject=3, zero bad, exit 0.
* **Regression suite** (sysroot32 restored rootless:
  libc6-dev-i386 + libc6-i386 + lib32gcc-14-dev + lib32gcc-s1 under
  `~/.cache/lccc-sysroot32`, with `/lib`, `/lib32` symlinks): 782 PASS /
  0 FAIL / 35 SKIP (seccomp ia32 runs — hosted CI authoritative), 0 AB-diff
  failures.
* **Benchmark output oracle**: 204 PASS / 0 FAIL.
* **asm-diff**: 1418 + 672, 0 failed, both modes (GAS 2.47.20260726).
* **cargo test --all-targets**: 3896 (3889+7 ignored) 0 failed;
  clippy `-D warnings` clean; rustfmt clean. encdiff mocked suite: 41/41.

## 5. Environment (reproduce after wipe)

Same as S03 plus: `LCCC_SYSROOT` AND `LCCC_I686_SYSROOT` =
`~/.cache/lccc-sysroot32` (the regression suite reads the latter);
`lib32gcc-s1` extracted into the sysroot (`libgcc_s.so.1` runtime);
`/lib → usr/lib`, `/lib32 → usr/lib32` symlinks (linker-script absolute
paths); valgrind 3.24.0 from the Debian deb into `/tmp/valgrind-local/vx`
(same version as the from-source build it replaces). The remote sweep
driver + per-chunk JSONs: `/tmp/encdiff-remote/` (session-local; the
`.godbolt-cache` in the repo persists across wipes).

## 6. Next session

1. Codegen: the rank-survey targets above (constant-trip unroll cascade is
   the single biggest lever — nbody/matmul/ring_fifo/int_alu).
2. The three partially-covered remote chunks (macro_vararg,
   p2align-maxskip, prefix) + the two never-reached (the sweep driver
   re-queues them; rate limits, not tooling).
3. EVEX no-base operands (encode_evex_mem path) stay unfolded —
   unevidenced this session; re-sweep after any EVEX corpus grows.
