# PR826: empirical red-team follow-up and corrections to the first review

## Executive decision

The original PGO close-under-lock and fixed-width relocation validation choices
remain sound. The previous review's recommendation to require blanket real-linker
rejection agreement was **wrong**: an independently executed LLD accepts many
malformed inputs that this compiler must reject. Reference compilers are
comparators, not specifications. This follow-up keeps strict LCCC safety checks,
uses the oracle for well-formed cases, and records disagreements explicitly.

The branch now fixes the identified ULEB pair/field boundary defect, the parser's
RISC-V NONE storage check, and stale-errno stream diagnostics. It strengthens
absolute PGO expectations, deterministic contention, failure reporting, metadata
drift checks, and hosted toolchain validation. These are correctness and
engineering improvements, **not demonstrated benchmark speedups**.

Do not merge on a partial result or a grade. Require final-head Rust formatting,
Clippy, integration contracts, and required CI to pass. The first compiled
implementation passed both new integration gates under Debian GCC 16.2 and Rust
1.99; final formatting and additional property tests require their own head's
checks. Broad psABI/optimizer debt has not magically disappeared.

## What the first audit got right, wrong, or overstated

| Prior finding or recommendation | Empirical disposition |
|---|---|
| Unsupported shared-link relocations must not vanish | Retain the original implementation. A dynamic relocation is not emitted by the old fallback. |
| Fixed-width input boundaries should be validated before merging/scans | Retain. LLD's acceptance does not weaken this invariant. |
| ULEB intermediate encoding can cross the original field and change length | Confirmed algorithmic issue; fixed with a separate bounded production module. Production Rust module tests and valid LLD pair fixtures exercise the replacement. |
| NONE semantics were not established end to end | Correct, but the first review was too broad about unusual inputs. LLD accepts NONE in an empty section with a symbol table; it rejects absent-table/invalid-index variants. Do not call all unusual NONE forms ecosystem-valid. |
| Enable the real linker for every rejection case | **Retracted.** LLD agrees with only 14/100 fixed-width rejection invocations; it accepts 86/100. A blanket agreement gate would reward the wrong policy. |
| PGO output/count expectations were self-referential | Confirmed. Six host-compiled UBSan executions establish analytic output expectations independently; absolute entry counts are now checked in LCCC integration. |
| ferror/perror can report unrelated errno | Confirmed semantic mismatch. Zero/stale-errno injections now require a stable generic write diagnostic. |
| Corrected-runtime contention was not forced | Correct. The reference-runtime mutation experiment now forces A to hold the lock until B is attempting acquisition. |
| 6.1/10 grade described proven quality | It was an editorial judgment, not an empirical confidence estimate. Test counts do not make a new 10/10 proof either. |

The first review clearly distinguished inherited defects from new regressions;
that distinction remains. The original exact patch-application checks were valid,
but zero fuzz/blob equality prove application integrity, not compiler correctness.

## Experiments actually executed

### Real ELF oracle: LLD 21.1.0

Installed the Zig 0.16.0 binary wheel from the allowed Python package registry.
Its LLVM tools report Clang/LLD 21.1.0. This is not GCC 16.2 or an LCCC substitute.

1. **38/38 well-formed link invocations passed**: original positive cases plus
   padded SET/SUB ULEB fields of lengths 1–10 and an empty-section NONE case,
   in executable/shared modes. Results include field bytes and adjacent sentinels.
2. **100 malformed fixed-width invocations**: 14 rejected as required; **86
   accepted contrary to the strict safety contract**. The experiment deliberately
   exits nonzero and is recorded as a disagreement experiment, not a passing gate.
3. Byte-level examination of a 16-byte section with an 8-byte R_RISCV_64 field
   at offset 9: LLD exits 0. The emitted eight-byte field is
   `58110100000000a5`, while the relocated symbol requires `5811010000000000`.
   The adjacent sentinel remained intact. This establishes an incomplete
   relocation in the accepted image; **it does not establish adjacent-memory
   corruption by LLD**. Acceptance alone was not used to infer corruption.
