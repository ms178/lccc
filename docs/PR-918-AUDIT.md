# PR #918: correctness, measurement, and merge audit

## Scope and interpretation

This is a review of the supplied SIMD/tooling change and the repairs made on this branch, not a proof that the compiler is free of defects. The patch was obtained with the GitHub Contents API. Integration retained main's benchmark-ranking gate and conditional SSE2 scratch reservation through two explicit context adaptations; no fuzzy application was used.

Runtime performance is the objective, but correctness and credible measurement are prerequisites. Static instruction counts are useful diagnosis, not elapsed-time speedups. QEMU establishes functional evidence, not native AArch64/RISC-V performance. GCC 16.2 and Clang 23.1.3 were the available reference compilers. ICX 2026.2 was not available in the permitted toolchain sources; no ICX result is implied.

## Findings and repairs

| Finding | Evidence | Resolution / consequence |
|---|---|---|
| Aggregate pair recognition lacked sufficient deletion, alias, and observer proofs | Six failures in the original 13 aggregate tests | Index definitions/uses; require complete deleted-use coverage, unsigned low-half extension, checked pointer provenance and displacement arithmetic, and effect/observer barriers. Preserve shared dynamic indexing without inventing disjointness. The expanded 18-test suite passes. |
| Full-width self-reload deletion ignored VEX upper clearing | Adversarial encoding regression | Retain VEX/EVEX self-reloads; remove only the semantically redundant legacy form. Do not replace YMM/ZMM contents with an XMM approximation. |
| Noncontracting two-wide matmul lost its intended packed path | Forced-two-wide shape and bit-pattern regression | Carry nonfused multiply/add explicitly in IR. Keep contraction permission separate from instruction availability. The x86 shape/runtime gate passes. |
| AArch64 integer comparison indexed the GP register table with an FP home | Full matmul fixture ICE, traced to `value_reg_name` | Reject FP physical homes from the GP fast path and use existing FMOV materialization. Two unit tests cover FP ranges and unchanged GP code shape. The matmul fixture compiles and runs freestanding under QEMU with both `-ffp-contract=off` and `fast`. |
| Conversion fixture itself read beyond a float array | GCC ASan reproduction | Supply a complete vector-sized input and check all four MXCSR rounding modes, both signs, and 32-/64-bit rounded/truncating results. Sanitized reference and native differential checks pass. |
| Matrix checker admitted incomplete or contradictory measurements | Five original adversarial tests produced 16 failures | Require compiler/status/body/hash/oracle/link completeness; reject invalid baselines. All five repaired checker tests pass. |
| Provisioning mocks inherited host linker availability | Hermetic PATH regression | Use an allowlisted utility PATH and explicit Bash; all 15 provisioning mock tests pass. |
| Slot-forwarding gate swallowed a failing installed GCC oracle | `GCC=false` previously appeared successful | Installed-oracle failures are fatal; genuine GCC succeeds and the negative control fails. |
| Atomic coverage stopped before optimized consumers | Runtime gate used only O1 despite a separate repaired call-clobber reproducer | Run lowering at O1/O2/O3 for baseline and cx16, and semantics at all three levels for baseline, cx16, and x86-64-v3. All six lowering and nine runtime configurations pass. O2 agrees with GCC 16.2; the separate call-clobber gate passes O0–O3 across five ISA configurations. |
| Three new workload defaults were below the measurement floor | Initial process times approximately 1.6/7/12 ms | Increase default passes to 8192/128/512 for gzip/list/zstd, retaining bounded Callgrind overrides. All nine compiler/workload outputs agree; longer defaults pass GCC ASan/UBSan. Noise still prevents a speedup claim. |

The scalar aggregate safety fixture also passes five optimization levels with LCCC/GCC/Clang natively, and with AArch64 and RISC-V freestanding QEMU execution. Its `__int128` requirement is unsupported by i686: this is an explicit coverage boundary, not a claimed 32-bit pass.

### Additional gate findings

The completed evolving-tree fast run reported **370 passed, 15 failed, 13 skipped**. It is not a final-tree pass stamp. Strict all-target Clippy and formatting passed; the latest completed library run has **5,741 passed, zero failed, 10 ignored**.

