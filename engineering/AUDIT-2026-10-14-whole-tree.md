# Whole-tree audit — 2026-10-14

> Historical audit record. The later implementation session rebased to PR #820
> (`e047aa1`), repaired A14-29/30, and reduced the queue to 31 open entries.
> See [current fixes, validation and limits](FOLLOWUP-2026-10-14-constant-gep-fix.md).
> Original evidence and claims below retain their original dates and bases.

Initial audited base: `3b66cd1d73743a7a79f912c01df48ac15cc646ef`.
Final rebased main: `1e83776f41529e2d754380d1d934ff901e25a097`.
The initial results below retain their original identity. The final-main
revalidation section distinguishes repeated evidence from carried-forward
source facts. This avoids silently relabelling old tests as new executions.
Scope: the owner's **audit/backlog** request, not a claim of performance supremacy.

## Evidence contract

- **R**: executable reproducer run in this session.
- **S**: inspected implementation establishes the stated structural limitation;
  its suspected user-visible consequence still needs the specified reproducer.
- **H**: hypothesis/research task, not a demonstrated defect or speedup.
- Historical `FIXED` labels are not evidence about this baseline. Preserve old
  IDs and cross-reference them rather than treating every old report as new.
- A repository-wide inventory is not a line-by-line semantic proof. This audit
  covers every subsystem at inventory level and selected high-risk contracts
  semantically. Untested paths remain explicitly unvalidated.

The implementation queue is in the **2026-10-14 whole-tree audit** section of
`backlog.md`. This document records method, scope, and reproducibility.

## Environment and constraints

The checkout is on the root filesystem (25 GiB, initially 21 GiB available),
which has more space than the tmpfs mounts. `/home/user/lccc` is a symlink to
`.cache/lccc`; `.cache` itself now points to `/opt/lccc-audit-cache`.
The original-base publisher counted even platform-excluded caches, so physical bulk
was moved out of the visible workspace after its budget check refused it.
Source/build bulk must not consume the workspace snapshot budget.
The complete committed source archive and cumulative patch are instead saved by
`scripts/lccc-snapshot.sh` under `/home/user/artifacts` and `/home/user`.
Rust and Cargo installations also live under `.cache`, not the persisted budget.

A 2 GiB `/lccc-swapfile` was created with `fallocate`, mode 0600, `mkswap`, and
successfully activated with `swapon`. It is **not persisted**; restore it using
`scripts/ensure_swap.sh` after a VM reset. LCCC is built only with the repository
`fastbuild` profile (`opt-level=1`, no LTO) and two Cargo jobs. Rust stable
resolved to `rustc 1.99.0 (b940084d7 2026-09-28)`.

`perf stat -e cycles,instructions true` reports both events **not supported**.
The fact that `perf` itself exits zero does not make the counters available.
There are no PMU, Raptor-Lake, frequency-controlled, or P-/E-core measurements
in this audit. No runtime speedup is claimed.

## Oracle and workload provenance checks

Executed `python3 scripts/godbolt.py audit`: the current aliases resolve to
GCC 16.2 (`cg162`), Clang 23.1.0 (`cclang2310`), ICC 2021.10.0
(`cicc2021100`), and moving ICX (`cicxlatest`). A moving-channel name alone does
not identify immutable compiler bytes; retain response metadata with a run.

Fetched mold v2.42.1 `CMakeLists.txt` directly: `MOLD_TARGETS` is the correct
CMake cache variable; `-DMOLD_TARGETS='X86_64;I386'` is already present in
`tools/linker/setup_oracles.sh`. Do not replace it with a made-up preset.
The requested linker pins are already in that script: bfd 2.47, mold 2.42.1,
lld 23.1.x. Their presence in a script does **not** prove local installation.
Distribution tools installed to exercise CI are not relabelled as those pins.

Queried the current `ms178/archpkgbuilds/packages` listing: gzip, zlib-ng,
expat, SQLite, zstd and linux-cachymod-6.18 are present. glibc's existing
provenance points to `toolchain-stable/glibc`, not `packages/glibc`.
The requested six initial extracted kernels and many additional algorithms
already exist in `tests/benchmark/programs/`; see `WORKLOAD_PROVENANCE.md`.
Do not duplicate them merely to increase corpus count. This audit instead
identifies integrity and coverage gaps in their existing harnesses.