4. NONE probes: empty section/offset 0 and empty section/offset UINT64_MAX,
   with index 0 in an existing table, are accepted. An absent symbol table or
   index UINT32_MAX is rejected by this LLD. The production exemption is restricted
   to actual RISC-V NONE and does not relax other architectures' storage checks.

Commands:

```sh
python3 tests/linker/check_riscv_relocation_contract.py \
  --oracle-only --oracle ld.lld --only valid- --json valid-oracle.json

# Expected exit 1 for LLD 21.1; preserve and inspect disagreements.
python3 tests/linker/check_riscv_relocation_contract.py \
  --oracle-only --oracle ld.lld --only bad-extent --json malformed-oracle.json
```

The current generator has **105 fixtures / 210 LCCC link invocations**:
19 accepting and 86 rejecting fixtures. Newly covered failures include SET-only,
SUB-only, reversed/mismatched pairs, insufficient capacity, negative difference,
unterminated fields, and oversized encodings. Pair positives extend positive
kind coverage to 5/39 kinds, not all 39. Full valid-kind/TLS/GOT coverage remains
work, not a completion claim.

### Host-compiled preload oracle, not just a source model

`tests/linker/test_pr826_redteam.py` passes **9/9 tests under GCC 12.2** and
**9/9 under Clang 21.1**. The tests include:

- Six generated-program configurations (1/96 helpers × O0/O2/O3), compiled with
  UBSan and checked against independent output `1` / `-4464` and exit 23.
- Eight injected I/O failures with a compiled reference profile writer,
  including zero and stale errno for a sticky stream error.
- Two synchronized processes with a profile larger than 8192 bytes.
- A deliberate original-ordering mutant: unlock before close. It is detected by
  both the unlocked-close marker and corrupted serialization, while the corrected
  reference writer preserves exact bytes.
- Optimized-Python invocation refusal, malformed/blank records, header identity
  reset, structured missing-tool failure JSON, ELF generator integrity, and a
  textual relocation metadata/dispatcher drift alarm.

These reference drivers validate the test oracle, **not** LCCC's generated
runtime. The hosted integration gate separately executes generated LCCC programs.
That gate now contains **72 intended checks**, rather than 60, and requires
absolute function entry counts of one before merging and two afterward.

### Production Rust module

The dependency-free production file is compiled directly through
`tests/linker/riscv_uleb128_unit.rs`; this does not substitute a Python translation
for the Rust implementation. The first hosted execution passed six Rust tests,
including **49,155 exhaustive small-value/length combinations**. The final suite
adds **40,960 deterministic full-width samples**, invalid reserved lengths, and
retains tests for padding, sentinels, widened arithmetic, malformed originals,
and field length preservation after an overlapping write.

These are exhaustive over the stated small domains and sampled over larger
ones. They are not exhaustive over every input ELF, optimizer state, or u64 tuple.

```sh
rustc +1.99.0 --edition=2024 --test tests/linker/riscv_uleb128_unit.rs \
  -D warnings -C opt-level=1 -C debug-assertions=true -o uleb-tests
./uleb-tests
```

### Corpus inventory and reference correctness

The actual checked-out runner registers **45 workloads**, not 39. They use
**43 distinct sources**; `programs/` contains 58 C files, of which 15 are not
registered runtime workloads. Several are specialized code-shape studies and
must not be silently counted as tested executable workloads.

Executed the existing runner at O2 and O3 with GCC 12.2 and Clang 21.1:
**45/45 output comparisons passed at each level; 180 compiler executions total**.
The runner's paired-round structures contain correctness samples in skip-perf
mode; those are **not timing evidence**. LCCC was not available locally, so these
local results validate reference/harness consistency, not LCCC superiority.

The exact-toolchain hosted job builds LCCC and screens all 45 registered workloads
against Debian GCC 16.2 and Clang at both optimization levels with `--strict`.
It records failures rather than silently dropping an unavailable compiler.

