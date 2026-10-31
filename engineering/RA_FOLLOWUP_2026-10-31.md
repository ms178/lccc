# Register allocator follow-up — 2026-10-31

## Scope and evidence

This session advanced the x86 allocator research programme's measurement
infrastructure. It did **not** change allocation, spilling, register policy,
wide-integer lowering, or emitted machine code. Do not describe these changes as
an allocator optimization or a performance win.

Working-tree base: `935f243273329e168aa80a29f57e54bbbd0b64cf` (the checked-out
`origin/main` at session start). Current container has GCC 12.2.0 and Python
3.11.2. `clang`, `rustc`, and `cargo` are not installed in `PATH`. The
repository's `scripts/ensure_swap.sh` successfully activated an 8 GiB `/swapfile`
for this container; current swap usage was zero. `scripts/build_lccc_fast.sh`
was attempted as required and failed before compilation because `rustc` and
`cargo` are absent. No lccc binary was available, so no lccc code-generation comparison was
possible and `ci_local.sh --fast` could not pass its build gate. There is no Intel
14700KF or usable PMU evidence in this VM.

The latest-main fetch returned the same commit as the session base; no rebase
was necessary. No benchmark timing or allocator-quality result is claimed.

## Accomplished

### RAP-01 — reproducible corpus and scoreboard foundation

* Added `tests/ra/gen_rastress.py`, a deterministic generator for 20 independent
  kernels: the 10 assessment seed shapes and 10 additional workload shapes.
  Generated sources stay outside Git by default; the Python generator is the
  corpus source of truth.
* Added `tests/ra/spill_compare.py` to compile distinct functions to assembly
  for LCCC/GCC/optional Clang and report instruction, stack operand, memory,
  call, push/pop, loop-local push/pop-proxy, checksum, and compiler wall-time
  fields as text/JSON/CSV.
* Added explicit `holdout`, `tuning`, and `both` suite selection. Default is
  holdout, so it remains easy to evaluate a frozen candidate separately from
  the tuning corpus. The existing benchmark C files are included via glob, not
  copied or rewritten.
* Added checksum checking against GCC `-O0`; the deterministic corpus is
  checked in the fast CI lane by `tests/ra/test_harness.py`.

**Validation performed:** generator repeated twice and byte-compared; all 20
kernels compiled and ran under GCC `-O0`, `-O1`, `-O2`, and `-O3`; every output
checksum matched the `-O0` reference. The final scoreboard self-test completed a
GCC-only `-O1/-O2/-O3` run for all 20 holdout sources and wrote 174
function/compiler/level rows; its `tuning` O2 census parsed 110 translation
units into 616 rows after the data-section parser fix. These runs use GCC 12.2.0
on both labels only to test the harness, not as a cross-compiler result. Reports
carry source SHA-256, compiler version, target flags, and host platform.

### RAP-02 — partial reporting only

The scoreboard supplies deterministic *assembly-derived metrics* in a
per-function report and records the exact command plus a variable compile
elapsed-time sample. It cannot see allocator values,
register classes, live-range segments, MAXLIVE, demotion causes, rematerialized
values, or weighted spill cost. Consequently it cannot certify an avoidable
spill, and the RAP-02 internal report and `fft4_stage` certificate remain open.

### RAP-05 — diagnostic coverage only

The report includes a lexical direct-backedge census for push/pop in a loop and
unit tests it against an assembly fixture with prologue saves and loop-local
push/pop. This exposes the `bn_mul`-style symptom, but is not a CFG-complete
detector and does not replace a push/pop temporary with a parallel-copy
sequentializer. The x86/i686 backend algorithms still need independent audit,
cycle/permutation tests, and an emitted-assembly gate.

### RAP-08 — evidence protocol, no cost constants

Documented a target-hardware measurement protocol and explicitly prohibited
turning VM timings into Raptor Lake spill costs. No benchmark constants were
measured or added.

### RAP-10 — compile-time observation, no ratchet baseline

Translation-unit wall time is emitted and an opt-in `--compile-budget-seconds`
limit exists. A stable budget needs repeated measurements on a pinned host and
must be separated from compiler cold-start/cache effects; no global threshold
is enabled.

## Validation and red-team notes

* `python3 tests/ra/test_harness.py --full` — PASS (20 kernels, GCC O0/O1/O2/O3,
  deterministic source bytes, assembly-parser expectations).
* `python3 tests/ra/spill_compare.py --gcc gcc --lccc gcc --opt-levels 1,2,3
  --check --json /tmp/ra-report.json` — PASS; this deliberately uses GCC on both
  sides solely to test harness mechanics.
* `bash -n scripts/ci_local.sh`, Python byte-compilation, `scripts/check_ci_gate_parity.py`
  (with PyYAML in a temporary validation venv), and the full RA harness tests
  pass after the red-team fixes.
* `-fsanitize=undefined,float-cast-overflow` passed on all 20 fixed-seed kernels
  at GCC `-O2`; a separate AddressSanitizer + UBSan pass also passed all 20 at
  GCC `-O1`.
* Red-team parser fixtures now cover `.data` globals, `.pushsection/.popsection`,
  SIB operands, `lock` prefixes, x87 `%st(n)` register operands, the exact
  stack-reference exclusions, and loop-local push/pop. Reports hash each input
  source and record compiler version/flags; empty function parses fail closed.
