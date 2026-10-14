# PR827 versus PR826: upstream reconciliation and causal red-team audit

## Verdict and scope

Keep PR826's tested bounded-pair/diagnostic/descriptor hardening, retain PR827's
correct C-example documentation fence, and fix the instruction-displacement gap
that **both** versions retained. Do not choose by green badges or call either a
universally superior compiler. The current change is correctness hardening, not
an optimizer, vectorizer, register-allocation or CPU-model performance victory.

The latest merged upstream PR is **#827**. Its metadata/files were retrieved
through GitHub's API; actual immutable trees were compared locally. Its entire
tree differs from PR826's original pre-audit implementation in **one file, four
added/two removed documentation lines**. There is no independent competing
optimizer design in that diff. Seven merge conflicts were reconciled on the
existing branch without changing branches. All upstream history is incorporated.

### What I agree with

- Input-contribution width validation before GOT/TLS scanning and mutation is
  necessary; merged-buffer capacity alone cannot protect the next contribution.
- Unknown active relocation kinds must not disappear merely because output is
  shared. Symbol indices and offset/address/host-size conversions need checks.
- Hold the profile-file lock through final buffered flush and close. Stop after
  failed acquisition; preserve the application's result.
- The `text` fence around a C declaration in Rust documentation is the right
  upstream fix. It avoids rustdoc interpreting illustrative C as Rust.

These production choices are largely shared foundations, not evidence that one
agent independently beat another's optimizer.

### What I disagree with or must correct in both audits

1. **PR827's numeric-range claim exceeds its implementation.** Its branch/JAL/
   compressed-branch arms cast to u32 and mask immediate bits without range or
   low-bit checks. Byte extents are not displacement representability. PR826's
   prior audit also failed to close this gap. New independent ISA decoding and
   real-linker controls specifically cover it.
2. **Its ULEB application is still the original intermediate SET then SUB.** A
   large absolute label can overwrite neighbors even if the final difference
   fits; a short final result destroys reserved padding. PR826 models the pair
   and immutable original reservation instead. Successful operations are bounded
   and nonallocating; unsupported overwide encodings are diagnosed explicitly.
3. **A sticky stream flag is not an errno snapshot.** PR827 still calls perror
   after ferror. PR826's generic diagnostic avoids inventing an errno cause.
4. **A pre-acquisition marker does not prove kernel contention.** PR827's writer
   A need not remain alive until B acquires. Our own earlier pre-call-marker
   version was also insufficient. The current oracle makes A wait for B's real
   nonblocking acquisition refusal, before allowing A to continue.
5. **Descriptor lifetime extends beyond one close when aliases exist.** PR827's
   fopen mode does not request close-on-exec. The Linux descriptor request and
   actual FD_CLOEXEC oracle check in PR826 avoid ordinary exec inheritance. This
   does not certify fork-only termination, arbitrary descriptor duplication, or
   all libc implementations.
6. **Reported counts must have units.** PR827's script contains 86 fixture
   definitions and makes 172 executable/shared invocations, not 172 distinct
   fixtures. Its original PGO grid has 60 checks. Its tests do not include the
   claimed exhaustive addend/displacement or stdio-global-close scenarios.
7. **Our own tests are not automatically sound.** The newly added C reference
   driver initially failed Ubuntu's fortified warning policy by ignoring write's
   return. Explicit FORTIFY_SOURCE=3 reproduced it. We checked the return instead
   of suppressing the warning; compiler traces are now bounded API annotations.
8. **Positive coverage was over-described.** The previous 5/39 positive-kind
   count includes NONE; it is not five storage kinds plus NONE. The new count is
   9/39 including NONE and RELAX, not all-kind semantic coverage.

## New displacement design

`src/backend/riscv/linker/displacement.rs` defines four typed immediate formats.
It computes RV64 arithmetic modulo XLEN and interprets the result as signed,
then validates the discarded low bit and exact signed range **before patching**:

| Encoding | Bits including implicit zero | Valid byte displacement |
|---|---:|---:|
| BRANCH | 13 | -4096 through 4094, even |
| JAL | 21 | -1048576 through 1048574, even |
| RVC_BRANCH | 9 | -256 through 254, even |
| RVC_JUMP | 12 | -2048 through 2046, even |

This prevents both silent truncation and debug-only host arithmetic overflow.
It deliberately does not reuse these rules for AUIPC/LO12/CALL/TLS forms whose
composition and relaxation constraints differ. Inventing one generic signed
range rule for those encodings would add unvalidated compatibility risk.