* `llvm-scalar-facts`: the hot-web mechanism witness remains inert. Narrowing protection to actual whole-web eviction victims is independently tested: disjoint protected occupants stay assigned, but a protected member of an evicted copy web still vetoes the mutation. This does not resolve the mechanism witness.
* `gla-cross-reach-band`: AArch64 matmul now has a 128-byte frame at both reach bands; the existing pin requires 96. The N=17 fixture runs correctly under QEMU, but the frame/code-shape change is not waived or rebaselined without a proper before/after analysis.
* `env-test-hygiene`: 148 pass-local environment read sites exceeded the 147-site budget. Move the fixed-SLP ablation to the shared pipeline entry, reusing the existing no-fixed-SLP wrappers; the matcher no longer reads the environment. The unchanged 147-site budget now passes. Entry-selection and fixed-SLP mechanism controls accompany the change.
* `audit-census-ratchet`: the pin counted 514 knob names while both fetched main and this branch contain exactly 515, with identical name sets. Correct only that stale inventory count to 515; retain every other threshold. This is an explicit main-inventory repair, not permission to add a knob or relax a performance ceiling. The census now passes.
* Nine assembler/XML gates failed because pinned binutils or the Arm XML package was absent and the upstream hosts were unreachable. Build the pinned binutils from the already digest-verified repository cache; do not substitute the host assembler. The exact Arm XML tarball was restored from the repository cache tag and passed the provisioner digest/manifest checks. All nine assembler/XML gates now pass, including 526/526 documentation golden words.
* `kernel-relocs-tool` and strict `linker-suite` lacked the kernel relocs consumer; the linker suite also encountered an older assembler (301 pass, zero fail, five mandatory skips). Stage the exact v6.12 sources through the GitHub Contents API, then use the existing digest-checking provisioner. The relocs tool has now built successfully; The remaining GNU linker dependency was built from the same verified binutils 2.47 sources. The strict linker suite now passes with no mandatory skips.

The 15 failed registered invocations were rerun after the repairs: **13 are now green** (12 in the grouped rerun, followed by the strict linker suite after provisioning GNU ld 2.47). **Two remain red:** `llvm-scalar-facts` and `gla-cross-reach-band`. This is not a substitute for a complete final-tree fast run and does not produce a pass stamp. The compiler tree's latest library run passed **5,741 tests** (10 ignored), strict all-target Clippy passed, and formatting passed.

The AArch64 frame change is tied to newly admitted nonfused matmul vectorization: the N=17 vectorized body is 93 static instructions with a 128-byte frame, whereas disabling the `vectorize` pass gives 55 static instructions and a 96-byte frame. The vectorized body contains packed F64 multiply/add pairs, and both the N=17 runtime fixture and the stronger bit-pattern fixture pass under QEMU. Body-size counts include control/remainder scaffolding and do not establish native performance; the frame increase still requires an explicit cost/acceptance decision rather than an arbitrary gate update.

## Quantified performance position

The hardened 122-row SSE2 matrix records **3,230 LCCC instructions**, unchanged by the safety repairs. The corresponding totals are GCC **1,544**, Clang **1,553**, and per-row best **1,425**. LCCC loses all 122 static-count comparisons; geometric mean LCCC/per-row-best is **1.9233**. These numbers do not imply an equivalent runtime ratio.

Large gaps remain: `sqrtsd` 85 versus 11 instructions, `maxsd` 84 versus 11, `minsd` 82 versus 11, and matmul 65 versus GCC 16 / Clang 141. This supports targeted lowering and ABI investigations, not loosening LCCC's own instruction-count budget.

Pinned, randomized, 15-round measurements after two warmups:

| Workload | LCCC median ms | GCC median ms | Clang median ms |
|---|---:|---:|---:|
| gzip longest-match | 103.91 | 87.88 | 79.98 |
| Linux list sort | 268.63 | 180.17 | 180.62 |
| zstd double-fast | 235.94 | 239.23 | 231.25 |

Every arm exceeds 50 ms, but coefficient of variation is **11.6–25.6%** on this shared host. These are calibration observations only. Neither the near tie on zstd nor the apparent gaps establish accepted speedups/regressions. Raw samples and the reproduction command accompany the workload-calibration snapshot and `PERFORMANCE-BACKLOG.md`.

## Design decisions

**Agree:** reuse the existing memory-backed vector IR rather than adding backend-specific aggregate-copy machinery; represent nonfused arithmetic explicitly; price complete evicted webs; use target-aware instruction footprints; require negative controls for the measurement harness; keep unsupported optimization cases conservative.

**Disagree with the original implementation:** recognizing a syntactic pack/store shape is not enough to delete its intermediate values; an opcode name does not prove a vector's byte extent; removing a reload can change architectural upper bits; a benchmark is not credible merely because its output matches; and a missing or failing oracle must not produce a green result.

