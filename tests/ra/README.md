# Register-allocation research harness

This directory is the repeatable, compiler-independent scaffolding for the
register-allocator assessment. The generated source corpus is deterministic and
kept out of the repository; the generator is its checked-in source of truth.

## Suites and holdout discipline (RAP-01)

`gen_rastress.py` builds 20 standalone C translation units:

* **Tuning-adjacent seed shapes:** bit reader, five-limb multiply/reduce,
  radix-4 FFT butterfly, ten-limb field multiply, BLAKE2b-style rounds,
  bytecode interpreter, MD5-shaped rounds, SHA-512-shaped rounds,
  Keccak-shaped lanes, and schoolbook 32×32 `u128` bignum multiply.
* **Holdout extensions:** tree walker, regex-NFA step, LZ77 match search,
  three-stream CRC32C-shaped folding, Montgomery reduction, 4×4 matrix
  inversion, radix histogram/scatter, SipHash-shaped streaming, aggregate
  return/geometry transform, and recursive-descent parser fragment.

The names describe stress shapes; these are synthetic kernels, not copied
third-party implementations and not cryptographic test vectors. The holdout is
evaluation-only: select hypotheses on the existing tuning corpus, freeze code
and knobs, then run the holdout. Do not use holdout results to choose thresholds
or tune allocator modes. Keep the two suites and their outputs separate.

```sh
python3 tests/ra/gen_rastress.py --output-dir /tmp/lccc-ra-holdout
python3 tests/ra/spill_compare.py --suite holdout --lccc target/fastbuild/lccc \
  --gcc gcc --clang clang --opt-levels 1,2,3 --check \
  --json /tmp/ra-holdout.json --csv /tmp/ra-holdout.csv
python3 tests/ra/spill_compare.py --suite tuning --lccc target/fastbuild/lccc \
  --gcc gcc --clang clang --opt-levels 2 --json /tmp/ra-tuning.json
```

`--check` is an executable checksum test: selected translation units must have
an entry point and produce deterministic output. The tuning `kernel_corpus`
files are function-only codegen probes, so use their function-specific driver
or run that suite without `--check`; the tool reports a link failure instead
of silently marking such a source as correct.

For an end-to-end deterministic generation, parser, and checksum smoke test:

```sh
python3 tests/ra/test_harness.py             # GCC O0/O2
python3 tests/ra/test_harness.py --full      # GCC O0/O1/O2/O3
```

The fast CI lane runs the smoke test. The full sweep is an explicit research
command because it compiles every generated kernel repeatedly. Both modes
compare checksums against GCC `-O0` and run the fixed-seed kernels under GCC
UBSan (including float-cast-overflow) at `-O2`; these screens are not a proof
of C semantic correctness. Run fuzzing and targeted reference models for
aggressive transformations.

## Scoreboard semantics and limitations (RAP-02 / RAP-05 / RAP-10)

`spill_compare.py` emits one record per compiler, optimization level, source,
and function. It reports:

* instruction count, stack-based operands, loads/stores, calls, and push/pop;
* `loop_push_pop`, the number of push/pop instructions lexically covered by a
  direct backward branch interval;
* explicit-operand load/store classification (including SIB addresses); implicit
  string/stack memory effects are not reconstructed;
* compile wall time for the translation unit and the exact compiler command;
* optional checksums and JSON/CSV outputs.

The stack count deliberately excludes every `push*`, `pop*`, `call*`, and
`lea*` mnemonic, following the assessment's proxy definition. AT&T operands are
split with balanced-parenthesis handling so SIB forms are not mistaken for
separate operands; common instruction prefixes (for example `lock`/`rep`) are
normalized before counting. Load/store totals cover explicit memory operands;
implicit string/stack accesses are not reconstructed. The loop push/pop field
is only a *screening heuristic*: irreducible loops, indirect branches, or
non-local control flow can escape its lexical approximation. Review assembly
before treating any count as proof. Stack operands also include legitimate
local arrays and ABI homes; a count is never a standalone performance verdict.

Compile wall time is recorded for RAP-10 exploration. `--compile-budget-seconds`
can be used as an opt-in, host-specific ratchet; do not check in a threshold
until it is based on repeated measurements on a pinned host. The harness has no
internal allocator instrumentation and therefore does **not** claim the
MAXLIVE/avoidable-spill certificate required by RAP-02. Nor does a census
implement RAP-05's parallel-copy rewrite; it helps expose a class of bad output.

## Hardware measurements (RAP-08)

The development VM currently has no reliable access to Intel Raptor Lake
hardware counters. Do not publish cycles, IPC, or spill-cost constants from
this VM as target results. For a real i7-14700KF measurement:

1. record CPU model/stepping, microcode, kernel, compiler revision/flags,
   linker, libc, governor, SMT/P-core affinity, and temperature/frequency;
2. pin to one P-core (`taskset`) and run each variant in balanced AB/BA order;
3. collect at least 30 paired samples after warm-up; report median, minimum,
   dispersion, and outliers, and preserve raw pairs;
4. use `perf stat -r 30 -e cycles,instructions,branches,branch-misses,cache-references,cache-misses`;
   record permission failures and never replace missing PMU data with
   `perf`-looking wall-clock estimates;
5. measure dependent store-to-load-forwarding, independent load throughput,
   and memory-operand ALU cost separately, with aligned same-size accesses and
   explicit size/line-crossing negative controls.

Wall-clock scoring on a shared VM is for gross screening only. Static code
shape, checksums, and compile-time observations are reproducible there; they do
not establish generated-code runtime superiority.