Unregistered sources, explicitly not covered by those runtime comparisons:
`adler32_do8_hot`, `affine_countdown`, `base64_enc`, `conv_u8_3x3`,
`crc32_nibble_hot`, `csv_field_sum`, `fir_filter`, `fp_memfold_stencil5`,
`global_addr_pressure`, `i686_alu_chains`, `moving_stats`, `prefix_scan`,
`reduction_vecreg`, `vecreg_new_ops`, `vector_remainder`.

## Red-team finding in the new harness itself

The first expanded exact-Debian job passed, but the Ubuntu main job failed in
our new reference-driver compilation. A fast Ubuntu reproduction and bounded
API-readable unittest annotations localized it to that compilation, not LCCC.
A local `-D_FORTIFY_SOURCE=3 -O1 -Wall -Wextra -Werror` experiment reproduced the
specific diagnostic: casting `write`'s return value to void does not satisfy its
`warn_unused_result` contract. The reference driver now checks the return,
cleans up, and preserves application status on diagnostic failure. Its compile
command explicitly enables fortification so this cannot hide behind a distro's
default settings. Compiler stderr is captured in failure annotations. Warnings
remain errors; the test was fixed rather than weakened. This defect was introduced
by the red-team harness, not by the original PR's production implementation.

## Design rationale for the fixes

### Bounded ULEB module

- Recover reserved length from the **immutable original input**, not the current
  merged bytes. Earlier overlapping relocations cannot resize the field.
- Validate SET/SUB adjacency and equal offset before application. Never encode
  an intermediate absolute address or silently consume a standalone SUB.
- Compute `(S + A) - (S2 + A2)` in i128. All combinations of u64 symbols and i64
  addends fit this intermediate domain; reject a negative or >u64 final result.
- Check final capacity before mutation, then write only the exact reserved slice,
  preserving padding and neighboring data. There is no partial write on failure.
- Explicitly reject unsupported >64-bit original encodings; never scan into the
  next contribution or perform an oversized shift.
- Success is bounded by ten scanned/encoded bytes and allocates no heap memory.
  That is an algorithmic cost bound, not a measured compiler-throughput speedup.

This is better than adding another merged-buffer check or clipping a write. The
field and pair semantics are represented directly and independently testable.
It does not introduce a giant unmeasured optimizer rewrite to fix a linker bug.

### PGO diagnostics and contention

Retain close-under-lock. Use a short direct stderr diagnostic for sticky stream
errors instead of attaching an untrustworthy errno. The PGO instrumentation entry
point is explicitly x86-64-only, so its size_t/ssize_t call ABI is 64-bit. This
remains best-effort error reporting and does not roll back partially written
records or override application status.

The preload oracle's A/B barrier now creates actual acquisition contention. Its
reference mutant demonstrates that the test rejects the old ordering. Counter
parsing validates shape/ranges and resets per-record function state. Failures
produce JSON instead of disappearing before summary publication. Assertions
cannot be silently disabled by an optimized interpreter invocation.

## Toolchain, swap, and measurement limitations

- Local host starts with Debian GCC 12.2 and no Rust compiler. Debian downloads
  and `static.rust-lang.org` fail under the outbound host policy. A rustup 1.29
  manager was installed through a Python wheel, but **Rust 1.99 installation
  locally failed**. Do not label the manager as the requested compiler upgrade.
- Added `.github/workflows/pr826-redteam.yml`: Debian unstable container, explicit
  `gcc-16 -dumpfullversion == 16.2.0`, Rust/Cargo/rustfmt/Clippy from toolchain
  1.99.0. The exact-version provisioning steps have passed on hosted CI.
  This supplements, rather than changes, the repository's latest-stable policy
  and existing measured MSRV floor.
- Activated the repository's **8 GiB swap file**; `/proc/swaps` confirms it.
  The root swapfile is outside the persisted repository and may need recreation
  after a sandbox restore.
- Hardware CPU-cycle `perf_event_open` probe returns **ENOENT**. No PMU evidence
  is available. Valgrind, perf, gdb, and strace executables are not installed
  locally; do not pretend they were used or equate static metrics with PMU data.
