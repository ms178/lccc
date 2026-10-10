# 2026-10-10 — GCC 17 development torture, native x86-64 / i686

## Scope and evidence rules

This campaign starts at upstream `ms178/lccc` main
`a941096869e686b66cb556de9e0f8e1f9ce8d636`. The target is correctness of the
compiler **plus integrated assembler plus standalone `lccc-ld`**, before any
claim about generated-code performance. Intermediate recovery snapshots are
explicitly UNGATED; they are not CI clearance. Final validation belongs to the
exact source tree named in the CI stamp and delivery receipt.

The GCC development corpus is revision
`bd261b39dedc064209a14be95ffb456f5cd01c1d` (BASE-VER `17.0.0`), resolved from
upstream master on 2026-10-10. The provisioner verified 24,089 selected files
against Git objects; selected-content SHA-256 is
`b74eb8214d4f3b5a112514e5f66a66afb14d16a7feeeaf7c931d5d33a07675b9`.
There are 1,698 top-level execute sources and 2,016 top-level compile sources.
No GCC source is vendored in this patch. Reproduce acquisition with:

```sh
python3 scripts/ensure_gcc_torture_git.py \
  --ref bd261b39dedc064209a14be95ffb456f5cd01c1d
```

**This is not a full DejaGnu result.** Native top-level `.c` coverage does not
cover all target selectors, nested directories, diagnostic suites, `.S`, LTO,
or all optimization combinations. In particular `pr96998.c` has an ARM /
AArch64 target selector: inclusion by this runner is not evidence of a native
x86 regression. GCC-internal `__GIMPLE` input is not ordinary C. Explicit
reference skips and target restrictions must stay visible, not turn into
passes. Reference eligibility uses installed GCC 14.2, **not** a local GCC 17
compiler; Compiler Explorer GCC 17 is a separate oracle.

## Environment and resource policy

- Debian 13, KVM, approximately 1.9 GiB RAM and two logical CPUs; reported Xeon
  model 106 / 2.60 GHz. This is **not** the intended i7-14700KF target.
- Active 4 GiB swap; work/cache on the largest available filesystem.
- Rust 1.99.0 (`b940084d7`, 2026-09-28). Compiler builds use only the
  `fastbuild` profile, opt-level **1**, **two Cargo jobs**, no LTO. Reduced
  test debuginfo is permitted; release/O3 compiler builds are not.
- GCC multilib installed; a native 32-bit compile/link/run canary passed.
- No hardware-PMU access assumed. Shared-VM wall times and static instruction
  counts are not physical Raptor Lake performance measurements. Baseline
  elapsed times below are workload accounting, **not** speed comparisons.

## Completed, frozen original baseline

Original binaries and original native runner were frozen before source fixes.
All four legs used `-O2`, two worker jobs, and the same corpus. Execute legs
linked through standalone `lccc-ld`; compile legs assembled to ELF objects.

| Target / mode | Cases | Pass | Candidate compile failures | Reference compile skips | Reference run skips | Unsupported | Seconds |
|---|---:|---:|---:|---:|---:|---:|---:|
| x86-64 execute | 1698 | 1685 | 1 | 1 | 6 | 5 | 139.0 |
| x86-64 compile | 2016 | 1955 | 43 | 17 | 0 | 1 | 234.4 |
| i686 execute | 1698 | 1683 | 1 | 4 | 5 | 5 | 247.7 |
| i686 compile | 2016 | 1950 | 40 | 25 | 0 | 1 | 197.0 |

Evidence is in the delivery workspace under
`artifacts/evidence/baseline-{arch}-{mode}-O2.{json,log,failures.log}`,
`baseline-binaries.sha256`, and `baseline-build.txt`. Preserve these original
reports; use separately named targeted retries for timeouts and later fixes.
The old runner's limitations are not retroactively erased by its replacement.

## Completed candidate matrix (compiler fixes through S06)

The expanded native run is complete; it is **not an all-green compile suite**.
Each execute source ran at O0/O1/O2/O3/Os through the standalone linker.
Compiler binaries were unchanged during all four legs and are SHA-256-recorded.

| Target / leg | Selected cases | Pass | LCCC compile failure | Reference failure | Reference skip | Unsupported |
|---|---:|---:|---:|---:|---:|---:|
| x86-64 execute, five levels | 8490 | 8442 | 0 | 0 | 23 | 25 |
| i686 execute, five levels | 8490 | 8430 | 0 | 0 | 35 | 25 |
| x86-64 compile, O2 | 2016 | 1957 | 41 | 3 | 14 | 1 |
| i686 compile, O2 | 2016 | 1955 | 35 | 3 | 22 | 1 |

