# PR814 red-team audit and repair — 2026-10-14

## Scope, base and acceptance

This cumulative patch preserves PR814's IV/address strength reduction and x86
peephole work while repairing the semantic and validation defects encountered
in review. It is rebased on `ms178/lccc` main
`3ff36e8592b0d0e26645bf37d6685395b1a6eca5` (PR826, after PR827).
The base was fetched again before freezing this report for Fast CI.

**This is not a proof that the whole compiler is bug-free, nor a claim of
universal performance superiority.** The whole-codebase audit remains an
ongoing program. New upstream linker/RISC-V work was retained, not replaced
with the older PR814 tree.

Final acceptance requires `bash scripts/ci_local.sh --fast` on the exact
frozen tree, including rustfmt and strict clippy, followed by another upstream
fetch and the gated snapshot script. The authoritative tree/base/patch hash
and CI result are emitted into the delivery receipt (`LCCC-DELIVERY.json`,
`LCCC-DELIVERY.md`, snapshot ledger and Fast-CI log in the delivery artifacts),
**after this document is frozen**. Interim snapshots explicitly say UNGATED;
they are recovery points, not final CI approvals. Full/slow CI is deliberately
left to GitHub, as requested; a local fast pass is not full CI equivalence.

All LCCC and LCCC-ld builds use the `fastbuild` preset, compiler optimization
level 1, no LTO, and exactly two jobs. Debug information was disabled on this
1.9-GiB VM. Active 4-GiB swap was verified. No compiler release/LTO build was
used. Builds and workloads use the filesystem with the available workspace
capacity. Oracle linkers have their own documented build configurations.

## Repairs and soundness boundaries

| Area | Finding and repair | Evidence / boundary |
|---|---|---|
| Shift peepholes | Debug directives could change the matching window and generated behavior. Match across transparent debug metadata, but retain opaque directives and register/flag barriers. | Pre-fix debug-window failures and fannkuch parity failure retained; repaired 58-program debug parity passed. |
| RIP-relative address CSE | An expression involving the assembler location counter `.` is not a position-independent identity at different instruction positions. Refuse that reuse. | Focused positive and negative unit tests. |
| Store forwarding | VEX masked stores are implicit memory-write barriers; a later load cannot reuse an earlier value through them. | Regression tests for masked-write variants and ordinary safe cases. |
| Dead moves and FP liveness | Pinned moves, partial vector writes, live upper lanes and opaque directives must not be treated as full dead definitions. Keep uncertain cases conservatively. | Unit coverage; new FP tests were not recorded failing before repair, so no red-before-fix claim is made for them. |
| IVSR | Preserve vector `(base,index,disp)` use collection, global-base coalescing, copy identity and recurrence work. Apply the SIB cost rule only to x86-64/i686. | Avoids transferring an x86 addressing cost assumption to AArch64/RISC-V; existing target regression gates retained. |
| Min/max reduction reach | Early IR contains `Load -> Copy -> Select`; exact value-ID matching missed a legal min/max, and a later I64-IV form was correctly refused by the I32 remainder emitter. Follow **Copy only** for element identity in the matcher and legality check. | 20-shape gate passes again; differential execution at multiple optimization levels, scalar refusals and i686 checked. Cast, foreign value and cyclic Copy negative unit cases added. |
| Reduction safety | Keep signed I32 exclusive-bound remainder restrictions and exact exit/branch matching. Preserve side-effect, volatile-load, nonzero-start, seed-edge and escaping-value guards. | Do not remove a legality guard merely to get packed assembly. Inclusive-bound refusal now reports the established diagnostic at the earlier rejecting stage. |
| Map address arithmetic | The original complete-domain no-wrap check wrongly rejected ordinary pointer-width I64 maps. Address arithmetic may wrap **modulo pointer width**, but narrower intermediates still need range proofs before widening. | I64 two-lane AVX2/SSE2 shape and runtime gate passes; U32 narrow-overflow refusals retained and tested. |
| Map scalar remainder | A wrapped byte IV cannot recover a full-width element index by right shift. Only reuse the byte IV after its exact bound was proved; otherwise retain the element counter and original remainder resume. | Concrete high-bit counterexample and full-width/narrow-domain unit cases; no assumed array-size fact absent from IR. |
| Environment hygiene | Reuse the vectorizer's already-read explanation flag rather than query it again. | 151 reads / budget 151; budget was not raised. |
| TLS linker fixture | GCC 16.2 hoisted a local-dynamic module-base lookup before an inline `arch_prctl` installing FS. Both LCCC-ld and bfd then faulted at the same `%fs:0` read. | Memcheck identified address 0. Split TLS accesses into a separate non-LTO TU, reached after FS setup; use an assembly process entry with correct C-call stack alignment. Check required TLS relocations rather than silently passing if absent. No production linker workaround. |
| New assembler forms | GCC's private assembler search can bypass the pinned assembler on PATH. Assemble the two APX/MOVRS fixtures directly with `as --64`. | Both previously unsupported fixtures now run under GAS 2.47 rather than skip. |
| Oracle provisioning | Upstream bfd defaults were not the native Linux configuration expected by RUNPATH/TEXTREL and standalone multiarch `-lc` tests. | Build 2.47 with new dtags and native multiarch search defaults, retaining emulation-specific lib32/lib64 handling. Store and validate the configuration marker, so an old cached binary is rebuilt. Idempotent rerun verified. |