## Initial source census (not a bug count)

`python3 scripts/audit_census.py --json REPORT --top-fns 12` on the INITIAL base reported:

| Metric | Value |
|---|---:|
| Rust files scanned | 514 |
| Panic-class text sites outside its test cutoff | 917 |
| Frontend/lowering/driver subset | 109 |
| Unsafe keyword sites | 29 |
| Bare ignored-test attributes | 6 |
| Functions over 300 / 500 / 1,000 lines | 196 / 77 / 23 |
| Distinct / undocumented knob literals | 438 / 243 |

**Do not call these exact semantic counts:** A14-01 records independently
reproduced lexer/cutoff defects in the census. The numbers are useful for
comparing the same implementation, not proving absence of new hazards.
The largest reported functions include x86 `emit_intrinsic_impl` (5,838 lines),
`emit_shared_library` (4,130), `emit_executable` (4,111),
`link_with_script_machine` (3,080), `encode_mnemonic` (3,056),
`allocate_registers` (2,669), and SLP `build_pack` (2,235).

## Red-team rules for implementing the queue

1. Reproduce before changing semantics. A TODO is a search lead, not a bug.
   Example: `lex_unicode_escape` still has a TODO for short escapes but already
   diagnoses insufficient digits and surrogate/out-of-range values; do not
   reopen that entire TODO as an unimplemented check.
2. A fail-closed diagnostic is preferable to fabricated successful output, but
   do not replace an internal invariant with a zero-valued fallback. That can
   turn a visible ICE into a silent miscompile.
3. Preserve address space, volatile/atomic semantics, exact integer widths,
   FP contraction policy, ABI classification and dominance at every rewrite.
4. Reuse existing corpus/CE/Callgrind infrastructure. Static instruction count
   and simulated Ir are **not cycles**. Compare complete outputs on every arm.
5. Snapshot after focused validation. Interim snapshots without the full fast
   gate are marked `UNGATED`; never synthesize a CI stamp. Final delivery must
   report the actual gate outcome and any infrastructure blockers.

## Coverage ledger

The immutable file inventory is
`engineering/evidence/2026-10-14-audit/inventory.json`, generated by
`scripts/audit_tree_inventory.py --revision 3b66cd1d73743a7a79f912c01df48ac15cc646ef`.
It enumerates source/header/tool/CI blobs, counts **all 5,624 tracked blobs** by
subsystem, and hashes the complete Git-tree listing. All 514 Rust files are in
that structural inventory. It is explicitly not semantic certification.

| Area | Inspected contracts / executed checks | Outcome / remaining queue |
|---|---|---|
| Preprocessor | includes/embedding/macros; -E chains 512/4096/8192 | all six chain runs succeeded; broader resource workloads A14-26 |
| Lexer/parser/AST | Unicode escape diagnostics, type-name vectors, identity | vector-size miscompile A14-13; avoid stale TODO false positives |
| Sema/lowering | type checker fallback, ExprId cache, invalid struct return | struct-to-int return correctly rejected; A14-09 |
| IR/analysis | Load/Store facts, UseDefInfo, LoopFrames, pipeline consumers | explicit metadata/invalidation tasks A14-10/11 |
| Scalar/loop/vector/IPO | IV widening, LICM/GVN, inlining costs, vector legality boundaries | full zlib miscompile A14-30; no speculative pass change shipped |
| Register allocation | interval weighting, occupied points, existing RA catalog | scoped performance work A14-24, not a new allocator rewrite |
| x86 codegen/headers | native artifacts, FP/intrinsic contracts, ignored comments vs live definitions | existing tests reused; no inferred header bug from obsolete TODO |
| i686 codegen/ABI | target-size vector reproducer; mixed-compiler ABI CI gate | A14-13; existing aggregate gate 390 executions passed |
| ARM codegen | integer popcount under no-FP/SIMD contract | A14-19, confirmed forbidden NEON emission |
| RISC-V codegen | explicit fallback/intrinsic paths and assembler/linker boundaries | A14-15/17; no RISC-V kernel boot or execution claim |
| Assemblers | ADR bounds, relocatable expressions, native debug directives | A14-14/15/21 with negative AND valid-boundary controls |
| Linkers/ELF | raw-name identity, unknown relocations, mmap/SSO/parallel write contracts | A14-17/31; raw pointer safety not formally verified |
| Driver | option capability honesty, -g intermediate/final mismatch | A14-16/21 |
| Runtime/PGO | profile context lifetime and generated file I/O; syscall trace | A14-08/20; flush occurs after unlocking |
| Build/restoration | actual fastbuild/-O1/-j2, active swap, storage budget and restore artifacts | A14-07/32; use existing hardened snapshot, not the older error-swallowing template |
| CI/tests/fuzz | local fast suite, script syntax, negative probe runner, corpus eligibility | A14-01/23/29/33; new defect probes intentionally report failures |
| Benchmarks/workloads | six filter runs; complete Expat/zlib runs; 3-source CE screen | 28/28 micro checks; gzip 30/30 each arm; Expat/zlib harness failures retained |
| Documentation/licensing | current vs historical claims, extraction identity, exact zlib notice | A14-05/06/27/28 and new reproducer provenance |