Thus **16,872 eligible native executions passed**, with no LCCC compile,
link or run failure in the execute legs. The other 108 execute cases remain
explicit skips/unsupported, not passes. At O2 there is no pass-to-fail regression
against the frozen baseline. Besides `920302-1.c`, K&R handling unblocks
`pr51694.c` and `pr99324.c`; the i686 fix unblocks `pr17906.c` / `pr35432.c`.
The newly passing `limits-externalid.c` is resource-sensitive and is not
attributed to a compiler optimization.

The three reference failures per compile leg are GCC14.2 ICEs in
`pr117358.c`, `pr123365.c` and `pr123703.c`, previously mislabeled as skips by
the original runner. The hardened runner now fails them explicitly. Evidence
summaries separately count reference failures without calling them LCCC bugs.
`20050622-1.c` (72-MiB by-value aggregate) and `pr46534.c` (20-MB expanded
string literal) hit the 60-second budget; isolated, longer-budget diagnosis is
separate from the completed matrix and will not overwrite its results.

## Tooling fixes already validated and saved

1. **Pinned GCC development-suite provisioning.** Sparse acquisition, exact
   revision/content verification, atomic replacement, damaged-cache recovery,
   five offline contracts. The release-16 provisioner remains available.
2. **Native torture evidence contracts.** Bounded shared subprocess execution
   with working-directory support; reference infrastructure/signal/ICE errors
   fail closed; multilib preflight; minimal ELF identity validation; atomic
   incomplete/progress/final reports; provenance and selection hashes;
   all-skipped selections are not success. Twelve native-runner, twelve
   process/publication, and 32 Callgrind contracts passed. Filtered standalone
   link smoke: x86-64 6/6, i686 4/4. These are **filtered tests**, not the full
   suite. Import/document/diff checks passed at those checkpoints.
3. **Binutils release identity.** The SHA-256-pinned binutils 2.47 archive
   genuinely reports `2.47.20260726`. Accept exactly this banner or plain
   `2.47`, not arbitrary datestamps, patches, dirty suffixes, or `2.470`.
   Linker setup now verifies the archive before building, using the same pin
   as GAS provisioning:
   `154ab23b60070e8f27013c22977f1129425d67d1e8acd6e13010e617811e4cff`.
   Positive/negative version contracts, shell syntax, and the actual built
   BFD binary passed. Mold still requests exactly 2.42.1 and the explicit
   `-DMOLD_TARGETS='X86_64;I386'` source-build preset.

## Compiler investigation: K&R versus prototype

Both execute baselines reject `920302-1.c`: the parameter declarations inside
an identifier-list function definition were treated as a visible prototype.
That imposes a pointer-compatibility constraint at a call that has no
prototype. Simply disabling pointer checking for all functions would hide
real errors and is not an acceptable fix.

The validated fix keeps **body parameter types for lowering/ABI** separate
from an optional **call-site prototype**. A prior prototype survives a K&R
body; a subsequent declaration refines the callable type without overwriting
the body's narrow parameter types. Callable resolution uses lexical symbols,
including function pointers and nested functions, rather than the TU-wide
body metadata table. An unspecified-arguments redeclaration must not erase
an existing nonempty prototype. Tests cover pointer constraints, arity,
prior/later declarations, direct/indirect and nested scopes, and default
float/integer promotions. Acceptance-only tests with mismatched arguments
are not executed as defined-C examples.

There is a separate pre-existing representation debt: empty `f()` versus
`f(void)` and pre-C23 versus C23 modes are not fully distinguished. This
change must not be described as complete C function-type conformance. The
existing record-tag negative/prototype gate remains mandatory.

Remote code-generation oracle requests for the promotion regression
succeeded with GCC 16.2, Clang 23.1.0, ICC 2021.10.0, ICX latest, and GCC trunk
(`-O2 -std=gnu17`). Those requests do not execute code. GCC can constant-fold
this self-check; its smaller static count is **not** a speed result. Raw
oracle identities and assembly are in `artifacts/evidence/kr-oracles/`.
The no-local Markdown rendering labels the absent local row `ERROR`; the JSON
contains the actual remote records and must not be mistaken for an LCCC run.

Validation: fastbuild opt-level 1 / jobs 2, eight semantic unit tests,
20/20 filtered native execute cases (two sources × five levels × two targets,
standalone `lccc-ld`), and the existing 50-row record/prototype differential
gate all passed. The two sources are GCC `920302-1.c` and the new
`kr_prototype_promotions.c`. Test levels are O0/O1/O2/O3/Os. The new regression
uses a volatile input so constant folding cannot mask the ABI failure.