The Copy resolver is bounded and fail-closed on overlong/cyclic chains; it does
not reinterpret a narrowing or floating-point Cast as a load identity. Extra
accumulator consumers remain refused. Address-bit preservation is deliberately
separate from element-count preservation: a proof suitable for a pointer is
not automatically suitable for a loop bound or remainder counter.

## Executed validation before final Fast CI

Evidence root: `engineering/evidence/pr814-redteam/`.

- Current-source warning-as-error fastbuild succeeded.
- Rust library suite: **4,687 passed, 0 failed, 8 ignored**. The ignored tests
  remain visible; they are not counted as executed coverage.
- The previously failing targeted min/max, FMA-matcher, I64-map and environment
  hygiene gates pass. Earlier first Fast CI was **240 passed / 5 failed /
  5 skipped**; those results must not be presented as green.
- Full strict linker suite with current GCC 16.2 and pinned bfd/lld/mold:
  **303 passed, 0 failed, 0 warnings, 0 skipped**, including i386 and kernel
  relocation-tool checks. An early temporary GCC wrapper placed its `-B`
  oracle prefix before the suite's LCCC linker shim and invalidated that run;
  corrected ordering preserves caller `-B` priority. Only the corrected run
  is published as linker evidence.
- Six current generated programs passed Valgrind Memcheck: gzip CRC32,
  zlib-ng Adler32, XML scan, SQLite varint, nbody and histogram.
- **Expat 2.8.5 real project:** SHA-pinned complete configured C library,
  examples, xmlwf and upstream `runtests` built with LCCC and GCC 16.2 at O2;
  one CTest driver passed for each. Test identity and CMake feature sets agree.
  Five canonical XML cases and three malformed XML rejection cases agree.
  One CTest driver does not mean only one internal Expat assertion, nor do we
  invent a larger assertion count not reported by that driver.
- **zlib-ng 2.3.3 real project:** pinned source plus the pinned archpkgbuilds
  patch; all **69 CTests** pass. Deterministic binary roundtrips at levels
  1, 2, 6 and 9 and independent system-gzip stream checks pass. AVX-512,
  AVX512VNNI and VPCLMULQDQ are intentionally off in this configuration.
  This is bounded correctness coverage, not a full package/ISA build matrix.

`validation/` retains both the pre-fix and fixed evidence. Workload reports
are under `expat-current/`, `zlib-current/` and `memcheck-current/`. Log suffixes
were changed to `.log.txt` where necessary to avoid the repository's ignored
scratch-log rule. Generated workload executables are not added to the patch.

## Oracle versions and static code generation

Actual resolved tools: GCC **16.2.0**, GAS/bfd **2.47.20260726** from the pinned
2.47 release archive, Clang **23.1.0** on Compiler Explorer, lld **23.1.4**,
mold **2.42.1**, ICC **2021.10.0**, and the resolved **ICX latest** channel
recorded in each CE manifest. Mold was source-built with
`-DMOLD_TARGETS='X86_64;I386'`, not the all-target template build. Wild was not
part of the requested comparison and was explicitly not provisioned.

`oracle-current/` contains the successful current-source 5-workload ×
5-compiler comparison, raw assembly, manifests and static reports. Local flags
are `-O2 -march=haswell`; remote flags are `-O2 -march=core-avx2`. These are
explicitly different spellings: ICC rejected `x86-64-v3`, while LCCC does not
accept `core-avx2`. Do not describe this as an identical-flags experiment.

| Whole source, static instructions | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX latest |
|---|---:|---:|---:|---:|---:|
| gzip CRC32 | 73 | 50 | 82 | 60 | 100 |
| zlib-ng Adler32 | 275 | 156 | 148 | 244 | 147 |
| XML scan | 221 | 172 | 222 | 322 | 327 |
| SQLite varint | 395 | 330 | 373 | 461 | 421 |
| histogram | 64 | 76 | 107 | 132 | 124 |

These are **static structure counts**, not executed instructions, cycles,
uops, timings or proof of superiority. The earlier `oracle-before/` preserves
an unsuccessful ICC flag experiment; `oracle-final/` is the prior checkpoint,
not a replacement for `oracle-current/`.

## Current Callgrind screen — useful gaps, no fake cycles