The module's tests use an independent table of ISA widths, exhaust **1,054,976**
representable displacements, reject range neighbors/odd offsets, and cover XLEN
wrap, signed-boundary crossings, and extreme addends. The existing ULEB suite's
90,115 stated cases remain. Tests import actual production modules. Two unused,
unbounded legacy ULEB helpers were removed after repository-wide reference
inspection; there is no second dormant oversized-shift/truncation API to revive.

The integration generator adds **128 fixtures**: 24 accepting and 8 rejecting
cases for each of four encodings. Symbols and relocation fields stay within their
input contribution even for negative controls; rejection cannot be credited to
an unrelated invalid-section/extent check. Compressed fixtures declare RVC.
Output verification independently decodes the ISA immediate and checks opcode/
register preservation, complete prefix/suffix preservation, and neighboring data.

Totals are **233 fixtures: 115 accept, 118 reject; 466 executable/shared
invocations**. The 9 positive kind numbers are 0, 2, 16, 17, 44, 45, 51, 60, 61.

## Executed evidence versus scheduled measurements

Raw local results are in `engineering/evidence/pr827-comparison/`.

- **LLD 22.1.8: 230/230 accepting invocations passed**, including field semantics
  and sentinels. **64/64 new numeric-range/alignment controls were rejected.**
- On the entire strict contract, LLD agrees in **383/466** invocations. It accepts
  **79/100** fixed-width negative controls and **4/16** ULEB negative controls.
  This remains disagreement evidence, not permission to relax our gate. Compiler
  version and compressed-input flags changed, so this is not a controlled
  version-to-version improvement claim over the earlier LLD experiment.
- GCC 12.2 and Zig's Clang 22.1.8 each passed all **9 reference-harness tests**.
  Those are host-oracle tests, not local changed-LCCC integration results.
- Fresh codegen-oracle comparability logic **14 tests**, oracle-delta logic
  **7 tests**, benchmark-ranking logic **1 test**, workflow shell checks and
  **218-command/22-contract** core CI/local parity passed.
- Encoding-diff self-tests on the unpinned local GNU objdump 2.40 ran 73
  tests: 71 passed, two APX selector cases returned UNVERIFIED-BYTES. An actual
  four-byte APX disassembly produces `(bad)`. This is not evidence of a compiler
  encoding bug and is not relabeled a pass. The hosted core suite requires the
  repository's provisioned 2.47 oracle pair. No fallback or skip was added.
- A fresh compiled perf_event_open probe again returns ENOENT for hardware
  cycles. There is no local hardware-counter basis for a speedup claim.
- Local Rust is unavailable; no local Rust/Cargo compilation is claimed. GitHub's
  latest Rust release API reports **1.99.0**, published 2026-10-01. Hosted jobs
  explicitly install Rust/Cargo/rustfmt/Clippy 1.99 and require GCC 16.2.0.
- Local **8 GiB swap is active**; observed use was zero. We do not create
  artificial memory pressure merely to claim it was exercised.

The prior PR826 production head passed its exact-toolchain 210-relocation/
72-PGO/45-workload-O2-and-O3 gates before reconciliation. Those successes are
historical, not validation of the new displacement implementation.

### Three-way compiled comparison (results pending at publication)

`tests/linker/check_pr827_comparison.sh` builds immutable PR827, pre-range PR826,
and candidate sources into separate binaries without checking out another
branch. The scoped PR826 hosted research job executes it; local researchers can
run the identical entry point with the named toolchain/tools available. This is
not a new universal requirement that future optimizer PRs retain old codegen.

`compare_pr827.py` applies the **same 466-invocation contract** to all three
actual compiler builds and emits per-group counts, binary/report hashes, errors
and source identities. It separately probes actual generated PGO code for
FD_CLOEXEC and zero/stale-errno diagnostics. The diagnostic-only oracle build
isolates errno behavior from a baseline's known descriptor failure; the normal
72-check candidate oracle never enables that compile-time isolation switch.

The job also runs **strace** and **Valgrind** on actual generated programs. These
answer syscall ordering and memory-error questions, not whether an in-bounds
relocation changed the wrong semantic field. Independent byte/ISA checks answer
that question. No simulation count is called hardware-cycle performance.