That runtime regression also found and fixed a **pre-existing miscompile**:
when a prototype followed a K&R definition, `register_function_meta` reused
the body's `float` CType for `f(double)`. Calls passed F32 but the body
expected promoted F64. The original frozen compiler exits 2 at **all five
levels on both targets**; after the lowering fix all ten executions pass.
Keep prototype types and body types separate in IR metadata too. The initial
failed candidate reports and original-compiler reproduction are retained;
they are not overwritten by the final passing reports. Full native-suite and
CI clearance were separate outstanding gates at that checkpoint; the expanded
matrix is now recorded above, while CI clearance remains outstanding.

Oracle provisioning also completed: native BFD **2.47.20260726**, mold
**2.42.1** built with **X86_64;I386**, and LLD **23.1.2**. GAS / objdump
**2.47.20260726** are installed for both x86-64 and AArch64; the SHA-pinned
Arm A64 XML **2026-09** package is installed. Exact linker banners are in
the persisted evidence file `linker-oracles.lock`. These are provisioning results,
not an assertion that every oracle comparison has passed.

## Compiler fixes: empty aggregates and i686 fastcall

`pr17906.c` and `pr35432.c` no longer panic in parameter capture. A zero-size
GNU aggregate can have an addressable local object but contributes no incoming
bytes or registers; the callee now skips the copy, matching caller classification.

The new **cross-compiler** regression found an additional ABI defect which
same-compiler tests missed: an empty aggregate incorrectly broke the i686
fastcall register chain. LCCC caller and callee agreed on the wrong convention;
a GCC callee expected the next integer in EDX and `ret $4`, while LCCC placed
it on the stack and used `ret $8`. Only nonempty aggregates break the chain.
Variadic calls remain stack-only, and an empty argument cannot reopen a chain
already broken by a nonempty aggregate / wide integer.

Validation: four focused Rust tests passed (three new layout contracts),
20/20 GCC-source compile/assemble cases across both targets and five levels,
10/10 new standalone-link native executions, and **20/20 mixed-compiler
executions** (LCCC caller / GCC callee and the reverse, m64/m32, five levels).
The fixture checks cdecl, regparm(3), fastcall (including empty first/middle
arguments), and varargs, with addressable empty objects. Reproduce the ABI
comparison with `bash tests/regression/check_empty_aggregate_abi.sh`.
The original ICE reports, initial cross-ABI failure and before-fix assembly
are retained. An initially invalid fixture omitted ABI attributes on its
definitions; GCC rejected it, the runner correctly refused an all-skipped
success, and the fixture was corrected before any pass was claimed.

## Bounded aggregate calls and guard-page / cdecl defects (after S08)

The isolated 180-second retry confirmed that `20050622-1.c` was not merely a
busy-host timeout. Its 72-MiB by-value argument drove expansion toward
9,437,184 x86-64 pushes (and still more i686 scalar copy instructions), with a
sampled compiler RSS above 1.5 GiB. Argument copies at or above 2,048 bytes now
use a bounded, exact-size REP copy. Temporary register saves are below the
outgoing argument and do not create ABI holes. i686 additionally saves ESI/EDI
from function entry so unwind rules remain valid while REP owns those registers.
Small-copy policy stays otherwise unchanged; this is not a blanket threshold
retuning or an unmeasured throughput claim.

The guard-page regression exposed and fixes **three further existing bugs**:

1. x86-64 partial stack words read a full eight bytes past 1–7 actual tail
   bytes. A 17-byte object ending at a protected page faults on the frozen
   original compiler; tail copies now use exact 4/2/1-byte pieces.
2. i686 fastcall passed the padded slot size to the source copier, reading
   three extra bytes for a 2,049-byte object. Slot reservation and readable
   object size are now separate.
3. An i686 regparm caller making a cdecl call retained its own register count
   when the callee's effective count was zero. Classification selected
   registers while both writers skipped the argument. Zero now overrides the
   caller just like 1/2/3; a small independent regression fails on the original
   compiler and passes for all four TU `-mregparm` defaults.

Validation at this checkpoint:

- **390/390 native ABI executions**: 13 sizes (all partial-word tails, both
  sides of the bulk threshold, and 4,097 bytes), five levels, two targets,
  LCCC/LCCC and both GCC/LCCC directions. Includes guarded source boundaries,
  two aggregates, live scalar/double neighbors, over-aligned local sources,
  direct/indirect calls, regparm and fastcall.