`script-syntax.json` records **551** shell/Python files at that checkpoint
(including the first two new audit scripts): `bash -n` / `ast.parse`, zero syntax
failures. Later additions were checked separately. This is not an assertion that
all 551 script algorithms were audited or executed. Semantically examined
script families include build/swap/snapshot/recovery, CI parity/census, CE/cache/
codegen, runtime paired/bench/callgrind runners and full-workload validation.
Every other script remains inventory/syntax-only unless a CI log shows execution.

## Reproduced contracts

Run `python3 scripts/audit_contract_probes.py --compiler-probes --extended --check`.
The original 29-case replay on the initial baseline produced **29 observations, 14 violated contracts** (several are
controls of the same defect), not 14 independent bugs. The expected nonzero
exit is an audit finding, not a broken new CI gate. Missing tools, crashes and
timeouts are represented separately from compiler rejection. Local diagnostic
oracles were Clang/lld 19.1.7 and GCC 14.2/bfd 2.44; do not confuse these with
the remotely queried requested competition revisions. Exact outputs, emitted
assembly, commands and tool versions are in `contracts.json` / `environment.json`.

The strongest additional reproducer is A14-30's zlib table generator, with
independent TU mixing and sanitizer controls; A14-31 has a byte-mutation ELF
fixture rather than an unexplained binary blob. No compiled scratch artifacts
are added to the patch.

## Measurement / oracle interpretation

The existing codegen oracle was used, not reimplemented. Whole-TU counts at
-O2 -march=x86-64-v3 were:

| Kernel | LCCC | GCC 16.2 | Clang 23.1.0 | ICX 2024.0.0 (CE latest) | ICC with v3 |
|---|---:|---:|---:|---:|---|
| gzip_crc32 | 73 | 50 | 82 | 100 | refused ignored option |
| zlib_ng_adler32 | 276 | 156 | 148 | 147 | refused ignored option |
| expat_siphash24 | 846 | 308 | 496 | 515 | refused ignored option |

These are **static TU instruction counts**, including setup/checking/main,
not hot-loop work-normalized numbers or runtime ratios. Different inlining and
call boundaries invalidate “best compiler” runtime inference. Stack references
are not automatically spills. The failed ICC records are preserved. A separate
ICC retry succeeded with its accepted `-march=core-avx2` spelling (gzip 60,
Adler 244, Expat 756 static instructions); that does not prove
feature-by-feature equivalence to x86-64-v3, and must stay separately labelled.

For optimization acceptance after fixing correctness: keep fixed driver and
inputs, retain all raw output-checked paired samples, calibrate >=200 ms/arm,
record affinity/ISA/compiler/library identities, use identical-arm controls,
then use Callgrind Ir only as a deterministic dynamic-work screen. Cachegrind's
simulated hierarchy is not the 14700KF PMU. Bare-metal, per-core-type PMU runs
remain required for claims about IPC, execution ports, caches or Raptor Lake.
Never measure throughput concurrently with this compiler/CI workload and call
it target performance. The gzip runner's one-round session run is correctness
coverage only, not statistical performance evidence.