**Do not fold into this patch without separate evidence:** public vector-ABI migration, general IPA callback cloning, cross-target sibling calls, or broad allocator-policy relaxation. These require dedicated design and differential suites. AArch64 currently still selects a conservative nonfused two-wide entry even with fast contraction: that leaves optimization potential, but must not be reported as newly implemented fused policy.

## Assessment and merge criteria

Ratings are engineering judgments, not measured probabilities:

| Dimension | Rating / 10 | Reason |
|---|---:|---|
| Design | 7 | Good reuse and explicit semantics after repairs; scope combines several independently risky subsystems. |
| Correctness confidence | 7 | Specific miscompilation and ICE risks repaired with adversarial coverage; whole-compiler correctness is not established. |
| Regression infrastructure | 8 | Stronger fail-closed and mechanism controls; final-tree fast CI remains a required acceptance condition. |
| Demonstrated runtime performance | 5 | Useful calibration and code-shape evidence, but no sufficiently quiet before/after runtime acceptance. |
| Cross-target confidence | 6 | Native x86 and targeted emulated AArch64/RISC-V evidence; no universal target or ABI proof. |

**Merge recommendation at the time of writing: hold until the final-tree fast-CI run and strict lint are green, including a genuine hot-web mechanism witness, the AArch64 frame analysis, and a complete frozen-tree fast run.** Do not waive the witness, relax ceilings, or describe this patch as perfect. The broader optimization backlog is not itself a reason to reject a correct, bounded repair, but an unresolved required gate is.

### Resolution of that hold (recorded, not silently dropped)

A merge candidate must not ship an unaddressed hold, so each condition is
resolved here with evidence rather than waived:

* **Hot-web witness.** Closed by fixing the *gate*, not the baseline: the pass is
  inert because allocation improved (the 3100- and 310-use candidates are now
  already assigned), the protection veto never blocks a steal worth having (max
  31 uses), and the mechanism is covered by 15 unit tests that run the pass on
  synthetic contexts. The gate now asserts the structural contract and prints
  whether the assembly differs. See `FOLLOWUP…md` §5c.2.
* **AArch64 frame.** Attributed with a bisection rather than an opinion:
  96 bytes / 62 instructions at base `c8e9226bb`, 128 / 101 on main
  `eeece44e9`, and 96 / 95 with `CCC_DISABLE_PASSES=all`, so main's pass work
  causes it and not the vectorization this branch admits. The band contract
  stays exact; the absolute pin is re-measured and the growth is TODO-12.
* **`llvm-scalar-facts`.** The `_mm_cvtsd_si32` link breakage found after this
  audit was real and is fixed, and a link-level completeness gate now makes the
  whole class fail loudly instead of waiting for CI.
* **Frozen-tree run.** Belongs to CI, which is the only place it is a
  final-tree statement; the local runs recorded here are explicitly
  evolving-tree numbers.

## Handoff instructions for the next agent

1. Record the exact final commit and toolchain environment beside every result. Never substitute an earlier evolving-tree CI run for final-tree acceptance. Build compiler and linker with `fastbuild` and active swap; use strict warning handling.
2. Recheck the failing-gate inventory above; restore pinned oracle prerequisites from verified caches without bypassing required channels. Diagnose the hot-web mechanism failure using complete allocation-owner intervals, whole-web victim membership/cost, ABI hints, and loop-carried protection. Permit reuse only when protected values remain assigned and disjoint. Add refusal regressions for protected coalesced members; do not remove legality guards to manufacture an assembly difference.
3. Run `scripts/ci_local.sh --fast` on the frozen tree; retain the exit status and gate summary. Also run strict all-target Clippy, formatting checks, and targeted negative oracle/checker controls. Investigate every failure instead of rebaselining it.
4. For runtime acceptance, compare identical compiler configurations and output on a quiet, pinned machine using randomized paired samples above 50 ms. Report dispersion and uncertainty. Preserve raw data. Obtain ICX only from an authorized stable/cache source if a three-vendor comparison is required.
5. Prioritize callback devirtualization/clone specialization and direct min/max lowering using actual affected workloads. Treat sibling calls and public vector ABI as independent projects with interop and refusal tests. Preserve NaN, signed-zero, rounding, exceptions, and calling-convention semantics.
6. Fetch/rebase current main on this session branch, regenerate the complete `ms178-1.patch`, and check exact application against that base. Keep descriptive recoverable snapshots after each validated repair, push this branch only, and update PR #918 without adding a validation section to its description.