- **10/10** native standalone-link executions of the default 2,049-byte case.
- **20/20** regparm-default transition executions, standalone linker.
- **10/10** compile/assemble cases for the original 72-MiB GCC test, both
  targets and five levels. Focused O2 runs take 0.11 s each here, max RSS
  163,048 / 161,992 KiB, with `.text` 58 / 64 bytes and the correct 72-MiB BSS.
  These are compiler-resource observations, not application speedups.
- The existing **20/20** empty-aggregate cross-ABI gate still passes; the
  x86-64/i386 forced-unwind gate passes. Broader CFI/CI clearance is separate.

Initial failing test runs and the original-compiler SIGSEGV / wrong-result
controls are retained. The 20-MB string-literal stress `pr46534.c` remains a
resource failure (one x86 retry was killed with signal 9, not called a proven
compiler crash without a kernel diagnosis). This change does not solve
logical-size NOBITS, giant stack displacements, or every callee-copy path.
The final all-corpus/CI results must be read from the final evidence/receipt,
not inferred from the earlier S06 matrix.

## Triage queue — preserve negative results

### P0: correctness / crashes / memory use

- **Incomplete tentative records:** `930525-1.c`, `pr85401.c` declare an
  uninitialized file-scope object before completing its record type. Current
  sema diagnoses storage too early. Defer the appropriate tentative-definition
  obligation until translation-unit end, but retain local/initialized-object
  diagnostics and tag-scope identity. An incomplete array element is a
  separate immediate constraint, not a licence to accept it.
- **Closed under focused gates:** i686 empty-aggregate ICE and fastcall ABI
  chain handling; see the compiler-fix section above. Full-suite coverage is
  still independently required.
- **Length-only NOBITS:** `20010518-2.c`, `20080625-1.c`, `pr103813.c`, and
  `pr65680.c` hit intentional 256 MiB/directive / 512 MiB aggregate materialized
  fill caps. Do **not** remove the caps. Separate logical section length from
  resident bytes; audit labels, common allocation, align/org, relocations,
  ELF32 bounds, and shared serializer across all backends. Nonzero NOBITS
  stores must remain errors. See the existing fail-closed follow-up.
- **Huge x86 frames:** `20031023-{1,2,3,4}.c`, `stack-check-1.c` use stack
  adjustments beyond signed imm32. A fix must handle slot addressing and
  synthesized CFI, not merely replace `subq`/`addq`. Allocating a 1 TiB frame
  is a compile-only case, not a sensible runtime test on this host.
- **Large-input compile time:** `20050622-1.c` times out; i686 additionally
  timed out on `limits-externalid.c` and `pr46534.c` under build contention.
  Retry in isolation before attributing a regression. Preserve finite budgets.

### P1: language features and harness eligibility

- Fixed-underlying C23 enums (`enum e : bool`) fail parsing, including
  `pr111059-{7..12}.c` and `pr111911-2.c`. Direct GNU23 reproduction confirms
  the colon/underlying-type syntax, not just a bool spelling problem.
- Implicit-int file-scope variable declarations (`20101216-1.c`, `pr72802.c`,
  `pr90275.c`): parser fallback is restricted to identifier plus `(`.
  Several historical sources use `-fpermissive`. Implement correct mode /
  diagnostic policy rather than broadly weakening valid constraints.
- `__builtin_shufflevector` in `pr108892.c` reaches an internal error for an
  unsupported lane/argument count; fail with a user diagnostic or implement
  lane-correct lowering. The current four-lane-only assumption is insufficient.
- Remaining pointer/arity failures must be classified by actual prototype,
  mode/`-fpermissive`, and target eligibility. Do not lump all into miscompiles.
- `limits-*` parser limits are bounded-resource refusals, not crashes. Raising
  limits blindly would trade controlled failure for stack/memory exhaustion.
- Parse and honor target requirements safely. Do not pretend a partial Tcl
  parser is DejaGnu; unknown selectors should be explicit, reviewable coverage
  exclusions or delegated to the real harness.

### Performance / real workloads / structural audit

- Sparse `archpkgbuilds` recipes inspected: gzip 1.14, zlib-ng 2.3.3, expat
  2.8.5, SQLite 3.54.0. Builds and workload execution are still required.
- Reuse `codegen_oracle.py`, `perf_ab.py`, `callgrind_ab.py`, workload scripts
  and the existing 39-benchmark corpus. Add representative extracts only with
  source/revision/licence and meaningful correctness checks.
