# LCCC engineering backlog

**Live, evidence-qualified open work only.** This roadmap is reconciled against
`main` at `4ba567022b7d358837aa13be4eb999621871c9f9` (PR #941 merge),
2026-10-23. The audit reviewed the 100 most recent upstream commits and
inventoried 60 distinct engineering/docs Markdown paths they touched; statuses
below are revalidated only where the referenced gate or evidence was rerun,
not inferred from commit presence. This is not a compiler-quality certificate
or a record of every historical experiment. The previous 3,302-line queue is
preserved verbatim at
[`engineering/archive/ROOT-BACKLOG-2026-10-17.md`](engineering/archive/ROOT-BACKLOG-2026-10-17.md);
its dated status text must not be read as current without revalidation.

## Sign-test fold red-team closure (2026-10-24)

The previously listed sign-CMOV follow-up is closed on the current PR branch by
[`engineering/journal/2026-10-24-sign-test-cmov-redteam.md`](engineering/journal/2026-10-24-sign-test-cmov-redteam.md):
dedicated skip group, exact-flag producer refinement, directly mirrored
differential gate, i686 branch twin, orphan-gate mutation tests, refreshed lane
costs and sanitised evidence. Remaining open work is PMU-level runtime
confirmation and broader scalar min/max/abs lowering; those are not claimed by
this peephole receipt. Local ICX 2026.2 provisioning is blocked by the sandbox
egress allowlist, so the hosted authenticated ICX job is the authoritative ICX
oracle and is wired to this exact fixture.

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

## Current evidence boundary (2026-10-23)

- Current upstream identity: `4ba567022b7d358837aa13be4eb999621871c9f9`
  (PR #941 merge). The prior 2026-10-17 base and hosted receipts remain useful
  history only; they are not current-main test or performance claims.
- The last 100 main commits touched 60 distinct Markdown paths under `docs/`
  and `engineering/`. The highest-impact current findings are recorded in
  [`docs/FOLLOWUP-2026-10-23-open-pr-audit.md`](docs/FOLLOWUP-2026-10-23-open-pr-audit.md),
  [`docs/FOLLOWUP-2026-10-23-remaining-codegen-gap.md`](docs/FOLLOWUP-2026-10-23-remaining-codegen-gap.md),
  [`docs/FOLLOWUP-2026-10-22-ipa-ra-callee-clobbers.md`](docs/FOLLOWUP-2026-10-22-ipa-ra-callee-clobbers.md),
  [`engineering/journal/2026-10-22-inline-memmove-and-zstd-decode-gap.md`](engineering/journal/2026-10-22-inline-memmove-and-zstd-decode-gap.md),
  and [`engineering/FINDINGS-2026-10-21-s93.md`](engineering/FINDINGS-2026-10-21-s93.md).
  These reports invalidate several rows in the older 2026-10-22 codegen TODO
  table; current-tree rechecks below take precedence over those earlier statuses.
- Revalidated on the current main compiler before this follow-up: the phi
  acyclic-order gate passes (`rot()` default 50 instructions / 3 stack refs;
  GCC 16.2 60 / 6; both policy arms match on 65 trip counts); the LLVM scalar
  facts gate passes with GCC 16.2 and Clang; the 16-byte atomic runtime
  checks pass at `-O1`, `-O2`, and `-O3`. The first standalone atomic run
  lacked GCC 16 on `PATH`, so its oracle-parity leg was skipped there; the
  final fast-CI rerun below is the oracle receipt. Thus the old TODO-7 and
  TODO-10 claims are stale on current main. TODO-11's old witness is also superseded:
  the current scalar-facts gate uses a different mechanism witness and passes;
  do not revive the old guard diagnosis without a new failing control.
- The open-PR audit did **not** adopt the 10k-line SIMD stack: the current
  49-program corpus has no matching 16-byte XMM stack operands, so those PRs
  remain unvalidated, not accepted. The layout-tie result is tree-specific and
  already landed; the original #932 benchmark number is not portable across a
  rebase. Keep the `CCC_LAYOUT_TIE_ORIG=1` rollback and the strict stack rule.
- The remaining broad codegen/runtime gaps are not closed by those green gates:
  `hot_web_loop` still has an IPA-RA/callee-clobber hypothesis; Zstandard's
  recent memmove/byval fixes improved specific workloads, while
  `ZSTD_decompressSequencesLong_default` (2517 instructions / 871 frame
  references vs GCC's 1786 / 401) and `ZSTD_safecopy` (211 / 58 vs 152 / 0)
  remain separate allocator gaps. These are evidence for follow-up, not runtime
  wins attributed to the present peephole.
- Local tool provisioning for this task: Rust stable 1.99.1, GCC 16.2.0,
  Clang 23.1.3, ICX 2026.2.0, Valgrind 3.24, AArch64 and RISC-V cross GCCs,
  QEMU user, and an active 8 GiB swapfile. `gcc-16 -m32` links. The Debian
  `gcc-14-multilib` component and i386 development libraries remain installed;
  Debian's `gcc-multilib` meta-package was removed when the cross compiler
  packages were installed because of their package conflict. The pinned GAS
  2.47 x86-64 and AArch64 toolchains are present. The final fast-CI result for
  the final fast-CI run after refresh/rebase is the publication gate; prior
  incomplete attempts are not promoted to a pass. The delivery receipt reports
  its exact result with rustfmt and strict Clippy. Full/slow CI is not run.
- Compiler Explorer metadata is not silently equated with local pins: its
  `clang` channel resolved to 23.1.0 in this run. Exact Clang 23.1.3 comparison
  for the current change is local and recorded in the regression receipt. The
  offline oracle-tool self-tests remain separate from vendor comparison data.
  The historical 2026-10-21 authorization to run slow CI was scoped to that
  prior PR; this follow-up runs `--fast` only, as directed.

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

**Progress 2026-10-21 (layout matrix receipts):** both historical layout
mismatches are re-measured and closed on the current tree, and the i686 half
now has a receipt that does not depend on multilib glibc headers.
`_Bool ext_vector_type(4)` is 1/1 (`sizeof`/`_Alignof`) and the padded
three-lane `unsigned long` vector is 32/32 on LP64 and 16/16 on i686 -- the
same numbers Clang 23.1.3 produces, which is the contract compiler for this
extension. The evidence is a new gate
(`tests/regression/check_ext_vector_layout_matrix.sh`) over a freestanding
fixture (`tests/regression/ext_vector_layout/layout_matrix.c`): 12 element
types x 10 lane counts (1-8, 11, 16) printed as `sizeof`/`_Alignof` and
compared byte-for-byte with Clang 23.1.3 for `-m64` and `-m32` at `-O0` and
`-O2`, plus a hash over lane stores and lane loads of the 72 odd/prime-lane
shapes, plus 240 compile-time layout laws (`_Static_assert`). Two controls
keep it from passing vacuously: a perturbed size row must break the
comparison, and a `_Static_assert` naming the LP64 answer for three `unsigned
long` lanes must be accepted by `-m64` and rejected by `-m32` (a target that
answered with the host's width model would accept it). The fixture includes
no header, so the i686 leg runs wherever a 32-bit crt links; the older gates
in this area (`check_ext_vector_layout.sh`, `check_ext_vector_abi_grid.sh`)
still need the target's glibc development headers and therefore skip i686 in
a minimal sandbox -- that limitation, not a compiler defect, is why the i686
receipts were hosted-runner-only. Observed on this tree: 4 oracle legs +
2 optimisation-invariance legs + 2 controls pass; the ext-vector size gate
also passes on both targets.

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

**Evidence:** S101/S102's approximately **25% slower than main** result and
full-default Callgrind Ir totals (17,131,993,449 vs 16,054,326,746) are
historical, not a current Fannkuch receipt; Ir is not cycles. The latest
49-program paired report records corpus geomean `lccc/GCC16.2` 0.7897 on the
synthesized main, but does not establish Fannkuch's current per-program ratio.
Remeasure the exact program before carrying forward the old slowdown. A
`-fno-wrapv` ablation is diagnostic only and is explicitly not permission to
remove `-fwrapv`.

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

**Progress 2026-10-21 (causal stack census; no runtime claim):** the
question "why is lccc slower than GCC 16.2 on this library" was narrowed with
the repository's own causal tools rather than with another knob sweep.
`scripts/stack_census.py` over the three hottest compression/decompression
translation units (`zstd_fast.c`, `zstd_lazy.c`,
`zstd_decompress_block.c`, `-O2 -march=x86-64-v3`) attributes **60.8 % of all
stack references to register spills** (13,578 of 22,331; coverage 99.89 %),
against **7.0 % structurally required memory** (`alloca`/`address`/`wide`/
`nongpr`). The busiest slots are not short-lived temporaries: in
`ZSTD_compressBlock_lazy_generic`, offset `-64` is written **once** and read
**204** times, and offset `-584` is written once and read 194 times -- a
long-range value the global allocator spilled at its definition and reloads
per region, where GCC keeps it in a callee-saved register. A per-translation-
unit ablation (each hot unit rebuilt with GCC 16.2 inside an otherwise lccc
library, interleaved min-of-N process-CPU runs on one pinned core) shows the
deficit is **spread across units, not concentrated in one defect**:
substituting `zstd_fast.c` recovers 5.1 % at level 1, substituting
`zstd_double_fast.c` recovers 5.6 % at level 3, and substituting `zstd_lazy.c`
recovers nothing measurable -- while the whole-library gap is much larger than
any single unit. Two negative results are recorded so they are not retried:
the global location allocator is a **no-op** on this library (its archive is
byte-identical with `CCC_RA_GLOBAL_LOCATION=0`), and its per-function edit
budget is not the limiter (`CCC_RA_GLOBAL_LOCATION_MAX=1024` produces a
byte-identical archive too). Therefore no runtime claim is made here and no
knob is shipped: the remaining gap is the base colourer's global decision to
spill long-range hot values, which needs a residency/live-range-splitting
change with its own A/B and corpus review, not a threshold tweak.

**Progress 2026-10-23 (current gap attribution):** the latest 49-program
paired report shows `zstd_count` at +21.2% Callgrind Ir vs GCC 16.2, with
`mov*` accounting for 76% of the Ir delta. This is static instruction-count
attribution, not an application runtime result. The broader move census points
toward allocator-side zero-extension copies, symbol-plus-register address
composition, a loop-carried spill, and fixed-register shift chains; the report
explicitly rejects another peephole-only campaign. The exact current remaining
workloads are recorded above (`ZSTD_decompressSequencesLong_default` and
`ZSTD_safecopy`), and no global win or regression is inferred from the single
kernel.

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

**Progress 2026-10-21 (local CI on the pinned caches):** the hosted runs for
this PR never reached a compiler (two jobs died provisioning GCC 16 on a
Launchpad window), so the whole gate set was run locally from the tag caches
instead: all thirteen pinned assets verified against their `MANIFEST.sha256`,
Rust 1.97.0 (the declared floor), the bookworm i686 sysroot, and the pinned
`as`/`ld.bfd` 2.47, relocs, valgrind and A64 ISA XML oracles. `ci_local.sh
--fast` first reported 51 failures; on the pre-rebase tree it then ran 366
passed / 1 failed, and the linker suite went from 295 pass / 9 fail to 304
pass / 0 fail / 0 warn / 0 skip once the pinned oracles were in place. Four of
the 51 were repository defects fixed here: stale benchmark-registry counts in
three corpus modules; the `not_mask` bit idiom, now implemented with an
exhaustive-model unit test; the missing local mirror of the new layout-matrix
gate; and the audit-census drift, which a pristine base export reproduced and
which upstream's own rename pass has since brought back to the recorded
baseline (no baseline edit ships in this branch). The remaining 47 were
sandbox-toolchain gaps: the tag-cache GCC defaults to `-fno-pie` where distro
compilers default to `-fPIE`, the tarball ships no sanitizer archives, and the
32-bit glibc headers live under Debian's host-triplet directory — each
reproduced explicitly so a gate measures the compiler, never the host. With
`LCCC_I686_SYSROOT` pointing at the extracted sysroot the pre-rebase tree ran
the differential suite to PASS=1026 FAIL=0 SKIP=3 (AB-diff 0), i.e. no `-m32`
build failures remain. The maintainer directed that these local, pinned-cache runs are the
evidence of record for this PR while GitHub's hosted runs are degraded, so
`--fast` **and** `--slow` are both run here; the older "full/slow CI is not
authorized locally" line above is superseded for this PR by that direction,
not by a relaxation of any gate.

**Progress 2026-10-21 (oracle-contract defect inherited from main):** the
rebased tree failed one corpus test, and it fails identically on a pristine
`git archive` of `origin/main` — `run_regression.py` (added in `8a4d424`)
downgraded *any* oracle binary that exits non-zero to `SKIP-COMPARE`, while
`test_wrapping_regression_contract` pins the same case as `FAIL`. A blanket
waiver silently disables the differential comparison for every row an
out-of-date oracle touches, so the waiver is now declared per fixture
(`LCCC_ORACLE_SELFCHECK=skip` in the `.env` sidecar, with the reason) and a
failed oracle self-check is otherwise a hard failure naming the declaration.
The only fixture that needs it is `gnu_constant_builtins` (vector
`__builtin_classify_type` is 19 on GCC 14+, -1 on GCC 12). Two tests cover the
declared and undeclared paths plus the fail-closed key parse.

### P1.5 — Continue scalar min/max/abs lowering from the sign-only witness

**Evidence:** the recent codegen-round reports identify scalar min/max/abs as a
remaining family. This follow-up implements one exact x86 case:
`max(0, a - b)` represented as arithmetic → same-width TEST → sign-only CMOV.
The witness changes 6 to 5 actual instructions in LCCC; GCC 16.2 uses 4, while
local Clang 23.1.3 and ICX 2026.2 use 5. Runtime output matches GCC/Clang/ICX
for `-O0`, `-O1`, `-O2`, `-O3`, and `-Os`, including wrapping edges. The
comparison is static code shape only; no runtime-speed claim follows.
See [`engineering/journal/2026-10-23-sign-test-cmov.md`](engineering/journal/2026-10-23-sign-test-cmov.md)
and the artifact directory
[`engineering/evidence/sign-test-cmov-2026-10-23/`](engineering/evidence/sign-test-cmov-2026-10-23/).

**Next:** enumerate the remaining min/max/abs shapes and determine which are
legally reducible from their exact flag semantics; retain strict-positive and
other ZF-dependent controls. Add a minimal witness and negative control for each
accepted generalization. Keep same-width and all intervening-flag-use barriers;
never rewrite from source-level algebra alone.

**Accept:** exact compiler differential over the supported arithmetic domain,
optimization modes, and existing overflow flags; mutation-sensitive runtime and
codegen gates; final fast CI, rustfmt, strict Clippy, and exact-version oracle
receipts. Any runtime claim requires the matched output-checked paired runner
on a workload with useful duration; instruction count is not a substitute.

### P1.6 — IPA-RA: known-callee clobber sets for `hot_web_loop`

**Evidence:** `docs/FOLLOWUP-2026-10-22-ipa-ra-callee-clobbers.md` measures
`hot_web_loop` at 1.549× GCC 16.2 runtime on a 99.8 ms median and finds 18
stack references in LCCC versus none in GCC. The specific mechanism is that
four hot loop-carried values are copied into callee-saved homes around a
`static noinline` helper because the allocator assumes every call clobbers all
caller-saved registers. This is a measured, bounded workload result, not a
universal compiler ranking.

**Next:** prototype post-allocation per-callee written-register summaries in
reverse topological order. Preserve conservative caller-saved clobbers for
recursive cycles, indirect/unknown calls, escaped functions, and external ABI
calls; inspect the full report's fallback design before coding. A per-callee
summary that is not proven safe must not affect allocation.

**Accept:** exact helper/call-graph, recursion, indirect-call, escaped-function,
and external-ABI controls; no allocator miscompile; reproducible assembly and
runtime evidence on `hot_web_loop`, the complete 49-program corpus, and callback
kernels. Use a matched paired runner and retain all regressions. Do not adopt
from static counts alone.

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