- Compiler Explorer/uops.info endpoints and GitHub artifact/log blob hosts are
  outside allowed egress. Oracle logic tests and API job/check status are usable;
  live CE comparisons and artifact downloads are not. Formatter output was
  recovered through bounded GitHub check annotations and applied without changing
  semantics. Raw hosted artifacts remain available to the user through GitHub.

## Correct use of the repository's performance laboratory

Inspected the purposes and contracts of the relevant tools:

| Tool | Correct question | What it does not prove |
|---|---|---|
| `codegen_oracle.py --rank` | Where are whole-function instruction/stack/call gaps? | Runtime speed, or equivalent work when inlining/call scope differs. |
| `oracle_asm.py` | Inspect the same cached assembly/metric pipeline in a focused deep dive. | A separate independent metric or a runtime win. |
| `loop_density_oracle.py`, `hot_loop_metric.py` | Screen instructions per unit of loop work. | Semantic phase equivalence; a largest-step loop need not be the corresponding hot loop. |
| `tight_loop_oracle.py` | Encoded span/alignment eligibility and bucket decisions. | Throughput or a reason to align oversized loops. |
| `tune_oracle.py dump/matrix/compare` | Observe CPU tuning rows and instruction-selection decisions. | That a choice is faster on the intended hardware. |
| `cpu_model_audit.py` | Check model values against uops.info with declared tolerances. | End-to-end workload performance or exact agreement for tolerance-based checks. |
| `bench_kernels.py` | Isolated kernel A/B with a shared reference driver and output checks. | General compiler superiority or PMU evidence from a VM. |
| `run_benchmarks.py`, `bench_rank.py` | Paired randomized corpus results, sample reliability, regression distribution. | A win when correctness fails, samples are inadequate, or only a geomean looks good. |
| `oracle_delta_gate.py` | Guard against committed structural baseline regressions. | Outperforming every oracle; unreachable CE can be a recorded skip unless strict. |

Ran the available existing logic suites: codegen comparability **14 tests**,
oracle-delta guard **7 tests**, benchmark ranking **1 test**, relocation oracle
verdict **44 cases**. All passed. CI/local parity and workflow-shell checks also
passed. This protects interpretation and gating logic; it is not a measurement
of the newest competing compilers' generated code.

No CPU model, vectorizer, register allocator, or alignment optimization is changed
here. There is no defensible empirical basis in these experiments for claiming
that an optimizer rewrite would beat all other compilers. On a capable host:
correctness first, then scope-matched static/density screening, assembly inspection,
isolated A/B, randomized corpus timing with per-workload intervals, and finally
hardware counters on the intended CPU. Never hide a regression behind a geomean.

## Remaining test axes and debt

The next agent should extend the suite along these independent axes rather than
calling a long list of examples exhaustive:

1. All 39 supported kind categories, exact-end valid fields, truncated fields,
   storage versus value-range/alignment constraints.
2. Symbol binding/visibility/definition/section versus discarded/non-alloc data.
3. Separate input objects, repeated sections, regular/thin archives, required
   versus irrelevant malformed members, and multiple relocation tables.
4. GOT/PLT, TLS model combinations, paired HI/LO semantics, dynamic relocations,
   and unresolved/preemptible symbols.
5. ULEB field lengths and capacities, malformed pairs, repeated/overlapping
   relocations, label/addend extremes, and unsupported encoding widths.
6. PGO multi-TU/DSO destructors, indirect-call value profiles, record identity,
   interrupted/failed acquisitions, partial writes, short diagnostics, close
   errors, and multiple writers.
7. Host word size, debug/release assertions, optimized Python, malformed output,
   missing tools, timeouts, abnormal exits, and reproducibility after restore.
8. Every registered workload's existing semantic oracle, plus explicit harnesses
   for currently unregistered source shapes before runtime claims.

Archive parse-error suppression, numerical relocation range checks, and shared
TLS limitations remain inherited debt. The metadata check is a **textual drift
alarm**, not a type-level proof of dispatcher completeness. Multi-TU/value-profile
PGO and all-kind positive coverage are not complete. Do not assign 10/10 or merge
merely because the concrete blocking defect now has a bounded implementation.