- Existing promising work: vectorization breadth in moving statistics / byte
  streams / FP reductions, aggregate SROA, and ring-buffer invariant forwarding.
  Verify current code first; earlier gzip-CRC work already measured instruction
  parity and must not be repeated as a speculative win.
- Every claimed optimization needs per-case before/after data, verified output,
  assembly explanation, counters if available, and a physical-target follow-up.
  Investigate >1%, strongly investigate >2%, reject >5% regressions unless
  justified; a geometric mean cannot hide a regression.
- Scripts inventory: 192 tracked files / 55,379 lines, hashes, Python
  summaries, and shell syntax results in `scripts-inventory.json`. This is
  **not** full semantic comprehension of every script or a full codebase
  audit. Continue frontend, IR, passes, ABI, assemblers, ELF/linker, measurement,
  recovery, and CI audit with explicit coverage notes.

## Workload / oracle research update

A new integrated `expat_siphash24` benchmark preserves Expat 2.8.5's CC0
SipHash implementation and 64 known-answer vectors, adding exhaustive
streaming splits and a carried-state, variable-message workload. Native
LCCC/GCC/Clang correctness agrees at O2, and the reduced-iteration five-level
matrix agrees on both x86-64 and i686 (30 compiler executions). The source/version/license and the
archpkgbuilds checksum discrepancy are documented in
`tests/benchmark/WORKLOAD_PROVENANCE.md` and
`tests/workloads/expat-2.8.5/README.md`. The upstream signature was checked
against the pinned publisher key; no checksum bypass is used.

This work found a real performance *gap*, not supremacy: at matched O2/v3
and 200,000 workload iterations, Callgrind reports 442,526,918 versus
267,107,196 simulated instructions (LCCC/GCC14.2 = 1.656739). Five remote
codegen oracles also completed. Static whole-TU counts are LCCC 707,
GCC16.2 308, Clang23.1 496, ICC2021.10 745, ICXlatest 515, GCCtrunk 307.
These are not hardware cycles or wall-clock speed ratios. The round loop
retains ten loads/ten stores, and byte-word assembly remains eight separate
loads/shifts/ORs. Safe adjacent-byte folding and aggregate promotion need
proofs, focused tests, full correctness coverage and paired workload data
before enabling any change.

The bounded, pinned full-project Expat runner now passes: both GCC and LCCC
build the static C library, examples, xmlwf and the complete configured C test
suite. Each upstream run reports **4,932 checks, zero failures**; five nonempty
canonical XML outputs agree byte-for-byte and three invalid XML inputs return
the exact rejection status. An initial metadata mistake mentioned C++ even
though CMake compiled only C; unused CXX settings were removed and the entire
gate rerun cleanly. Both runs and the erratum are retained.

Native Clang **23.1.2** is now installed in addition to the original Clang19.1
used in the first SipHash correctness matrix. Remote Clang23.1 codegen records
were already available. Gzip/zlib-ng full-project gates are still pending.
The SipHash kernel remains separately scoped; it is not an XML throughput
measurement. Its 268 detailed evidence files are SHA-verified in the persisted
SipHash evidence archive to respect the recovery file-count budget.

## Recovery and final acceptance

After each validated fix, commit and immediately run `scripts/lccc-snapshot.sh`.
The persisted workspace must retain cumulative `ms178-1.patch`, its ZIP,
source archive, session bundle, named checkpoint, delivery receipts, evidence,
and snapshot ledger. Bulk trees, compilers and toolchains live in excluded
cache storage; the archives, not those symlinks, are the recovery source.

Final acceptance still requires: remaining compile-leg triage, targeted compiler /
assembler / linker regressions, `scripts/ci_local.sh --fast` green on the
actual tree, deep negative-case audit, workload/performance evidence without
inflation, and a fresh upstream-main check/rebase. The snapshot policy also
requires the slow CI half for a fully GATED publication; a fast-only pass is
not full hosted-CI equivalence. Document unavailable oracle pins honestly.


### Additional script/CI audit corrections

The native runner no longer requires Python3.11-only `hashlib.file_digest`,
records its own source hash, and filters credential-bearing feature environment
names out of published provenance. Fifteen offline runner/evidence contracts
pass. The evidence table keeps reference failures fail-closed but separates
them from compiler defects. The empty-aggregate cross-compiler gate is now
wired into both local and hosted CI; parity reports 149 commands. The fast-CI
header was corrected to name all **five**, not three, skipped slow gates.
These are focused semantic reviews, not a claim to have comprehended every
line of the 192-script inventory.
