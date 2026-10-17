# Work-item catalog — open items only (rewritten 2026-10-17)

Ranked work for **generated-code performance** vs GCC 16.2 / Clang 23.1 /
ICC 2021.10 / ICX 2026.2. This file carries **only open items**: every
landed/closed/superseded row, the architectural-flaw essay, the historical
scoreboard and the landed register live verbatim in
[`../archive/BACKLOG-2026-10-16.md`](../archive/BACKLOG-2026-10-16.md);
measured negatives and session history live in [`../DECISIONS.md`](../DECISIONS.md).

This rewrite also absorbs the 2026-10-16 external backlog audit
("MB-01…MB-42", baselined on `e8ae10a`). Every imported claim was re-probed
on the current tree (`c39b6e8c` + PR #844) before admission; §3 records the
claims that were **rejected or found already fixed** so they are not
re-imported later.

Evidence codes: **M** measured LCCC vs GCC on kernels · **G** Compiler
Explorer / local oracle (`scripts/godbolt.py`, `scripts/codegen_oracle.py`,
`-O2 -march=x86-64-v3`) · **C** tree fact · **S** screening suite ·
**V** verified by direct probe on 2026-10-17.

ROI rank: **P0** next session (testable) · **P1** this month · **P2** after
P0/P1 · **P3** research.

How to gather data (mandatory):

```text
1. Reproducer C (≤80 lines) in tests/benchmark/programs/ or engineering/evidence/godbolt/
2. scripts/godbolt.py compare FILE --local target/fastbuild/lccc \
     --oracles gcc16.2,clang,icc,icx --function NAME \
     --flags '-O2 -march=x86-64-v3' --artifact-dir results/NAME
3. Count: insns, (%rsp)/(%rbp), GOTPCREL, vfmadd, btq, ymm, push callee-saved
4. If runtime: tests/benchmark/run_benchmarks.py --only KERNEL --reps 15;
   compile-time claims use Callgrind Ir (deterministic under VM load), never wall time
5. Kill-switch the change; gzip longest_match stack-mem must not rise
6. PMU only on 14700KF after (2)+(4) agree
```

---

## 0. S05 measured snapshot (2026-10-17, rebased tree)

- **Compile-time pathology (V, Callgrind on `expat_siphash24.c`):**
  `-O0` 2.68 G Ir · `-O1` **6.10 G Ir** · `-O2` 1.64 G Ir. `-O1` costs
  3.7× `-O2`. Inclusive: `peephole_optimize_inner` = **90.9 %** of the whole
  `-O1` compile; `peephole_for_metric` (the peephole invoked as a *size
  metric* inside `generate_module`) = **44.8 %** alone; `memcmp` +
  `<[u8]>::starts_with` (text-based opcode matching) = **24.7 %**;
  `classify_implicit_operands` 3.7 %; `FileLiveness` ≈ 4 %. See MS-20.
- **Corpus:** 377 executed Clang-C cases = 248 PASS / 128 FAIL
  (95 accepts-invalid, 33 rejects-valid) / 1 ERROR (S04 outcome report,
  regenerate via `tests/corpus/`). See COR-01.
- **Runtime scoreboard (S04, pre-rebase, re-measure before use):**
  bench_kernels geomean 0.747× GCC; worst `namechars` 0.30×,
  `seg_gs_fill` 0.41×, `seg_gs_sum` 0.49×, `varint` 0.76×, `lz4` 0.88×;
  best `seg_gs_copy` 1.69×, `strlen_scan` 1.18×.

## 1. Open items carried from the previous catalog

Rows below are the still-open subset of the 2026-10-16 catalog, unchanged in
substance; consult the archive for full Accept/Do-not text where elided.

### 1.0 Peephole ledger — PF

| ID | P | Item | Evidence |
|----|---|------|----------|
| PF-06 | P1 | isort: secondary-IV strength reduction + int-index widening pairs | **G** isort 43 vs Clang 23 |
| PF-16 | P2 | `movq $imm32,%reg` → `movl`; redundant `cltq` after `lzcnt` | **G** ffs1 17 vs 11 |
| PF-17 | P0 | Loop rotation hardening → default-enable (the systemic ~1 branch/iter gap); do not default-enable before the 15 refused shapes are root-caused | **M** suite-wide |

### 1.1 Register allocation — RA

| ID | P | Item | Evidence |
|----|---|------|----------|
| RA-01 | P0 | Remat file-scope `window`/`prev`/`strstart` as `sym(%rip)` / `sym(%rip,idx,1)`, not GOT+stack (gates/fixtures landed in PR #844; the remat kernel itself is still dormant) | **M** 118 stk; **G** gcc `window(%r9,%rcx)` |
| RA-01b | P0 | Marching-pointer recurrences (IVSR Ptr phis) slot-homed: 151/159 of nbody's stack refs are GPR address traffic; extend GlobalAddr remat through Copy chains | **M/G** nbody 159 → <20 target |
| RA-02 | P0 | Keep match IVs (`scan`,`best`,`chain`) in GPR; gzip `longest_match` stack-mem <20 | **M** 248 B frame |
| RA-03 | P0 | Adler DO8: refuse the reassoc closed form above GPR budget or emit GCC's prefix-sum tree | **M** 91 vs 63 insns |
| RA-06 | P1 | Pressure splitter: **re-measure the residual gap's shape first** (two prior sessions each chased the wrong cause); cross-block SSA repair is the prerequisite | **M** DECISIONS.md |
| RA-34 | P1 | `rrmov` (317/272) and `push` (47/38) are the only buckets still trailing GCC; attack via RA-07 + RA-08 | **M** census totals |
| RA-07 | P1 | Call-site caller-saved save vs always r12 | **C** |
| RA-08 | P1 | Affinity coalescing of overlapping copies (`build_coalesce_groups` disjoint-only) | **C** |
| RA-11 | P1 | One `RangeMetadata` per function (compile time) | **C** |
| RA-15 | P1 | Param group priority remaining holes | **C** adler/memcmp history |
| RA-17 | P1 | Folded GEP **base** liveness x86 remaining | **C** gz_reset NULL-store |
| RA-19 | P1 | PhysReg map table in one file | **C** r10/r11 swap history |
| RA-29 | P1 | Feed rematerializability into `spill_cost_at` (`RematKind`); direct input to RA-01/01b | **C** |
| RA-32 | P1 | FP copy-web coalescing: prove the merged leader's interval cannot hide another value in a member's live hole, or make it conservative | **C** |
| RA-09 | P2 | Residual Briggs on spilled | **C** |
| RA-10 | P2 | GPR spill to XMM | ICX/ICC |
| RA-14 | P2 | Stronger `split_ranges` (phis, multi-exit) | **C** |
| RA-16 | P2 | Recalibrate PGO RA weights after RA-01 | **C** |
| RA-20 | P2 | i686 eax/ecx model → clobber oracle from emit | **C** |
| RA-30 | P2 | `calculate_spill_weight` denominator → `occupied_points()`; isolated A/B | **C** |
| RA-31 | P2 | Per-use cost classes (memory-foldable use ≪ reload use) | **C** |
| RA-22 | P3 | PBQP irregular (AArch64) — only after x64 codecs win | — |

### 1.2 ISel / peephole / MachInst — IS

| ID | P | Item | Evidence |
|----|---|------|----------|
| IS-02b | P1 | Field-store/arg halves of the SSE-class ABI (returns are done) | **M** mk 24→11 |
| IS-05 | P1 | Strict non-contracted `vmulps/pd` still round-trips one `VecLoad` through a slot | **M/G** |
| IS-07 | P1 | `cmov` limit idiom | **G** gcc `cmovc` |
| IS-08 | P1 | Load-cast fold remaining | **C** 33 % gzip-9 was movzbl |
| IS-09 | P1 | Cmp-replay only from slots (keep) | **C** sqlite SEGV |
| IS-10 | P1 | Flag fusion Copy/Cast chain — extend carefully, multi-use flags forbidden | **C** expat SEGV |
| IS-11 | P1 | C `__ffs` if-tree → GCC `andn`+`cmov` chain (not tzcnt) | **G** 1.51× gap |
| IS-13 | P1 | ALU+mem for spilled | ICX lookalike |
| IS-16 | P2 | Match inner 258 B with `pcmpeqb`; needs CRC/adler oracles first | spike |

### 1.3 Optimizer / alias / SROA / vectorize — OP, VEC, CG

| ID | P | Item | Evidence |
|----|---|------|----------|
| OP-05b | P0 | Multi-store scatter vectorization (nbody `advance`) + computed-invariant dot (spectral `A(i,j)`) — the remaining PERF-41 gap | **M** spectral_norm |
| OP-02 | BLOCKED | Dominator-aware SROA copy-out stays off (miscompiles `structs_bitfields`); needs dominance/ABI proof + full fuzz | **M** |
| OP-03 | P1 | SROA cross-block memcpy | **C** |
| OP-07 | P1 | Runtime alias versioning | **C** |
| OP-08 | P1 | Nested IVSR with dependence | **C** |
| OP-10 | P1 | Unroll vs RA pressure | **C** |
| OP-14 | P1 | If-convert >8 inst / more diamonds | **C** |
| OP-16 | P1 | `bit_idioms.rs` coverage (popcnt, andn, ffs) | **G** |
| OP-19 | P1 | `load_forward`/`store_load_forward` vs GVN overlap (compile time) | **C** |
| OP-20 | P1 | `loop_memory_promote` more consumers | **C** |
| OP-22 | P1 | `redundant_loads` follow-ups | **C** |
| OP-25 | P1 | Inline `name_cont` (watch the 189-inst varint spike) | **G** |
| OP-29 | P1 | Pass-order experiment with gzip/adler/expat oracles | **C** |
| OP-32 | P1 | restrict-param `forms_disjoint` wiring into GVN | **M** |
| OP-36 | P1 | Extend expression-boundary contraction tag coverage (`FpContract::OnExpr`) | **C** |
| VEC-12r | P1 | Re-admit loop-carried secondary affine recurrences by **epilogue reconstruction** (`acc0 + n*step`), never by relaxing the fail-closed reject | **C** |
| CG-07r | P1 | Audit remaining emitter `is_zero()` sites for the -0.0/C-truthiness confusion | **C** |
| OP-09/11/13b/17/18/21/23/24/27/33 | P2 | as in archive | — |
| CG-08 | P2 | Slot-coalescing headroom is in DETECTION (41/41 resolved); widen candidates | **M** |
| OP-28 | P3 | Full SCEV — do not rewrite IVSR+alias+licm at once | — |

### 1.4 Frontend / IR — FE

| ID | P | Item | Evidence |
|----|---|------|----------|
| FE-02 | P1 | TBAA / `restrict` → IR attrs; per-pointer-level qualifiers | **C** noalias grep empty |
| FE-10 | P1 | Alignment/range on IR for RA | **C** |
| FE-16 | P1 | Volatile through peephole phase 2 (MMIO casts, struct fields, `int * volatile`) | **C** |
| FE-23 | P1 | Address-space qualifiers on parameters / multi-level | **C** |
| FE-25 | P1 | Real `-march=native` for Raptor Lake (AVX2/BMI2/F16C/GFNI/VNNI; host-queried cost model) | **C** |
| FE-30r | P1 | Decide the DEFAULT dialect move to `gnu23` (GCC 15/Clang 19 parity) — semantics change, needs its own torture matrix | **C** |
| FE-03/04/05/06/07/08/09/11/12/14/15/17/18/20 | P2 | as in archive | **C** |
| FE-19 | P2 | `__builtin_cpu_supports`: runtime CPUID detection on non-x86-hosted targets (allowlist landed; x86 answers are already a runtime `__cpu_model`-equivalent probe — see §3 MB-03) | **C** |
| FE-24 | P2 | `#error` message verbatim | **M** |
| FE-13 | P3 | Use-def shared context | **S** |

### 1.5 ABI / aggregates / stack — AB

| ID | P | Item | Evidence |
|----|---|------|----------|
| AB-01 | P0 | SysV SSE-class args in XMM not rax shuttle (returns landed) | **S** struct_copy 1.35× |
| AB-02 | P0 | sret + one wide copy | same |
| AB-03/04/05/11/15 | P1 | as in archive | **C** |
| AB-08/10/12 | P2 | as in archive | **C** |

### 1.6 PGO / IPO / layout — PG

| ID | P | Item | Evidence |
|----|---|------|----------|
| PG-04 | P1 | RA use block counts after RA-01 | **C** |
| PG-06 | P1 | Keep conservative layout (expat 131→248 guard) | **M** |
| PG-08 | P1 | Force-inline budget vs frame | **C** |
| PG-11 | P1 | Keep the flat-profile gate (`has_spread`) | **M** adler −17 % pre-fix |
| PG-12 | P1 | Don't promote 95 % single-target | **M** +28 % if promoted |
| PG-01/02/05/07/09/10 | P2 | as in archive (PG-07 = LTO; also covers the imported "whole-program/-flto" ask) | — |

### 1.7 Linker / assembler / multi-arch — LK

| ID | P | Item | Evidence |
|----|---|------|----------|
| LK-19 | P0 | IFUNC end-to-end (`R_X86_64_IRELATIVE` data words, ld.so semantics); afterwards `__attribute__((target_clones))` rides the same mechanism | **C** |
| LK-24 | P0 | Externally-compiled PIE segfaults at startup against lccc-glibc | **M** SIGSEGV 139 |
| LK-28 | P0 | objtool interop (`.discard.annotate_insn`, `find_insn`), then real `vmlinux` link | **M** |
| LK-14 | P1 | `encdiff.py`/`insndiff.py` in the RA ISel loop | **C** |
| LK-20 | P1 | `-r` spurious blank `UND NOTYPE LOCAL` symtab entry | **M** |
| LK-21 | P1 | `-l` resolution in the `-r` path (`-lgcc` dropped) | **C** |
| LK-23 | P1 | ldconfig `feof_unlocked` undefined | **M** |
| LK-29 | P1 | Statement-expression + inline-asm type inference (blocks `net/*`) | **M** |
| LK-01/02/03/05/06/07/08/09/10/11/12/13/15/16/22/25/27/30 | P2 | as in archive | — |
| LK-18 | P3 | QEMU build | **C** |

### 1.8 Measurement / process / compile-time — MS

| ID | P | Item | Evidence |
|----|---|------|----------|
| MS-01 | P0 | CE-oracle delta half of the codegen gate | **C** |
| MS-11 | P0 | glibc `make check` triage harness | **C** |
| MS-03 | P1 | CI correctness+regression+fuzz smoke beyond lib tests | **C** |
| MS-04 | P1 | OnceLock env knobs break parameterized tests | **C** |
| MS-10 | P1 | `CCC_VALIDATE_SSA` in CI for the regression corpus | **C** |
| MS-12 | P1 | Audit remaining `phys_reg_name_32` sites for XMM-homed operands | **C** |
| MS-13/14 | P2 | as in archive | — |

## 2. New items admitted from the 2026-10-16 external audit + S05 probes

| ID | P | Item | Evidence | Accept | Do not |
|----|---|------|----------|--------|--------|
| MS-20 | P1 | **Compile-time: the peephole is 91 % of a `-O1` compile.** Three measured sub-causes, attack in order: (a) `peephole_for_metric` re-runs the whole text peephole as a size oracle inside `generate_module` — 44.8 % of all Ir; memoize per function body hash or compute the metric from a cheaper proxy; (b) opcode dispatch is `memcmp`/`starts_with` on instruction text — 24.7 %; pre-parse each line once into an opcode id + operand slice instead of re-scanning strings per pass; (c) `FileLiveness`/`classify_implicit_operands` rescans — ~8 %. Subsumes old MS-05. 159 `trim().to_string()` sites are the allocation tail, not the head | **V** Callgrind 2026-10-17: O1 6.10 G vs O2 1.64 G Ir; inclusive 90.9 %/44.8 %/24.7 % | `-O1` full-compile Ir ≤ 2× `-O2` on `expat_siphash24.c`; byte-identical `.s` on the whole benchmark corpus with the cache on | "optimize" by skipping peephole passes; any fix that changes emitted text |
| COR-01 | P0 | **Triage the 128 corpus FAILs** (95 accepts-invalid, 33 rejects-valid, 1 ERROR); includes the verified `&(0, inc)` accepts-invalid. Classify → fix the top recurring sema gaps; rejects-valid are potential user-facing blockers | **M** S04 corpus report | fail count strictly down with causes documented; no new accepts-invalid | bulk-whitelisting cases |
| PERF-42 | P0 | Root-cause the three worst runtime kernels: `namechars` 0.30×, `seg_gs_fill` 0.41×, `seg_gs_sum` 0.49× (then `varint` 0.76×, `lz4` 0.88×). Fresh post-rebase measurement first — PR #846 touched generator/IV paths | **M** S04 bench | each kernel ≥0.9× or a documented structural cause | tuning against the stale pre-rebase numbers |
| DW-01 | P1 | **`-g` emits zero `.debug_*` sections** — no DWARF at all. Stage 1: `.debug_line` + `.eh_frame` correctness so perf/gdb can symbolize; stage 2: variables/types. Prereq for honest PMU work on the 14700KF | **V** readelf: 0 debug sections | `perf report`/`gdb` resolve function+line on a `-g -O2` binary | emitting DWARF that contradicts actual code layout |
| FE-26 | P1 | **C23 completion**: `nullptr`, `constexpr`, `bool/true/false` keywords, digit separators, `enum E : T`, `_Float16`, `typeof` fix under `-std=c23` | **V** nullptr/constexpr/bool all rejected today | the §0 probe set compiles; torture subset green | shipping keywords in pre-C23 dialects |
| FE-27 | P1 | **Warning parity pack**: `-Wformat` (long vs `%d`), `-Wsign-compare`, `-Wunused-variable` under `-Wall -Wextra`; implement `-w` (currently a tolerated-unknown that spams every CI log line) | **V** 0 warnings where GCC emits 3; `-w` noise observed in ci_local | the probe file produces the three GCC-equivalent warnings; `-w` silences them | warning on system headers; diverging from GCC's format-string dialect |
| LK-31 | P2 | **Linker option contract**: verify each accepted `-z`/`-Wl` spelling acts or warns. Probed: `-z pack-relative-relocs` accepted, **no DT_RELR emitted**, silently. Emit DT_RELR or reject loudly; audit the audit's other four "dead flags" the same way | **V** readelf: no RELR | every accepted linker flag demonstrably acts or diagnoses | claiming RELR support via a no-op flag |
| DRV-01 | P2 | Remainder of the driver option contract: a generated table of every accepted flag × {acts, warns, errors}; the silent-no-op core is already fixed (loud warning + `LCCC_STRICT_OPTIONS=1`) | **V** probes §3 | table generated from code, gated in CI | hand-maintained table |
| SEC-01 | P2 | **Decision item, not a TODO**: PIE-by-default (distro parity, costs runtime perf — contradicts this project's priority ladder), default build-id (works explicitly today, off by default), real `-static-pie` (today a tolerated no-op warning). Record a measured decision | **V** default EXEC, build-id on request OK, `-static-pie` warns | decision in DECISIONS.md with A/B numbers | flipping defaults without the perf A/B |
| SEC-02 | P2 | Stack-protector canaries (`-fstack-protector-strong` is today a **loud error** with guidance — correct interim contract); `-fstack-clash-protection` after it | **V** error text verified | canaries work + the error becomes a feature | silent acceptance |
| IS-30 | P2 | Raptor-Lake cost model from uops.info measured latencies/ports for the instructions the backend actually chooses between (schedule-neutral subset first) | import ask; unverified | each table entry cites a uops.info measurement | wholesale latency tables nobody queries |
| MS-15 | P2 | i686 differential gate (x86-64 vs i686 output equivalence on the execute corpus) | import ask | wired into ci_local slow lane | blocking fast lane on it |
| MS-16 | P2 | Linker differential oracle (lccc-ld vs bfd/mold on identical inputs: section layout, dynamic table, runtime smoke) | import ask | structured diff tool + CI lane | byte-identity as the bar |
| COR-02 | P2 | Corpus runner: RUN-pipeline directive support (unblocks part of the 5130−377 unexecuted cases) | **C** S04 runner | executed-case count up | weakening outcome checks |

## 3. Reviewed and rejected / already-fixed import claims (do not re-import)

- **MB-03 "fold `__builtin_cpu_supports` to the effective target feature set" — REJECTED.**
  The current tree answers with a runtime CPUID probe on x86 (AUD-CPU-1,
  `src/ir/lowering/expr_builtins.rs`), which is what GCC 16.2, Clang 23.1 and
  ICX all do *even when `-march` guarantees the feature* (measured: GCC probes
  `avxvnni` under `-march=raptorlake`). Folding to 1 would answer "yes" on a
  CPU that cannot execute the instruction — the one answer no reference
  compiler gives. The "hardcoded Raptor-Lake table" premise is stale.
- **MB-01/MB-05 "silent no-op options" core — ALREADY FIXED** (PR #844, #753
  port): unknown options warn loudly and `LCCC_STRICT_OPTIONS=1` escalates;
  `-fstack-protector-strong` is a hard error with guidance;
  `-fstack-clash-protection`/`-static-pie` warn. Remainder tracked as DRV-01,
  SEC-01, SEC-02 at P2.
- **MB-21 "-O0 slower than -O1" — WRONG as stated.** Measured Ir:
  O0 2.68 G < O1 6.10 G. The outlier is `-O1` at 3.7× `-O2`; folded into
  MS-20 with the inclusive-cost breakdown.
- **MB-20's "71 `trimmed().to_string()` sites" — UNDERCOUNTED** (159 today)
  but allocation is the tail, not the head: the head is `peephole_for_metric`
  (44.8 %) and text-matching (24.7 %). Item admitted as MS-20 with corrected
  attack order.
- **MB-16 target_clones/IFUNC** — not new; merged into existing LK-19.
- **MB-15 LTO** — not new; remains PG-07 P2.
- **MB-40/41/42 engineering-system gates** — largely satisfied by the
  existing census ratchet, gate-parity checker and `ci_local.sh` driver;
  no separate items opened.
- **MB-04 "build-id dead"** — partially stale: `-Wl,--build-id` works
  (verified note emitted); only the *default* is off → folded into SEC-01.

## 4. What "beat ICX" means (kept from the previous catalog)

- **Integer codecs:** remat+RA+SIB (match, Adler, CRC) then ICX XMM-spill.
- **FP:** copy **ICX** `vfmadd231pd` YMM accumulator, **not** GCC's shape.
- **Aggregates:** SysV register class + ymm copies.
- **Parsers:** `btq`/range, segment RA, do **not** mega-inline (varint spike).
- **Bit kernels:** gcc `andn`+`cmov` on find_bit; `popcntl` on bitops.