## Audit disposition

The deliverable adds a specific 34-item queue and replay tools/evidence; it
**does not fix** the discovered compiler defects. The user's next task was an
audit for an implementation agent. A14-30 is deliberately at the front of that
agent's queue. Existing fast CI and newly discovered negative contracts have
different scopes: even a green fast suite cannot prove these defects absent.
The initial fast run had 216 pass / 1 fail (Clippy OOM) / 5 skip; A14-34
is reproduced independently. Swap increased from 2 to 6 GiB for the final run.
Full fast-CI completion/status is recorded by the delivered snapshot receipt,
not fabricated from a subset of passed tests. New compiler changes must add the
corresponding regression to ordinary CI rather than permanently quarantine it.


## Final-main rebase and scope reconciliation

At delivery refresh upstream advanced to 1e83776 (115 changed/renamed/deleted
paths versus the first base). The 217/0/5 fast-CI result from the earlier tree
is retained as **pre-rebase evidence only**, not as a stamp for the new tree.
The audit commits were replayed on the new main; no upstream implementation
commits were folded into the cumulative session patch.

- **A14-05 closed upstream:** local permissions/base markers and work/ scratch
  files are no longer tracked. The queue now has 34 entries, **33 open**.
- **A14-01 partially repaired:** current raw-string/line-preservation controls
  pass; test-suffix removal still deletes production, and a new control shows
  panic text inside an ordinary multiline string leaking into code counts.
  The replay was adapted to the upstream scanner's renamed API, not disabled.
- **A14-32 refined:** upstream publisher now excludes cache/checkout/artifact
  zones. Recovery artifacts still persist under this platform's rules; a
  separate whole-visible-workspace byte check remains necessary. Multi-file
  publication ordering remains nontransactional. Nothing in this audit weakens
  the platform budget to make publication pass.
- **A14-34 rechecked:** the new CI diagnostic wrappers do not change the
  order-dependent jobs decision; source-function replay still yields one
  automatic job in isolation and two after the unit-test gate.
- `inventory-rebased.json` is the immutable current-main inventory. Do not
  compare the new census's renamed metrics with old totals as if the scanner
  had not changed. Both source snapshots and their scope are explicit.

New compiler/workload checks are recorded in `rebase-validation.json` and
`contracts-rebased.json`. The standalone wrong-code reproducer, not an old
linked executable, is recompiled for the new compiler. The delivered final
CI receipt must name the rebased tree; the previous green receipt is insufficient.


### Executed final-base revalidation

All following checks use a freshly built compiler from main 1e83776, not the
previous executable. The full-source receipt is `rebase-validation.json`.

- Full gzip repeated: **30/30 upstream tests in each arm**, six exact compressed
  streams; one-round timings discarded as statistically invalid.
- Full Expat repeated: **4,932 checks in each arm pass**; the runner still exits
  1 in JUnit postprocessing on status=run, exactly A14-29.
- Full zlib-ng repeated: **68/69 CTests**, only maketrees fails; round-trip phase
  returns 0. Full three-TU isolation and the standalone sanitizer matrix are
  repeated: -O2/-O3 wrong, -O0/-O1 and no-widen correct.
- Six kernel filters repeated: **28 pass, zero fail/skip**, across -O0..-O3.
- Current contract replay: **30 observations / 13 violations**. Two old scanner
  controls now pass, but the added multiline-body control reveals a remaining
  tokenizer defect. This is not 13 new independent compiler bugs.
- Non-UTF-8 symbol identity repeated: bfd/lld link and exit 0; LCCC falsely
  rejects the two distinct names. No silent mislink is asserted.
- The CI-memory replay still distinguishes isolated -j1 from inherited -j2.

Final fast-CI status remains bound to the delivered final tree receipt, not the
pre-rebase 217/0/5 result. Initial JSON/XML evidence is retained for traceability;
new results have separate filenames rather than silently overwriting history.
