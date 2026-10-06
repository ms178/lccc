# 2026-10-06 — audit adjudication and codegen fixes

## Scope and verdict

Base: **08f4e1a124d753e9eeddb1c43ce68b54d4d95f68**, fetched from ms178/lccc main.
The submitted audit was reviewed against this exact tree, not accepted as a
specification. The broad direction (measure generated code, improve loop/address
interfaces, do not blindly enable spill gaps) is sensible. Its strongest causal
claims are **not established**. Several alleged missing capabilities already
exist, one high-priority branch defect is already fixed, and a fresh red-team
counterexample exposed actual wrong code in IVSR.

This session does not claim a complete whole-codebase audit, end-to-end wins on
all named packages, or superiority over GCC/Clang/Intel on Raptor Lake. Those
are a research program, not consequences of a static instruction census.

### Five reproducible defects fixed, plus a quarantined FP allocation candidate

| Defect | Evidence before | Fix / validation |
|---|---|---|
| IVSR-WRAP-1: truncating recurrence treated as affine pointer progression | Defined 8-bit wrap: GCC 510, LCCC 509; IVSR-off 510. Checked-in wider battery failed baseline. | Do not look through type-changing casts in basic recurrence matching. 8-/16-bit wrap battery passes O0–O3 and GCC UBSan/bounds. |
| VAPACK-LEN-1: count markers escaped the inliner | Full gzip 1.14 build failed linking `__lccc_va_arg_pack_len`; reduced fortified open failed O1–O3. | Stop double-remapping sentinel IDs and replace the valid zero-extra-argument case. 0/1/4-extra and actual fortified-open regressions pass O1–O3. |
| FP-LANE-1: missing scalar type/class at SLP boundary | Extracted F64 lanes stored then reloaded every nbody pair. | Type four scalar extract intrinsics; use the type in **both** RA filters. Unit + 12 BB-SLP regressions + F32/F64 test. Candidate inner pair 43 → 37, TU 302 → 296. **Allocation opt-in after adverse default-layout timing**; type correctness retained. |
| ORACLE-MEM-2: false memory/spill diagnosis and comparability claims | `movsd %xmm3,24(%rax,%rcx,8)` counted as a load; LEA counted as a load; equal calls called equivalent. | Parenthesis-aware operands, data-access classification, explicit stack-ref label/JSON, tests; equal calls now unproven and mismatches flag wins too. |
| ORACLE-SEM-2: valid ICC memcmp called wrong code | Four-oracle run: ICC -1, others -213; 1/8 programs marked divergent. | Normalize only libc memcmp's specified sign in the C fixture; keep the hand-written scan's exact byte difference observable. Independently check less/greater/equal/zero-length. Four-oracle rerun 8/8 agree. All-vendors preset includes classic ICC. |
| LINK-ORACLE-VERSION-1: cached filename trusted as a version | bfd/mold restore branches checked existence only. A binary named bfd-2.47 could be 2.44. | Require successful actual version banners before exposing wrappers/inventory. Offline stale-version, prefix, suffix and failed-command tests; local/hosted gate parity. |

Five fixes and one implemented-but-quarantined allocation candidate are delivered,
**not** five default-on performance wins, and **not** a claim that the audit's five broad
performance projects (full IVopts, branch lowering, FP, callee-save economics,
SHA scheduling) have all been implemented. The branch item needed closure,
not invented work; callee-save/SHA/remaining addressing remain open with
explicit acceptance criteria below.

## Audit claims: agree, qualify, or reject