* The generator removes only stale files bearing its own marker, atomically
  replaces its own generated outputs, and refuses to overwrite unmarked user
  sources; regression tests verify both preservation and refusal behavior.
* `scripts/build_lccc_fast.sh` — BLOCKED: no `rustc` / `cargo` binaries.
* `PATH=/tmp/lccc-validation-venv/bin:$PATH ./scripts/ci_local.sh --fast` — the
  Python/YAML, shell, and initial repository gates passed; the run then stopped
  at `build` because neither `rustc` nor `cargo` is installed. As designed, it
  refused to continue without `target/fastbuild/lccc`, so the newly added gate
  could not be reached through this runner. `tests/ra/test_harness.py --full`
  was run directly and passed. Do not report `ci_local.sh --fast` as green.
* The initial differential sweep found undefined behavior in the interpreter's
  stack updates (unsequenced `sp` read/write and signed overflow); those were
  changed to unsigned arithmetic with explicit sequencing. The subsequent
  red-team UBSan pass found negative floating-point results converted directly
  to `uint64_t` in the FFT, matrix, and geometry checksums. These now convert
  first to bounded `int64_t` values; the complete fixed-seed corpus passes GCC
  `-fsanitize=undefined,float-cast-overflow -fno-sanitize-recover=all` at `-O2`.
  Both the fast and full harness modes now execute that sanitizer check.
* The operation counts are deliberately labeled proxies. The loop finder only
  recognizes direct lexical backedges and the load/store classifier is an
  assembly heuristic. Neither is a compiler correctness checker nor a runtime
  performance oracle.
* No third-party source was copied. These are synthetic algorithm shapes, not
  byte-for-byte gzip/zlib-ng/expat/SQLite/kernel/glibc workload extracts.

## Ordered follow-up backlog

| Priority | Work | Evidence / acceptance | Dependencies |
|---|---|---|---|
| P0 | RAP-12 internal post-allocation definite-assignment + value-flow checker | Mutation tests (missing def, swapped homes, missing reload) caught; clean tuning corpus, holdout, and fuzz | Need machine CFG/instruction semantics and exact location maps |
| P0 | RAP-02 internal allocator report + exact MAXLIVE certificate | Deterministic function/class rows and `fft4_stage` avoidable/leaf-policy certificate | Extend existing liveness report plumbing without assembly scraping |
| P0 | Run lccc-vs-GCC/Clang on `--suite both` and baseline machine-readable reports | Verify checksums at O1/O2/O3; separate per-suite output; record compiler revisions | Need lccc fastbuild and pinned reference compilers |
| P0 | Reproduce cross-vendor compiler matrix / Godbolt oracle | Current GCC 12.2 is not the prescribed latest compiler; add ICX/ICC only when available | Oracle/toolchain credentials and upstream revision checks |
| P1 | RAP-05 pure parallel-copy sequentializer and backend integration | Exhaustive permutations (2–6 moves, cycles, memory/immediate sources); no loop temporary push/pop; regression reproducer | Define reserved scratch contract per emitter/class |
| P1 | RAP-04 wide integer legalization | Exhaustive small-width/reference tests, random u128 sweeps, i686 i64 pair tests, `bn_mul` code-shape | Preserve `cmpxchg16b` atomics and explicit flag dependencies |
| P1 | RAP-03 leaf/call-free register policy | Investigate reserved register census; invariant not reloaded inside `fft4_stage`; no holdout regression | RAP-02 report; actual assembler/runtime tests |
| P1 | RAP-08 target spill-cost microbench and RAP-06 callee-save policy | P-core pinned PMU raw pairs; distinguish register/register, forwarding, and memory-ALU costs | i7-14700KF; PMU access; CFI validation for shrink-wrap |
| P1 | Workload-derived stress extension | Review `ms178/archpkgbuilds/packages` and add small, licensed, reproducible kernels from gzip, zlib-ng, expat, SQLite, kernel and glibc | GitHub source/licensing audit; avoid vendoring whole packages |
| P2 | RAP-07 shadow phase | First compare estimated global spill cost vs legacy without emission; only proceed if headroom is real | RAP-02, RAP-08, RAP-12 |
| P2 | RAP-10 calibrated compile-time budget | Callgrind/inst-count baseline under repeatable settings; ratio gate, not host-time gate | Compiler available; benchmark corpus stable |

## Next experiment protocol

1. Build LCCC only through `scripts/build_lccc_fast.sh` (fastbuild/O1, fixed
   `-j2`) after restoring the Rust toolchain; keep the active swap file.
2. Run `tests/ra/test_harness.py --full`, then `spill_compare.py --suite
   tuning` to establish baseline. Archive JSON with compiler versions and flags.
3. Freeze changes selected using the tuning corpus, then run `--suite holdout`
   at O1/O2/O3, first checksum validation, then shape deltas. Review worst
   regressions individually; do not hide them with a geometric mean.
4. Do not enable allocator policy changes until the internal checker or a
   strong equivalent catches allocation mutations. The next highest-value
   implementation item is RAP-02/RAP-12 instrumentation, not another heuristic.
5. On target hardware, collect paired timings and perf counters for `bn_mul`,
   `fft4_stage`, and the holdout worst regressions. Publish raw pairs before
   claiming any speedup.