It reuses `run_benchmarks.py`'s source registry, exact extra flags (including
scaled/match-heavy variants), and compile API for **45 workloads × O2/O3 × three
compiler builds = 270 object compilations**. `asmdiff.py` supplies object
inspection, and exact object hashes test whether non-PGO codegen changed at all.
An identity difference requires inspection; identity is not proof of correctness
or a measured runtime ratio. Current strict runtime corpus screening remains a
separate gate against GCC/Clang. Missing tools, empty or partial comparisons and
optimized-Python assertion disablement fail closed.

Do not replace pending compiled measurements with predicted pass/fail totals.
The workflow's API-readable causal summary and raw artifacts will supply them.

The first new hosted attempt executed all 11 direct production-module tests
successfully, then failed formatting. Actual Rust 1.99 formatting annotations
were applied, not an approximate style rewrite. The new comparison container
also lacked curl, which the inspected rust-toolchain action invokes to bootstrap
rustup; its explicit prerequisite list was corrected. Both are our harness/setup
issues. The final push must execute its own corrected jobs.
A subsequent actual bootstrap trace identified Git's dubious-ownership rejection
in the container. Archive/rev-parse calls now trust only the known checkout with
per-command safe.directory; there is no global wildcard exception. The trace,
not a guessed exit-code interpretation, motivated this correction.

## Performance laboratory interpretation

The runner registers **45 workloads / 43 distinct sources**. The directory's 58
C files include 15 unregistered studies; those are not falsely counted as timed
or runtime-tested workloads. The older "39 corpus" label is not this checkout's
inventory.

- `codegen_oracle.py`: static whole-function load/store/branch/instruction and
  stack-reference screening, resolved remote identities; ranking mode is a
  survey and can exit zero despite compile failures. Inspect its error records.
- `oracle_delta_gate.py`: baseline-relative structural metrics with tolerances;
  an unreachable remote is a recorded skip unless strict. A green job is not
  necessarily a live all-competitor run or runtime superiority.
- `hot_loop_metric.py`/`loop_density_oracle.py`: CFG-derived innermost-loop
  instructions per byte, with unknown/composite loops withheld. Matching names
  or largest steps do not establish equivalent phases/work or hardware speed.
- `tight_loop_oracle.py`: encoded-span/alignment classification, not throughput.
- `tune_oracle.py`: observable CPU-model instruction-selection choices; a model
  difference must be isolated and tested, not automatically called a win.
- `asmdiff.py`/`encdiff.py`: whole-object/encoding semantics, with round-trip
  verification before shorter encodings count as improvements.
- `perf_ab.py`/`bench_kernels.py`: equivalent-work, output-verified isolated arms
  and interleaved runtime measurements. VM wall clocks and simulated/cache
  counts are not target-CPU PMU evidence.
- `run_benchmarks.py`/`bench_rank.py`: strict output screening, paired samples,
  per-workload intervals and regressions; a geomean cannot conceal bad workloads.

The shared oracle JSON contains six source reports, not a whole 45-workload
fresh measurement. For example its SipHash whole-TU snapshot has 846 LCCC versus
308 GCC static instructions, 54 versus 17 explicit stack references. Clang/ICX
have six calls versus LCCC/GCC's ten, so even this scope is not uniformly matched.
These counts are neither runtime ratios nor allocator-spill counts. ICX's
recorded ID is a moving latest alias, not a certified release.

Three direct parser counterexamples also distinguish the test harnesses:
missing function after a new record header, negative count, and extra record
fields are all accepted by PR827's parser and rejected by the current parser.
These are actual Python execution results, not compiled-runtime claims.

The checked-in older CE/density research is shared by both trees, not new
independent evidence that PR827 beats PR826. We have not conducted a fresh
all-vendor remote survey or proven a speedup. No optimizer/tuning changes are
justified by these correctness experiments.

## Remaining axes / merge discipline

The new tests are exhaustive over the stated immediate-value domains, **not over
all compiler programs, ELF semantics or concurrency schedules**. Numerical and
positive coverage still needs AUIPC/HI-LO/CALL compositions, GOT/PLT/TLS,
preemption/binding variants, separate-object/archives, NOBITS, and architecture
flags. Archive parse-error suppression and shared TLS limitations are inherited.
PGO multi-TU/DSO/value-profile, fork-only/descriptor aliases and multi-threaded
termination remain unproved; partial-record failures are not rolled back.

Merge only after the reconciled final head's required checks and causal candidate
contract succeed. No 10/10 editorial grade, universal dominance claim, or army
of engineers' approval is substituted for those results.