| Submitted claim | Disposition and reason |
|---|---|
| “Median 1.17, about +15.8%” | Reject the equivalence. 1.17 means +17%; 15.8% was a ratio of summed counts. Different statistics. Neither measures runtime. |
| Nbody inner loop 110 vs 14, no stack references | Reject as current matched-loop evidence. `.LBB7` is an outer composite loop; the actual pair loop `.LBB12` is 43 instructions before, 37 after, versus GCC `.L18` 29. Four explicit lane stack accesses existed before. The historical text itself displays stack operands while claiming zero. |
| IVopts is absent and is the primary cause of the corpus gap | Reject absence: IVSR, IV widening, un-IVSR, LICM and global-address CSE exist. Remaining coverage/profitability gaps are real. A corpus-wide causal attribution needs controlled pass A/B and dynamic hotness, not mnemonic correlation. |
| 171 movsd vs 54 proves excessive FP copies | Reject that inference: memory forms and register moves differ; VEX vmovsd and packed moves were omitted. New `xmm_reg_moves` is spelling-normalized register-only, not a claim every move is redundant. |
| Fewer stack references rules out RA | Reject. Static prologue accesses and dynamically hot loop spills have different weights. FP-LANE-1 is exactly an allocation-eligibility defect, even though improving eviction heuristics would not fix it. |
| Linear scan remains start-ordered | Agree for `LinearScanAllocator::run_with_seed`. Qualify architecture: tier-2 segment graph coloring, coalescing, multiple waves and residual fill also exist; the compiler is not one bare linear scan. |
| GLA remains phase 1; leave spill gaps off | Agree with current source/policy and historical negative A/B. No default was changed. |
| Callee-save economics deserves analysis | Agree as a hypothesis. Final-default census: 589/1298 callee-save references, 128/1298 classified spills; this is not dynamic cost. 262 `temp` references are not proof of unavoidable storage. |
| Implement a generic phi copy resolver next | Agree with audit's rejection: existing resolver/tests supersede that old task. No duplicate resolver added. |
| SHA gap must be physical rotation/scheduling | Plausible candidates, not proven root causes. Whole-TU current counts 279/231/232 for LCCC/GCC/Clang cannot be substituted for old per-kernel counts 148/142/129. |
| Less new peephole work; fix structural interfaces | Agree as a prioritization rule, not a ban. FP-LANE-1 fixes type/class propagation rather than matching assembly text. |
| Raptor Lake evidence missing | Agree. Host is KVM Xeon family 6 model 106, not i7-14700KF. `perf stat` reports cycles/instructions **not supported**. |
| ChaCha is close to GCC but ICX remains interesting | Current whole-TU counts 193/269/257/138 (LCCC/GCC/Clang/ICX) support a code-size question, not runtime superiority. Do not mix them with per-core-function 118/63. |

Additional backlog safety correction: **LOOP-PREHEADER-3's proposed rule was
unsound**. Header dominance does not imply a load executes. Every natural-loop
block is header-dominated, including a conditional dereference bypassed on every
iteration. `audit_loop_contracts.c` checks null pointers on skipped and early-exit
paths; do not weaken LICM on that argument. LLVM's formal terminology is useful
here as a definition, not an optimization oracle:
<https://llvm.org/docs/LoopTerminology.html#loop-definition>.

## Current measured evidence (candidate and final default distinguished)

All static comparisons use **-O2 -march=x86-64-v3** and identical sources.
The corpus has **55 programs** (54 existing plus the new composition kernel),
not the historical 51. Excluding the same four recursion names leaves 51:

- LCCC/GCC median instruction ratio: **1.22084**.
- Geometric mean: **1.19897**; arithmetic mean: **1.26764**.
- Best/worst individual ratio: **0.54301 / 4.30769**.
- LCCC smaller/larger/tied: **13 / 38 / 0**.
- Within-session before/**candidate**: only nbody instruction and explicit-stack counts
  changed across these 55 sources (302→296 instructions; 30→26 explicit stack
  refs with the corrected metric). No static count regression hidden in a mean.
  This is the opt-in candidate, not the final default. The candidate was later
  quarantined. Final default remeasurement: **55/55 unchanged instruction/stack
  counts** versus baseline; median 1.22084 and geomean **1.19944** versus GCC.
  Thus this delivery claims **no default-on runtime speedup** from FP-LANE-1.

This is an instruction-size screen, **not** a speed ranking. Whole-TU totals
also include harnesses, cold paths and unrolling. See the full matrix and hashes
in [evidence/2026-10-06-audit](evidence/2026-10-06-audit/static-matrix.md).

### Nbody causal experiment

The before arm is the compiler with IVSR-WRAP-1 already fixed, isolating the FP
change. Same source, flags, host libraries and linker; both arms print identical
energy output. The primary artifact is the saved before/after assembly. The d²
chain loses two stack stores, two reloads and two staging moves; frame allocation
shrinks by 16 bytes. The register-class omission, not a new spill heuristic,
explains this observed change.

**Runtime red-team result — allocation remains OPT-IN:** the final compiler
requires presence of `CCC_FP_EXTRACT_HOMES` (use `=1`; unset, not `=0`, to disable).
This setting is typed in `RaConfig` with policy tests. The scalar result types
remain accurate independently of allocation policy.

Exploratory 15-pair unpinned run: median after/before **0.7565**, minimum ratio
**0.8045**, approximate sign-test p=0.0003. Because the shortest after samples
were under 200 ms and affinity was not pinned, this is explicitly exploratory.
The longer CPU-0-pinned, 10-million-step runs exposed a material counterexample:

| Link/control | Pairs | Median after/before | Minimum ratio |
|---|---:|---:|---:|
| LCCC default link, no `-lm` | 21 | **1.52860** | **1.64318** |
| Same compilation with `-lm` | 21 | 0.79484 | 0.79070 |
| GCC14/bfd2.44 link, both `.data=0x406000`, bodies=0x406010 | 21 | 0.89401 | 0.85510 |

The first row is a **real measured regression in that configuration**, not
excluded as noise. `perf_ab.py` checked every execution's output/status in both
first rows. The matched-layout diagnostic used `paired_ab.py`, which checks
initial agreement, but does not check every timed output; treat it as causal
screening, not a complete correctness oracle. All raw sample sets are retained.

Inspection: without `-lm`, before bodies=0x404f40, candidate bodies=0x404f00.
The candidate's fifth velocity pair starts at bodies+248=0x404ff8: its 16-byte
store crosses a 4 KiB page (and cache line). With `-lm` the addresses shift by
16 bytes, avoiding that particular page split; the direction reverses. Matched
addresses remove this confound and show a smaller gain. PMU cannot establish
the precise cycle attribution here, so this is a strong layout hypothesis, not
a hardware-counter proof. Fewer instructions alone did NOT justify promotion.

No unconditional speedup is claimed; no benchmark-specific alignment hack was
added. Final default FP lane allocation is unchanged until a layout-distributed,
real-target experiment justifies promotion. This negative result moves layout
sensitivity ahead of further speculative scheduling changes.

### New real-source benchmark

`zlib_ng_adler32_combine.c` extracts the checksum-composition arithmetic from
zlib-ng **2.3.3**, selected via archpkgbuilds commit
`d9953b4f185fe6b33506af4b5730039af7849174`. Provenance, source digest, license,
adaptations and independent self-checks are in `tests/benchmark/WORKLOAD_PROVENANCE.md`.
It adds constant division/modulo and modular branch decisions rather than
another duplicate byte-scan loop. Existing gzip CRC, zlib-ng accumulator,
Expat UTF-8 scanner, SQLite, glibc and kernel-derived kernels remain the broader
screen. No claim that these kernels replace full project validation.

### Full gzip 1.14 result after VAPACK-LEN-1

The pinned archive was fetched from the kernel.org GNU mirror after the primary
urllib HTTPS request failed; SHA-256 exactly matches the runner's pinned
`01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6`.
Archive signature was not verified; the historical archpkgbuilds recipe digest
mismatch is **not resolved** by this run. Both LCCC and GCC 14.2 complete builds
and **30/30 upstream tests** pass. Six compressed-stream cases are byte-identical,
and both binaries restore both exact 8 MiB inputs. Fortify was not disabled.

CPU 0, randomized per-round order, 11 rounds + 2 warmups, no concurrent builds:

| Workload | LCCC median ms | GCC14 median ms | LCCC/GCC |
|---|---:|---:|---:|
| Source compression level 1 | 76.543 | 61.094 | 1.25288 |
| Source compression level 6 | 214.138 | 185.301 | 1.15562 |
| Source compression level 9 | 525.103 | 457.111 | 1.14874 |
| Mixed compression level 6 | 148.655 | 117.056 | 1.26995 |
| Source decompression | 22.861 | 21.319 | 1.07232 |

Geomean **1.177639**, mean **1.179904**: LCCC is slower in every measured case.
This is not a before/after optimization speedup: the before compiler could not
link this configuration. Short cases, especially decompression, are only rough
VM screens; no PMU/bare-metal conclusion or uncertainty interval is claimed.
Raw samples, hashes, inputs and build flags: `gzip-results.json` alongside the
static evidence. The gzip integration failure is fixed, not hidden by a small
kernel benchmark or an error-skipping flag.

## Reproduce

```sh
# Required constrained-machine build; all compiler builds in this session used
# fastbuild (-O1), -j2. 8 GiB active swap was eventually needed for unit builds.
bash scripts/ensure_swap.sh
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 bash scripts/build_lccc_fast.sh

# New correctness contracts
bash tests/regression/check_audit_loop_contracts.sh
bash tests/regression/check_linker_oracle_versions.sh
python3 scripts/test_codegen_oracle.py
python3 tools/oracle/godbolt_oracle_selftest.py
for f in ivsr_narrow_wrap slp_fp_extract_home; do
  for opt in -O0 -O1 -O2 -O3; do
    target/fastbuild/lccc "$opt" tests/regression/$f.c -o target/$f
    target/$f
  done
done

# All four execution oracles: exact stdout/exit except the C program's own
# specified memcmp sign normalization. ICC remains a separate compiler.
python3 tools/oracle/godbolt_oracle.py --oracle-set all-vendors \
  --semantics-only --jobs 2 --json target/four-vendor.json

python3 scripts/codegen_oracle.py --all-functions --totals \
  tests/benchmark/programs/*.c --local target/fastbuild/lccc \
  --flags='-O2 -march=x86-64-v3' --json target/current-codegen.json
python3 scripts/stack_census.py --corpus --lccc target/fastbuild/lccc \
  --cflags='-O2 -march=x86-64-v3' --json target/current-stack.json

# PRE is retained before-FP. For the final compiler, explicitly opt in B:
# append --env CCC_FP_EXTRACT_HOMES=1 to the following perf_ab command.
# Repeat with and without --cflag=-lm; preserve BOTH results.
taskset -c 0 python3 scripts/perf_ab.py --compiler-a "$PRE" \
  --compiler-b target/fastbuild/lccc --only nbody --opt=-O2 \
  --cflag=-march=x86-64-v3 --cflag=-DSTEPS=10000000 --reps 21 \
  --json target/nbody-pinned.json

python3 tests/workloads/gzip-1.14/run.py --archive /path/to/gzip-1.14.tar.xz \
  --lccc target/fastbuild/lccc --artifact-dir target/gzip-e2e --rounds 11 --warmups 2

CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 CI_LOCAL_JOBS=2 \
  bash scripts/ci_local.sh --fast
# The snapshot helper additionally requires the complementary slow half.
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 CI_LOCAL_JOBS=2 \
  bash scripts/ci_local.sh --slow
```

## Validation and limitations

- Rust unit suite: **4123 passed, 0 failed, 7 ignored**, plus one binary test.
  The initial attempt caught a missing import in the new unit test; it was fixed
  and the full suite rerun, rather than suppressing the unit gate.
- O0–O3 narrow-wrap regression; O2/O3 base/v3 scalar extract checks; twelve
  BB-SLP execution fixtures passed. Five branch shapes are already optimized
  in upstream and now have an explicit assembly gate.
- VAPACK-LEN-1 was discovered by the full gzip build, not a synthetic benchmark.
  The undefined marker was not a missing runtime library: it represents a
  compile-time count. A runtime stub returning a guessed value would be wrong.
  The fix is in the inliner, with baseline-failing 0/1/4-extra and fortified-open
  regressions in the same local/hosted gate. The workload runner now retains
  full configure/build/check diagnostics on command failure (26 helper tests).
- Four-vendor execution: **8 programs agree, 0 divergence/error** after fixing
  the memcmp test's invalid magnitude requirement. The previous ICC divergence
  is preserved as a diagnosis, not silently excluded.
- Local GCC is **14.2**, local system GAS/bfd **2.44**; those are local differential
  tools, **not** the requested GCC16.2/GAS2.47/bfd2.47 oracles. Remote versions
  are recorded independently. ICX identity is the latest channel, not a proved
  fixed release. Mold's v2.42.1 source confirms `MOLD_TARGETS='X86_64;I386'`;
  the existing provisioner already uses it. No linker-performance claim is made.
- First final-tree fast-CI run: **159 passed, one failed, five skipped**. The
  sole failure was an unavailable 32-bit C++ header (`bits/c++config.h`) in the
  i386 DSO test. Installed `g++-multilib`, rather than skipping the test. All
  compiler/unit/codegen/rustfmt/clippy gates passed in that run. A complete
  frozen-tree rerun is required; the final delivery receipt records its result.
  `--fast` omits five slow gates in this revision (some script comments still
  say three), and is not hosted full-CI equivalence.
- That CI run fetched, built and used actual **GAS banner 2.47.20260726**, native x86-64 target,
  for the assembler differential gate. System bfd remains 2.44; do not call
  the dated GAS 2.47-family build an exact 2.47 release or a bfd 2.47 linker build. The first download mirror timed out;
  the fallback mirror succeeded. Targeted requests were not silently downgraded.

Final default causal census: **1298 references**, 1284 attributed (**98.92%**),
14 unknown; alloca 238, wide 19, pressure spills 128, non-GPR 42, temp 262,
callee saves 589, outgoing args 1, incoming args 5. The quarantined candidate's
older 1294/99.07% snapshot is retained separately, not presented as final default.

The causal stack census covers programs **plus the kernel corpus**, includes
callee-save traffic, and is not the same scope as `stack_refs` in a whole-TU
program-only table. Their totals must not be directly subtracted.

## Follow-up queue (acceptance criteria, not promises)

1. **P0: complete IV recurrence proof.** The truncation bug is fixed; unsigned
   32-bit wraps and signedness changes in derived expressions still deserve
   adversarial testing. Reuse/share no-wrap reasoning, not “addressing means UB”.
   Prove widened recurrence equality, init, stride arithmetic and last update.
2. **P1: resolve FP/link-layout sensitivity before default promotion.** Reproduce
   with varied but identical per-arm `.data` addresses; identify page/cache-line
   split effects and code alignment independently. Do not align one benchmark
   specially or discard the losing configuration. Then validate on Raptor Lake.
3. **P1: scalar/SIMD address sharing.** Nbody SLP memory intrinsics retain direct
   base/byte-offset operands that IVSR's GEP-use list does not enumerate. Prototype
   common pointer recurrences with displacement preservation, verify SSA/hidden
   uses and nested-loop legality; compare pair loops, not composite main regions.
4. **P1: callee-save economics.** Census output is available; obtain dynamic
   calls/loop counts before charging prologue saves like loop spills. Keep spill
   gaps off. Price caller-save/reloads versus callee-save using actual hotness;
   require broad output-checked A/B and per-workload regression reporting.
5. **P1: SHA scheduling/recurrence.** Isolate compression from driver/expansion,
   compare the same loop and ISA, establish register/port dependencies. No new
   generic parallel-copy resolver. Test message-schedule vectorization against
   scalar and pressure baselines before accepting code growth.
6. **P1: bare-metal target confirmation.** Replay pinned long A/B on i7-14700KF
   P-core and E-core separately, record microcode/governor/SMT/temperature;
   collect cycles, instructions, branches and model-supported Top-Down counters.
7. **P2: general GLA/RA redesign.** Only after representative dynamic bottlenecks
   justify it. Current tier-2 coloring and copy-web support must not be erased
   from architectural descriptions.
8. **Measurement debt:** mnemonic memory classification is a static estimator,
   not disassembly-backed semantic decoding. `temp` slots need finer causality.
   Equal call counts remain unproven. Hot-loop-density auto-selection can choose
   the cheapest loop rather than the hot one; nbody needs explicit pair-loop IDs.
9. **Scope debt:** no complete Linux kernel/glibc/SQLite build-and-run audit or
   full zlib-ng/Expat project certification was performed by these kernel tests.
   Promote a proven kernel gain to end-to-end package workloads before broad claims.

## Persistence and recovery

Every targeted validation was followed by an atomic snapshot. Interim snapshots
are honestly labelled **UNGATED** until CI succeeds; this is loss prevention,
not a false CI stamp. The repository's hardened `scripts/lccc-snapshot.sh` was
used (the supplied mechanism with verified atomic archives and patch checks).
The first publication hit its workspace budget; it left the canonical patch
intact and was retried after moving rebuildable toolchains/worktrees out of the
small delivery workspace. The persisted patch, compressed tracked source,
verified base-dependent session bundle and ledger remain under `/home/user`.

Use the source archive for **offline** recovery if the worktree disappears. The
session bundle preserves commits but explicitly requires the base; it is not a
standalone full-history backup. Check the delivery receipt's hashes and rebase
onto newly fetched main before the next session. Do not treat historical CI
stamps as valid after editing or rebasing.