`callgrind-current/index.json` and per-kernel manifests retain exact flags,
binary/source/tool hashes, output agreement, cache geometry and all raw
counters. Raw counter files are preserved as `.cg.txt`; contents and recorded
hashes are unchanged. Paths in original manifests identify the workspace run,
not a promise that temporary binaries ship in this source patch.

| Bounded workload | LCCC Ir | GCC 16.2 Ir | LCCC/GCC |
|---|---:|---:|---:|
| gzip CRC32 | 800,966 | 808,129 | 0.9911 |
| zlib-ng Adler32 | 463,788 | 505,240 | 0.9180 |
| XML scan | 1,666,645 | 1,434,344 | 1.1620 |
| SQLite varint | 822,567 | 761,217 | 1.0806 |
| nbody | 694,930 | 532,944 | 1.3039 |
| histogram | 3,223,573 | 1,690,633 | 1.9067 |
| Linux find-bit | 5,957,942 | 5,077,233 | 1.1735 |
| glibc memcmp | 722,848 | 691,892 | 1.0447 |

All eight output comparisons succeeded. Flags are O2, x86-64-v3, `-fwrapv`
and the checked-in bounded corpus sizes. The count includes startup/runtime
and the harness. This VM has no usable PMU: **Ir means Valgrind simulated
executed instructions, not Raptor-Lake cycles or a measured native speedup**.
Callgrind cache/branch predictions are a model, not hardware event readings.
These are a cross-compiler screen, not a paired pre/post-PR causal experiment.
Small differences from the earlier run may reflect environment/runtime paths;
they are not attributed to an optimization.

The histogram's smaller static code is demonstrably not a performance win by
this metric. Attribution puts **3,082,018 Ir** in LCCC `main` versus
**1,541,932 Ir** in GCC `main`: the main-loop work, not loader noise, dominates
the gap. Investigate initialization/vectorization and the indexed histogram
loop separately before changing the recurrence or claiming a win.

## Reproduction and delivery discipline

1. `bash scripts/ensure_swap.sh`; restore the stable Rust toolchain and multilib.
2. `CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=1 bash scripts/build_lccc_fast.sh`.
3. Run `tools/linker/setup_oracles.sh` with `WITH_WILD=0 JOBS=2`; provision GAS
   using `scripts/ensure_gas_247.sh`. Use the resolved pinned tools, not just
   version-looking directory names. Preserve any caller linker-shim priority.
4. Run `scripts/ci_local.sh --fast` with `CARGO_PROFILE_FASTBUILD_DEBUG=0`,
   `CARGO_INCREMENTAL=1`, `CI_LOCAL_JOBS=2`, and the pinned tools on PATH.
5. For instrumentation use `scripts/callgrind_corpus.py` and the workload
   `run.sh` scripts, not a homemade timing loop. Every performance comparison
   first requires matching outputs and flags/sizes recorded in its manifest.
6. Fetch main again. If it moved, rebase, rebuild and repeat relevant tests and
   Fast CI on the new exact tree. Use `scripts/lccc-snapshot.sh` for the final
   gated atomic patch, numbered checkpoints, format-patch series, source
   archive, bundle, ledger and patch-application check.

## Follow-up priorities and residual risk

1. **Hosted slow CI:** debug-assertion builds, full SSA regression corpus,
   full benchmark-output oracle and remaining slow gates must run on GitHub.
   The eight ignored library tests and architecture/ISA exclusions remain
   explicit gaps, not waived correctness requirements.
2. **FP/ABI audit:** implicit wide-vector returns and complete xsave/fxsave
   interactions are not exhaustively proven by the liveness tests. Expand
   ABI-boundary and partial-register adversarial coverage before relaxing any
   conservative barrier.
3. **Map domains:** retain the separation between modulo address arithmetic
   and exact counters. Future I64/unsigned reduction remainders need their own
   typed lowering and tests; do not lift the current I32 guard by analogy.
4. **Performance work:** prioritize histogram, nbody and Linux find-bit using
   function/loop attribution, assembly and Callgrind. Then use repeated,
   interleaved, output-checked native measurements on a suitable host, report
   sample distributions, and use PMU only where genuinely available. GCC,
   Clang and Intel compilers are comparison points, not assumed optima.
5. **Workload breadth:** complete Linux/glibc/gzip/SQLite package-scale and
   non-x86 workload validation is not established by similarly named kernels.
   Extend the archpkgbuilds corpus with pinned source, real input distributions,
   independent output oracles and bounded resource use. Preserve the measured
   Expat/zlib-ng configurations rather than generalizing to every option.
6. **Maintenance:** factor repeated Copy/alias reasoning only after type and
   dominance contracts are explicit. Keep debug metadata transparent and
   unknown assembler effects opaque. Avoid weakening shape/runtime gates or
   increasing environment-read budgets to conceal optimizer regressions.
