# 2026-09-28 — P0/P1 portfolio review, CC-O0CALL-1 repair, PF-TLS-1 landing

Review baseline: upstream `main` `6f8ace9c0bf98bb2c4df671342d89f983e0ab4c1`.
The reviewed portfolio is the one committed in `88c5d8e5a4f50f88a8bd35c29240d8534f7047ab`
(`88c5d8e5`; the file is not present on current `main`, so the review ran
against that historical text and the current tree — see
<https://github.com/ms178/lccc/blob/88c5d8e5a4f50f88a8bd35c29240d8534f7047ab/engineering/evidence/P0-P1-PORTFOLIO-2026-09-27/README.md>).

Environment: 2-vCPU Xeon KVM sandbox, 1.9 GiB RAM, **6 GiB swap installed
(`/home/user/.cache/lccc.swap{,.2}`)**, all compiler builds via
`scripts/build_lccc_fast.sh` (fastbuild `-O1`, `-j2`). No hardware PMU
(`perf_event_paranoid=2`, virtualized) — every runtime number below is
wall-clock only and flagged where it is below this host's noise floor.

---

## 1. Accomplished this session

### 1.1 CC-O0CALL-1 — the proposed fix is falsified; the real cause is repaired

* **Falsification.** The portfolio's item 1 proposes keeping accumulator-address
  loads off the non-SSA `-O0` path (`disable_regalloc` guard in `memory.rs`).
  Tested: the original Csmith reproducer (`artifacts/repros/miscompile_csmith_20260945_-O0.c`)
  still **SIGSEGVs** with that guard applied, and the guard's own regression
  test passes on the *broken* baseline — so it is not a discriminating test.
* **Real cause.** `%rcx` is both the secondary value cache and SysV's fourth GP
  argument. Staging an argument into `%rcx` left the cached pointer identity
  intact, so a later aggregate argument "rematerialized" the pointer as
  `movq %rcx,%rax; movq (%rax),%r8` and dereferenced the scalar that argument 4
  had just written (NULL in the reduced case).
* **Fix.** Invalidate the secondary cache *after* a GP argument writes ABI slot 3
  and *before* the next argument is staged, for every classification that can
  write it (scalar, i128/i64 register pairs, by-value aggregates, both mixed
  struct classes). Next-iteration placement also covers the early-`continue`
  staging paths (global-address remat, hazard restore) without discarding the
  current argument's source before it is read.
* **Evidence.** Reduced reproducer
  `tests/regression/call_secondary_cache_clobber.c` (8 ABI shapes; baseline
  SIGSEGV with and without `CCC_NO_PEEPHOLE=1`), gate
  `tests/regression/check_call_secondary_cache.sh` (20 output-checked arms:
  `-O0..-O3/-Os` × {default, no-peephole, no-GLA, both}), the full Csmith seed
  20260945 now prints `checksum = 39DEBCF9` and exits 0 like GCC, and GCC
  `-fsanitize=address,undefined` is clean on the reduced reproducer.
* **Status.** Done, pending the CI run recorded below.

### 1.2 PF-TLS-1 — implemented (with a prerequisite assembler defect fixed first)

See [`evidence/PF-TLS-1/README.md`](evidence/PF-TLS-1/README.md). Summary:
`set_all` 54 → 36 instructions, `sum_all` 41 → 28, the 3-store `set` 12 → 6
(matching GCC exactly), `tls_pass` 34 → 30 (GCC 37, Clang 31). Runtime is
**below this host's noise floor** and therefore not claimed.

Two additional defects were found and repaired while validating:

* the assembler dropped the relocation modifier for `sym@MOD+N`
  (`R_X86_64_32S` against a symbol named `sym@TPOFF` — a silent wrong address);
  lccc's relocations for that spelling are now byte-identical to GNU as 2.47.
* `lccc -fPIC -shared` could not **link** any `static __thread` (pre-existing,
  reproduced on the review baseline): Local-Exec was selected for shared output.
  Shared output now uses Initial-Exec; verified by a `dlopen` round-trip.

### 1.3 Re-measurements that change backlog premises

| Item | Backlog claim | Measured today (`-O2`, `ra_quality_census`) |
|---|---|---|
| RA-PRESSURE-3 (`sha256_transform`) | 218 insns, **64 spills** | **148** insns, **10** stkref, 14 rrmov (GCC 142/8/19, Clang 129/26/24) |
| PF-CLS-1 (`expat_utf8_name_length`) | 79 vs gcc 78 insns | **70** vs gcc 79 — lccc is now ahead statically |
| PF-CHACHA-1 (`chacha20_core`) | 118 vs gcc 180 / clang 176 | 117 vs gcc 182 / clang 176 (unchanged; the ICX gap remains) |

The sha256 copy/spill web is no longer the problem: lccc now has **fewer**
stack references than Clang (10 vs 26) and fewer register moves than both.

---

## 2. Verdicts on the 28 portfolio items

Legend — **A** agree as written, **A\*** agree with an amended scope/order,
**D** disagree (evidence attached), **P** partially agree.

| # | Item | Verdict | Basis |
|---:|---|---|---|
| 1 | CC-O0CALL-1 | **D** | The proposed `-O0` accumulator-address guard does not fix the reproducer and its test passes on the broken baseline. Real cause: secondary-cache (`%rcx`) clobber by GP argument 4. Repaired here. |
| 2 | RA-PRESSURE-3 (phi-copy cycle resolver) | **D** | A general resolver already exists (`plan_edge_copies` Kahn decomposition, exhaustive equivalence test over every parallel-copy graph for n=2..5, `check_phi_acyclic_order.sh`, default-on since 2026-09-12). The stale "64 spills" premise is gone: 10 stkref today. Re-scope to *physical* rotation (register permutation / `rorx`-class rotation) and instruction selection, not copy resolution. |
| 3 | RA-GLA-04 (post-RA slot-traffic feedback) | **A** | Diagnostics-first is right; `[RA-STATS]` already gives per-function `spilled=`. Extend it with post-RA traffic before any policy change. |
| 4 | RA-01 (opt-in allocation-order policies) | **A\*** | Sound and cheap, but order it *after* the causal spill census (#7): without causes, order policies are tuned blind. |
| 5 | RA-02 (explain every eviction) | **A** | Same rationale; a per-eviction explanation is the cheapest way to falsify the position-relative cost model. |
| 6 | RA-04 (phi-web physical-home diagnostics) | **A\*** | Agree, but the expected payoff dropped: RA-33 already removed the width-mismatch rejections. Measure before coding. |
| 7 | SPILL-01 (classify every hot stack reference) | **A** | The single highest-value missing measurement on this list. `ra_quality_census` counts stkref; no tool attributes *causes* (alloca/local, spill, call-argument area, address materialization). |
| 8 | SPILL-02 (reload placement experiment) | **A** | Only meaningful once #7 exists. |
| 9 | ADDR-01 (target-neutral operand roles) | **P** | Necessary long term, but a cross-backend refactor of address selection is high-risk; gate it behind a spike after RA/SPILL work, not before. |
| 10 | ADDR-02 (address-enabling homes in RA policy) | **A\*** | Agree with the "no benchmark-specific weights" rule. Sequence after #9. |
| 11 | ADDR-03 (shared staged-address representation) | **A\*** | Agree, same sequencing as #9. |
| 12 | PF-LZ4-1 (prove byte ranges before widening) | **A** | Backlog records a 10.8× regression when the idiom was disabled and a live overread hazard. Do not widen without both range proofs. |
| 13 | PF-MB-1 (Mandelbrot) | **A** | Modelling before vectorizing is right; note that static AVX counts prove nothing here. |
| 14 | PF-FB-1 (find-bit) | **A** | Already falsified as a peephole (median 1.013, i.e. no win). Structure, not instruction removal. |
| 15 | PF-SCHEDULER-1 (sha256 message schedule) | **A** | High value; needs sliding-window legality work. |
| 16 | PF-CHACHA-1 (ARX vs ICX) | **A** | Re-measured: static parity with GCC/Clang reached, ICX gap (74 @-O2) remains. |
| 17 | PF-TLS-1 | **A — IMPLEMENTED** | See §1.2. Two prerequisites were not in the item: the assembler `sym@MOD+N` defect and the `-shared` illegality of Local-Exec. |
| 18 | PF-CLS-1 (byte-classifier chains) | **A\*** | Agree it is branch/schedule-bound — and today lccc is *ahead* of GCC statically (70 vs 79), so the 1.31–1.37× runtime is pure prediction/layout. Measure branches, not instructions. |
| 19 | RA-PRESSURE-4 (aggregate webs) | **A** | Classify before changing split policy. |
| 20 | MI-CLOBBER-1 (clobber model before `Call`) | **A** | Strongly: "correct by rejection" must not be weakened before explicit clobbers exist. |
| 21 | MI-XMM-1 (XMM register class) | **A** | Prerequisite for vector RA quality. |
| 22 | MI-PARAM-1 (emissive `ParamRef`) | **A** | Incoming-register identity must be preserved; the fallback's aliasing hazard is a live miscompile risk. |
| 23 | SIMD-02 (XMM copy webs) | **A** | Measure surviving copies first. |
| 24 | ABI-02 (caller-save vs callee-save costing) | **A** | Needs weighted call frequency from a profile. |
| 25 | PGO-01 (profile ingestion audit) | **A** | Profile must demonstrably reach RA/scheduling/layout/unroll/inlining before it is trusted. |
| 26 | LOOP-01 (recurrence classification) | **A** | Feed one consumer at a time with A/B evidence. |
| 27 | SCHED-01 (target-neutral scheduling interface) | **A** | Interface before policy — this ordering is correct. |
| 28 | SCHED-02 (critical-path-first on hot loops) | **A** | Depends on #27. |

Net: 17 agree, 5 agree-with-amendment, 1 partial, 2 disagree. Both
disagreements are backed by measurements made this session, not by opinion.

---

## 3. To-do (ordered by expected value / risk)

1. **SPILL-01 causal census** — a tool that attributes every stack reference to
   (alloca/local, spill reload, spill store, call-argument area, address
   materialization, callee-save push), per function, JSON output. Reuse
   `ra_quality_census`'s function splitting; combine with `[RA-STATS] spilled=N`
   to bound the spill share. Gate: classification must cover ≥95 % of references
   on the benchmark corpus.
2. **Re-scope RA-PRESSURE-3** — replace the "cycle resolver" framing with
   "physical rotation / isel" and re-baseline `sha256_transform` (148 insns,
   10 stkref, 14 rrmov today). Target: Clang's 129.
3. **PF-CLS-1 runtime instrumentation** — the expat kernel is now statically
   ahead of GCC, so measure branch misses / layout. Without a PMU on this host,
   use `scripts/callgrind_ab.py` (deterministic `Ir`, I1/LL misses, branch
   mispredictions) instead of wall-clock.
4. **RA-GLA-04 feedback** — per-planned-location-piece slot traffic, machine
   readable, before reopening any spill gap.
5. **ADDR-01/02/03 spike** — only after 1–4; keep the refactor behind a flag.
6. **Target hardware** — every number here is unverified on the i7-14700KF.
   Re-run `tls_seg_access`, `sha256_transform` and `expat_xml_scan` there with
   ≥200 ms arms and output checking.

## 4. Harness notes (for the next session)

* Swap is NOT recreated automatically: `scripts/ensure_swap.sh` only acts when
  `/proc/swaps` is empty, and the harness wipes `/`. Two files were created this
  session (2 G + 4 G); recreate them before any `cargo` build, otherwise the
  `cargo-test` gate is OOM-killed (observed: SIGKILL during the lib-test link).
* Never run a second compiler build while `ci_local.sh` is running — the OOM
  kill above was caused exactly by that.
* Oracle pins are current: `scripts/godbolt.py audit` reports gcc 16.2,
  clang 23.1.0, icc 2021.10.0 all `ok`; ICX resolves through the moving
  `cicxlatest` channel.
