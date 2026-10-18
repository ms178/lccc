# LCCC engineering backlog

**Live, evidence-qualified open work only.** This roadmap is reconciled against
`main` at `a88fb736bfe253c91018b32ea0ba5d4b7dc6057e` (PR #844 merge, after
PR #846), 2026-10-17. It is not a compiler-quality certificate or a record of every
historical experiment. The previous 3,302-line queue is preserved verbatim at
[`engineering/archive/ROOT-BACKLOG-2026-10-17.md`](engineering/archive/ROOT-BACKLOG-2026-10-17.md);
its dated status text must not be read as current without revalidation.

## Decision rules

- **Correctness and evidence integrity outrank performance.** Every semantic
  or code-generation change needs a minimal failing witness, a positive control,
  and a negative control. Compare exit status and output, not stdout alone.
- **No performance claim from static instruction count or Callgrind Ir alone.**
  Callgrind events are simulated counts under a model, not PMU cycles. Runtime
  claims require the canonical output-checked paired runner, immutable compiler
  and workload identities, matched flags, complete raw rounds, and meaningful
  durations. Report initial screens and follow-ups separately.
- **Do not tune by workload name.** Use general legality/profitability rules;
  retain all regressions and rejected controls. Never remove `-fwrapv`, weaken
  an alias/volatile/FP barrier, or widen an optimization's scope to recover a
  benchmark result without a proof and its named correctness oracles.
- Re-oracle every code-generation change, including the gzip
  `longest_match` stack-memory veto. Keep rustfmt, strict Clippy and
  `scripts/ci_local.sh --fast` green before publication. Full/slow CI is a
  hosted-runner responsibility for this task.
- Evidence limits are part of each result. A skipped/unavailable oracle is
  missing evidence, not a pass. A moving Compiler Explorer channel is not a
  binary pin; record its resolved metadata.

## Current evidence boundary

- The current upstream base is `a88fb736bfe253c91018b32ea0ba5d4b7dc6057e`
  (PR #844 merged atop #846). `gh pr checks 844` reported 14/14 checks passing,
  including build/unit, Clippy, MSRV, codegen/runtime, assembler/linker,
  relocation, corpus, and GCC 16/Rust 1.99 contracts. This is hosted evidence;
  the candidate-specific audit/performance job was skipped, so it is not a
  fresh performance acceptance receipt.
- The upstream PR #844 S05 follow-up records a 377-case corpus result of 248
  pass, 128 fail, and 1 error, plus an identified compile-time profiling
  concern (MS-20) and active FE-26/FE-27/LK-31 work. Preserve these as open
  engineering items; they are not closed by green CI. See
  [`engineering/FOLLOWUP-2026-10-17-s05-rebase-backlog.md`](engineering/FOLLOWUP-2026-10-17-s05-rebase-backlog.md)
  and [`engineering/agent/BACKLOG.md`](engineering/agent/BACKLOG.md).
- This workspace lacks Rust/Cargo, Valgrind/Callgrind, GCC 16, and AArch64
  cross tools; available host GCC is 12.2. Python checks passed where run, but
  PyYAML was missing from system Python; after installing it in a temporary
  virtual environment, CI gate-parity and the previously blocked selected tests
  passed. The final `ci_local.sh --fast` attempt under system Python therefore
  still failed its workflow-shell parser gate, then stopped at build because
  Rust/Cargo are absent. No post-merge FAST pass, rustfmt, Clippy, LCCC build,
  compiler regression, or performance measurement is established here. Hosted
  checks and local evidence are intentionally kept separate.
- The offline oracle-tool failure suite passed **40 tests**. In particular,
  `oracle_asm.py` and `tune_oracle.py` already fail closed on missing local or
  remote output; the older backlog's proposed fixes for these behaviors are
  stale and are not reopened.

## P0 — semantic correctness and capability honesty

### P0.1 — Revalidate and complete `ext_vector_type` layout semantics

**Evidence/source:** archived A14-13 follow-up in
[`BACKLOG-2026-10-16.md`](engineering/archive/ROOT-BACKLOG-2026-10-17.md),
section A14-13; `tests/regression/check_ext_vector_size.sh`.
The prior audit reported two unresolved semantic mismatches: `_Bool`
`ext_vector_type(4)` storage size and padded three-lane `unsigned long` vectors
on i686, plus agreement between parser-time static assertions and runtime
`sizeof`. The typedef/type-name width repair is not proof of logical lane-count,
layout, ABI or execution semantics.

**Next:** reproduce each case on the current tree against current Clang and
GCC where applicable; inspect declaration and type-name paths separately.
Represent logical lane count and storage size explicitly if the mismatch
persists. Add runtime, `_Static_assert`, operation, and ABI controls for m32/m64.

**Accept:** same observed `sizeof`, alignment, lane count, and observable
operations as the supported compiler contract for every accepted case; invalid
forms diagnose; the full regression/differential battery passes. Do not round
byte counts or alter `vector_size` semantics as a substitute for a representation
model.

**Progress 2026-10-17 (PR #852 update):** sema now evaluates `_Static_assert`
conditions with the full type context, which exposed an internally
inconsistent `bool33` assertion in `ext_vector_layout.c` (sizeof 5 beside
alignof 8 is not a representable C layout); the fixture now asserts the
single coherent power-of-two rule its own `bool17` case follows (8/8).
Parser/sema/lowering enum layout follows the GCC compatible-type rule
(widening, packed minimum, struct inheritance) and is byte-identical to the
system GCC oracle; one-lane FP vectors ride `%xmm0` with integer one-lanes
in GPRs (AUD-VABI-4), covered by `check_enum_layout_semantics.sh` and
`check_ext_vector_one_lane_fp.sh`. Remaining: cross-compiler receipts for
the bit-packed bool rule need a Clang/ICX oracle (unavailable in this
sandbox; hosted CI lanes are the vehicle).

### P0.2 — Close unsupported-option acceptance; revalidate wrapped byte-address semantics

**Evidence/source:** archived A14-16 and the accepted-`-fwrapv` byte-address
witness. PR #844 adds `LCCC_STRICT_OPTIONS` plus tests for strict unknown-option
handling, and the current fast gate includes `check_pr824_boundaries.py`, which
executes `wrapv_byte_address.c` across optimization modes and supported targets.
The #844 hosted checks are green. This closes the historical unknown-option
acceptance defect and gives a permanent executable wrap-domain oracle, but the
local compiler run is still unavailable in this session; do not claim a fresh
local replay.

**Remaining:** inspect classification of `-flto` and sanitizers; once Rust/Cargo
are available, run the exact `-O1`/`-O2` witnesses and the boundary gate locally.

**Accept:** unsupported capability requests diagnose unless explicitly
classified as benign compatibility no-ops; `-fwrapv` preserves the modular
result across optimization levels and targets. Keep wrapping and non-wrapping
controls and avoid blanket rejection of build-system `-f*` probes.

### P0.3 — AArch64 shared-symbol visibility repair landed; maintain regression evidence

PR #844 replaces the former narrow ADR repair with a shared visibility merge
across all object references and definitions after dynamic resolution. The
shared emitter uses that merged visibility consistently for export filtering,
`dynsym` visibility and ADR admission. Hidden/internal definitions are excluded;
protected symbols remain exported with their binding semantics. Existing
AArch64 address-contract tests cover hidden shared output. Static source review
and green hosted assembler/linker checks support closing the historical
implementation defect; this session did not execute the local cross-target
matrix or independent-linker comparison. Keep those as regression-evidence
limitations, not as a claim that the specific old defect remains open.

### P0.4 — Revalidate RISC-V relocation error and preservation contracts

**Evidence/source:** archived A14-15/A14-17. Unknown-relocation and AUIPC fixes
are recorded as landed, with bounded tests, but the old wide audit also called
out expression-classification, extent/index, ET_EXEC/ET_DYN, and relocation
preservation coverage. Do not treat the original zero-emission reproducer as a
current defect without replaying it.

**Next:** replay the archived malformed-expression and mutated-relocation
fixtures on current main; enumerate applied, dynamically retained, relaxed,
and rejected relocation classes across executable and shared outputs.

**Accept:** no unsupported expression silently becomes zero or loses a symbol;
malformed relocation metadata is rejected in both output modes; valid supported
relocations and addends are preserved/applied exactly. Keep valid symbolic
subtraction as a positive control. Add no blanket rejection for documented GAS
expression semantics.

## Integration audit — PR #844 atop PR #846

- The direct source overlap is limited to pass scheduling/configuration and
  regression gates; #844 does not replace the generator-map transform or its
  legality predicates. In the composed pipeline, `late_vectorize_entry` keeps
  the established AArch64 two-wide entry, while generator-map is invoked only
  from the main x86 vectorization mode, is disabled in late-minmax-only mode,
  and retains its dedicated kill switch/trace. Loop rotation remains explicitly
  opt-in. Static review found no contradictory pass ownership or evidence that
  the default pipeline silently broadens these changes. This is not a substitute
  for compiler execution.
- One actual harness-contract defect was found: generator trace validation
  compared only the set of kernel names, allowing duplicate events when all
  expected positives were present. It now requires exactly one trace event per
  positive fixture (and none when disabled), with a compiler-free mutation test
  for the fully-present-plus-duplicate case. Its focused 7-test suite passes.
- The `-fwrapv` byte-address case is not discarded as stale: it remains inside
  the permanent PR824 boundary gate. PR #844's codegen/runtime and integration
  checks are green, but the individual hosted substep was not separately
  inspected in this audit; local replay remains blocked by missing Rust/Cargo. A14-35 is no longer treated as an open defect:
  the merged visibility decision is shared by AArch64 export, dynamic-symbol
  visibility, and ADR resolution. Existing audit obligations for ext-vector
  layout and RISC-V relocation edges remain distinct and open until executed.

## P1 — measured performance work, correctness-preserving only

### P1.1 — Fannkuch runtime regression: explain before changing code

**Evidence:** S101/S102 reported about **25% slower than main** in paired native
screens; the later bounded record shows full-default Callgrind Ir
17,131,993,449 versus 16,054,326,746. These are separate metrics: Ir is not
cycles. A `-fno-wrapv` ablation is diagnostic only and is explicitly not
permission to remove `-fwrapv`.

**Next:** freeze the exact current source, flags, executable and input; attribute
hot-loop instruction/value-flow changes using Godbolt assembly, the existing
IVSR/stack census tools, and matched Callgrind. Test any proposed dominating,
width-aware range/no-wrap certificate against executed backedges, zero-trip
loops, negative/variable steps, `INT_MIN`/`INT_MAX`, narrow consumers and all
pointer widths. Keep unsupported cases on the original arithmetic path.

**Accept:** semantics on x86-64, i686, AArch64 and RISC-V; verifier/dominance
and wrap-domain controls; no increase in gzip `longest_match` stack memory;
output-checked native A/B with complete paired rounds and useful duration. A
reduction in Ir or assembly size alone does not close the runtime regression.

### P1.2 — Zstandard code quality and application-level runtime: no victory claim

**Evidence:** S101/S105/S106 show no reliable application-level zstd speedup.
The broad GCC deficit remains; an apparent random/long-input gain reversed
across controls. The header-inline-assembly BZHI candidate was rejected after
its pilot (compression GM 1.017476, with an entirely slower JSON level-3
interval and increased instruction/cache counts). The portable implementation
is the baseline.

**Next:** only continue with a causal, source-independent hypothesis. Profile
bounded Callgrind regions and inspect exact hot instructions/register-stack
traffic before considering general narrow-load forwarding, address recurrences,
or explicit parameter-save provenance. Rebuild the patched source from a frozen
identity and verify assembly and output before timing. Preserve RLE and
random/long20 as separate workloads; do not pool them to hide opposite outcomes.

**Accept:** semantic/memcheck controls, unchanged valid workload and flags,
matched compiler/ELF identities, independent paired runtime confirmation whose
interval supports the claimed direction, and full corpus regression review.
Otherwise retain as investigation only. Do not reintroduce BZHI inline asm,
relax alias/liveness checks, or claim GCC superiority from Ir.

### P1.3 — Validate performance on actual target hardware

The supplied Xeon VM is screening evidence, not the requested Raptor Lake
14700KF PMU environment or EPYC7763 host. No native target-hardware/PMU claim is
closed by VM percentages or Callgrind.

**Next:** acquire same-window, same-baseline, output-checked runs on the named
physical host; record CPU/microcode, affinity, frequency policy, compiler and
workload hashes, raw pairs and PMU availability. If hardware is unavailable,
keep the item explicitly blocked and report VM evidence as screening only.

**Accept:** reproducible paired runtime and available hardware counters for a
bounded, named claim; preserve unavailable counters and every failed/unsupported
case. No universal “best compiler” conclusion.

## P1 — baseline freshness and integration

### P1.4 — Fresh current-tree comparison and required local gates

PR #844 CI is valuable integration evidence, but it does not establish all
candidate-specific semantic/performance obligations, and the named candidate
audit job was skipped. Once toolchain prerequisites are present, freeze current
`main` and candidate independently, recompile the relevant benchmark corpus,
confirm output identities, rerun worst regressions, and keep the receipts
separate from older S101–S106 evidence.

**Required before publication:** `scripts/ci_local.sh --fast`, rustfmt, strict
Clippy; for code-generation edits, the `engineering/agent/RULES.md` test and
oracle requirements. Full/slow CI is not authorized locally; rely on hosted CI.
Never publish or open a PR as locally validated unless these gates actually pass.

## Rejected/stale queue claims

- The root backlog's dated A14/PR834/PR837 status is historical; its baseline
  hashes are not this tree. Reproduce before calling any original failure
  current. Completed entries stay in the archive, not the live queue.
- The earlier claims that `oracle_asm.py` returns success on compile/extraction
  failure and that `tune_oracle.py` returns success with a partial vendor table
  are rejected as stale: current code returns nonzero and the 40 focused offline
  tests exercise local, remote, missing-function, empty-output, stale-report,
  and execution-result failures.
- The BZHI header-asm optimization is rejected on measured negative evidence;
  no production adoption or relaxed safety gate is authorized.
- Callgrind Ir, static instruction counts and single-run VM timings are not
  runtime wins. Historical compiler comparisons remain attached to their
  original source/base and cannot be promoted to current-main claims.

## Measurement and implementation workflow

1. Check [`engineering/agent/RULES.md`](engineering/agent/RULES.md),
   [`engineering/DECISIONS.md`](engineering/DECISIONS.md), and the matching
   source/evidence receipt before each item. Use the canonical tool map at
   [`engineering/SCRIPT-MEASUREMENT-MAP.md`](engineering/SCRIPT-MEASUREMENT-MAP.md).
2. Use `scripts/godbolt.py`, `scripts/codegen_oracle.py`, `scripts/oracle_asm.py`
   and the relevant script-specific gates. Live Compiler Explorer was not
   reachable/used in this audit; offline oracle self-tests are not vendor
   comparison data.
3. For dynamic instruction/caching investigation, use the repository Callgrind
   tools only when Valgrind is installed; preserve exact geometry, full counters,
   source/binary identities, native/instrumented output equivalence and complete
   status. Callgrind is not a substitute for PMU/runtime measurement.
4. Runtime: `tests/benchmark/run_benchmarks.py` with immutable compiler/input
   identities, matched options, canonical output checks, paired/interleaved
   rounds and raw receipts. Rerun at least the baseline control when an
   apparent effect changes across ASLR/layout/order.
5. Add permanent tests for a real defect, mutation-check the targeted failure,
   re-oracle, remeasure, and only then update an item's status. Record the dated
   session digest under `engineering/journal/`; retain durable negative
   decisions in `engineering/DECISIONS.md` and raw data in `engineering/evidence/`.
