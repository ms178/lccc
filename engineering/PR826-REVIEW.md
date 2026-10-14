# PR #826: critical audit and implementation handoff

> **Historical review:** This documents the pre-fix implementation and its then-current evidence.
> [The empirical red-team follow-up](PR826-REDTEAM.md) supersedes the blanket
> oracle-agreement recommendation, narrows NONE compatibility claims, and records
> the implemented fixes and real test results. Do not apply this old grade or
> merge checklist to the current branch without reading that follow-up.
> [The newer upstream/three-way audit](PR827-COMPARISON.md) supersedes both older
> snapshots for the reconciled implementation and its actual compiled findings.

## Decision

**Do not merge the current implementation yet.** The direction is good and the
PGO lock-lifetime fix is particularly well chosen. However, the input-section
safety boundary remains incomplete for variable-length relocations. Fix R1,
add its adversarial and positive controls, and require the relevant GitHub CI
steps to pass before reconsidering merge. Complete R2–R8 for the review's
10/10 target. R9–R10 are explicitly inherited adjacent debt: do not silently
expand this delivery to fix them, but track them and narrow the stated contract.

This audit did not compile the compiler, fixtures, or any test program. It used
source inspection, Python fixture enumeration, JSON consistency checks, a
Python model of the existing ULEB algorithms, and compiler-independent CI
configuration checks. The model demonstrates algorithmic behavior, **not an
executed LCCC reproducer**. No claim is made that this review proves the absence
of other defects.

The report audits the original implementation delta. Its addition to the branch
is documentation-only; none of the production findings below has been fixed by
adding this report.

## Scores and scale

Scores are reviewer judgments, not measured code coverage or defect probability.
10 means all stated contracts have independent positive/negative evidence,
clear failure semantics, maintainable implementation, and a passing CI result.

| Dimension | Score / 10 | Weight | Reason |
|---|---:|---:|---|
| Correctness and safety | 6.5 | 35% | Fixed-width validation and PGO locking improve safety, but ULEB writes still escape the input contribution. |
| Regression design | 6.0 | 25% | Good deterministic ELF and fault fixtures; narrow valid coverage and partly self-referential PGO expectations. |
| Maintainability | 7.0 | 15% | Readable localized changes, but independent relocation tables and hand-allocated generated IR need invariant checks. |
| Evidence/reproducibility | 4.5 | 15% | Honest performance caveats, but substantial baseline-only evidence, moving aliases, missing identity/transcripts, and an unfinished follow-up note. |
| Scope and integration | 6.0 | 10% | Both gates are mirrored correctly, but unrelated codegen surveys dominate the delivery. |
| **Weighted overall** | **6.1** | **100%** | Not merge-ready against the advertised safety contract. |

**Code quality: 7/10. Engineering/delivery quality: 6/10.** The PGO production
change alone is approximately 8/10; relocation safety is approximately 6/10.
Those component scores are qualitative, not additional weighted dimensions.

### Quantified observations

- Original delta: **59 files; 23,223 added and 62 deleted lines**.
- Production code: 3 files, 238 additions / 62 deletions. Tests: 3 files,
  440 additions. CI integration: 2 files, 14 additions.
- Evidence: **50 files, 22,497 added lines, 606,118 bytes**: 96.9% of additions
  and 84.7% of changed files. The size is manageable; relevance and provenance,
  rather than repository capacity, are the criticism.
- Relocation generator: **86 fixtures = 172 LCCC link invocations** across
  executable/shared modes. These comprise 78 rejecting fixtures and 8 accepting
  fixtures (156 rejection invocations / 16 acceptance invocations).
- Accepting fixtures cover only **3 of the extent table's 39 kinds**:
  `R_RISCV_64`, `R_RISCV_NONE`, `R_RISCV_RELAX`. That is 7.7% kind coverage,
  **not source-code coverage**. There are 18 unknown-kind fixtures, 10 bad-symbol
  fixtures, 50 bad-extent fixtures, and 8 positive fixtures.
- PGO test has six optimization/size configurations (`-O0/-O2/-O3` × 1/96
  functions), each recording ten high-level checks: **60 intended checks**.
  These are enumerated intended checks, not newly executed results.
- Supplied relocation summaries are internally consistent: historical baseline
  **105 pass / 67 fail**, supplied after-state **172 pass / 0 fail**. They contain
  boolean records, not full invocation/output provenance. This audit did not
  reproduce those runs.
- **8 scoped findings** below: 1 P1 merge blocker, 5 P2 follow-ups, 2 P3
  maintainability/harness follow-ups. **2 additional inherited-debt findings**
  are separated from the scoped grading. No P0 finding.

## Choices I agree with

1. **Unsupported shared-link relocations must diagnose, not disappear.** Collecting
   RELATIVE records is not a reason to ignore an unrelated relocation; the old
   catch-all emitted no replacement dynamic relocation.
2. **Validate fixed-width extents against the input section before merging.**
   A merged-buffer check cannot distinguish a legitimate write from corruption
   of the next contribution. The subtraction-based bounds expression avoids
   overflowing `offset + width`.
3. **Place validation before GOT/TLS scans in both link entry points.** This
   prevents downstream indexing of invalid relocation symbol indices for objects
   that actually reach these entry points.
4. **Use checked offset/address sums and host-size conversions.** These errors
   should become diagnostics, not wrapping indices or host-dependent behavior.
5. **Hold the PGO flock through `fclose`.** Closing releases the lock after the
   stream's buffered flush. Removing an explicit early unlock fixes the actual
   ordering error rather than relying on a larger buffer or probabilistic stress.
6. **Fail closed on unsuccessful lock acquisition.** Abandoning a best-effort
   dump on EINTR is a defensible policy. A bounded retry would be another policy,
   not automatically a correctness requirement. Do not write unlocked.
7. **Check both stream error and close result, preserving application status.**
   Earlier buffered writes and the last flush are distinct failure points.
8. **Correct `fopen` to non-variadic and advance generated block allocation.**
   Fresh SSA results and a next-label value above generated blocks are sound.
9. **Keep standalone gates mirrored in hosted and local CI.** The existing
   compiler-independent parity checker confirms integration.
10. **Label instruction/density surveys as screening rather than speed claims.**
    Keep that honesty, while improving artifact relevance and identity.

## R1 — P1: variable-length relocation writes escape the validated contribution

**Locations:** `src/backend/riscv/linker/reloc.rs:58–72, 99–100, 511–515,
590–645`.

**Classification:** confirmed algorithmic defect, inherited writer behavior;
new validation explicitly includes these kinds but does not close their safety
boundary. Not a claim that this PR introduced the old ULEB encoder.

`SET_ULEB128` and `SUB_ULEB128` receive an extent of one byte. The application
helpers then decode/encode using `data.len()`, which is the whole merged section.
The input section/contribution's end and the original encoded field length are
not passed to them. A valid starting byte therefore does not prevent either
operation from touching subsequent input bytes.

A direct model of the unchanged writer demonstrates:

- One-byte field `00`, followed by adjacent bytes `a5 a5 …`.
- SET symbol value `0x1000` writes `80 20`, already overwriting the neighbor.
- SUB symbol value `0x0fff` produces the valid final difference `1`.
- Result starts **`01 20 a5 …`**; the neighboring byte should still be `a5`.

A separate padded-field case `80 00 a5`, SET value 1, produces `01 00 a5`,
shortening the field rather than preserving its two-byte encoding. The decoder
also increments a shift without bounding it to the supported integer width;
long continuation strings can reach an oversized shift or consume adjacent data.

The RISC-V ELF psABI requires the SET/SUB pair to produce a **single final value**
and requires the allocated ULEB length to be preserved. Intermediate absolute
label values need not fit the final difference's reserved field. Reference:
<https://github.com/riscv-non-isa/riscv-elf-psabi-doc/blob/master/riscv-elf.adoc>,
`uleb128-note` and the linker-relaxation ULEB128 paragraph. The text was retrieved
through GitHub's Contents API for this review.

**Instructions to implementing agent:**

1. Carry the input contribution boundary into relocation application; never
   expose the following contribution as available field storage.
2. Determine the original encoded field's reserved length within that boundary.
   Reject unterminated/unsupported-width encodings with an object/section/offset
   diagnostic; do not silently break at the merged buffer end.
3. Process an adjacent same-offset SET/SUB pair as one operation using the
   appropriate psABI arithmetic. Validate the pair structure and final result
   before writing; do not encode the intermediate absolute SET address.
4. Preserve reserved length, padding a shorter final encoding correctly.
   Diagnose a final value that does not fit. Bound all shifts and conversions.
5. Make these routines return `Result`, with no partial mutation on validation
   failure. Reject an unsupported sequence explicitly rather than approximating it.
6. Add positive padded encodings and large-absolute-label/small-difference cases;
   add negative boundary, unterminated, excessive-width, insufficient-capacity,
   and malformed-pair cases in both link modes. Include a separate-object
   contribution and an adjacent sentinel, not just two sections in one object.

**Acceptance:** the above `0x1000 - 0x0fff` case links to a one-byte `01` with
sentinel unchanged; a two-byte field for 1 remains `81 00`; no decoder reads or
writer writes outside the original field/contribution; all invalid cases
produce diagnostics, not crashes, silent truncation, or published wrong images.

## R2 — P2: NONE semantics stop at the shared parser

**Locations:** `reloc.rs:83–85, 135–137`;
`src/backend/linker_common/parse_object.rs:344–354`;
`tests/linker/check_riscv_relocation_contract.py:121–123`.

**Classification:** confirmed mismatch between backend intent and end-to-end
behavior, due to inherited shared-parser behavior. A legal-semantics claim has
not been tested with the inputs that distinguish it from normal relocations.

The backend says NONE has no symbol/storage field and skips it. But the parser
first rejects **every** relocation with `r_offset >= target_size`, before the
backend sees NONE. A zero-sized target section with an offset-0 NONE record,
for example, cannot reach the newly introduced skip. The positive fixture uses
a populated section and an existing symbol table, so it cannot establish the
comment's stronger “even without a symbol table” promise. Whether unusual NONE
inputs must be accepted should be resolved against the ABI and a real linker,
not assumed from a comment.

**Instructions:** add zero-sized section, no-symbol-table, and deliberately
irrelevant symbol/offset NONE fixtures. Confirm intended accepted/rejected
semantics against the psABI and provisioned real linker. If NONE's no-operation
semantics permit those inputs, exempt it at the parsing layer using an explicit
architecture/type policy, preserving bounds checks for real writes and all
other architectures. If support is intentionally narrower, document that limit
and remove the stronger promise. Do not conflate NONE, RELAX, ALIGN, and
TPREL_ADD; their semantics differ.

**Acceptance:** documented parser/backend policy is consistent; fixtures exercise
it end to end; real writes remain strictly bounded.

## R3 — P2: positive coverage is too narrow and CI leaves the oracle unused

**Locations:** relocation test `cases()` and `main()`;
`.github/workflows/ci.yml:250–254`; `scripts/ci_local.sh:341–343`.

**Classification:** introduced coverage/integration gap, not proof that all
untested relocation kinds are broken.

Only 3/39 extent-table kinds have positive controls. Most tests prove rejection,
not that valid instruction/data relocations still work at their last legal
starting byte. ULEB pair handling, NONE without symbols, archives, separate input
objects, and ignored non-alloc sections are absent. The CLI supports `--oracle`,
but neither new gate passes it. Hosted CI already installs `lld`; the new
relocation fixture contract therefore needlessly has only one implementation's
verdict in its gate. The local parity check passing does not mean this oracle
is exercised.

**Instructions:** add a table-driven matrix for every supported width/kind,
including exact-end valid fields and one-byte-overrun negatives. For instruction
kinds assert output bits and preserve surrounding bytes; for shared dynamic
relocations assert exact entries, not only presence of one expected entry.
Add separate-object and regular/thin-archive cases. Distinguish discarded-section
policy from retained-section safety. Wire a probed RISC-V-capable `ld.lld` via
`--oracle` in both mirrors; make inability to execute the required oracle an
infrastructure failure. Record its version. Match rejection classes rather than
counting any arbitrary nonzero oracle exit as agreement.

**Acceptance:** all 39 kinds have a positive control or an explicit documented
reason for not being applied; new boundary matrix passes; both gate commands run
the oracle; probe/diagnostic mismatch/timeout is never counted as valid rejection.

## R4 — P2: PGO expectations are partly self-referential

**Locations:** `tests/regression/check_pgo_io.py:68–79, 115–133`.

**Classification:** introduced regression-quality gap.

The test's expected stdout and baseline counters come from the same instrumented
LCCC binary it is testing. Doubling those counters checks aggregation, but not
whether they were correct in the first place. The production-reader check is
valuable, yet still agrees with that same baseline. A consistently omitted or
wrong counter can survive this test. For the fixed generated source and
`argc == 1`, independent stdout is known: `1\n` for one helper and `-4464\n`
for 96 helpers. Function entry counts should be one per executed function per
process, and two after the two-writer merge. Value-profile records and multiple
instrumented translation units/DSOs are not covered here.

**Instructions:** assert analytically derived stdout and absolute entry/known
branch counts before using the byte-level golden result. Retain `raw * 2` and
production ingestion checks, but name them aggregation/serialization checks.
Add a small independent host-compiler executable oracle if useful. Add a
multi-translation-unit destructor case and an indirect-call value-profile case;
verify record identity rather than merely total function count. Harden the
counter parser for those records.

**Acceptance:** a deliberately wrong baseline count or stdout fails even when
single, merged, and profile-use executions are mutually consistent. Exact entry
counts equal 1/2 as appropriate, value-profile records survive ingestion, and
multiple helpers serialize without interleaving.

## R5 — P2: ferror is not an errno-producing operation

**Locations:** `src/pgo/instrument.rs:1097–1133, 1524–1551`;
`tests/regression/fixtures/pgo_io_interpose.c:81–85`.

**Classification:** diagnostic correctness gap in the newly introduced error
branch. Not a data-race finding and not a reason to revert the lock fix.

The new `ferror` path calls `perror`. `ferror` reports a sticky stream flag; it
does not identify or set errno for the original failed write. Successful calls
after that write may leave errno stale or change it. The test shim always sets
`errno = EIO`, masking the problem. A legitimate write failure can therefore
produce a misleading suffix, including “Success.” Immediate fopen/flock/fclose
failure diagnostics are a different case and may use their reported errno.

**Instructions:** either capture the first failing write's return/errno promptly
and preserve it through cleanup, or emit a stable generic stream-write diagnostic
without an untrustworthy errno suffix. Do not fabricate an errno from `ferror`.
Add an injected sticky stream error with errno zero and another with unrelated
errno; verify accurate classification and unchanged application status. Add
operation logging proving no profile write after failed lock acquisition and
exactly one close on each successful-open path. Document that failure handling
is best-effort, not transaction rollback: partial earlier records may remain.

**Acceptance:** no “profile write failed: Success” or stale unrelated cause;
failed locking still emits no profile bytes; cleanup cannot double-close or
change exit status; partial-write behavior is explicitly documented.

## R6 — P2: evidence needs provenance and a final account

**Locations:** `engineering/FOLLOWUP-2026-10-14C-relocation-audit.md`;
`engineering/evidence/2026-10-14C-relocation-audit/`.

**Classification:** delivery-quality gap. Historical evidence is not alleged to
be fabricated; the available files simply cannot establish reproducibility.

The 50 evidence files dominate this patch while most concern unrelated x86
codegen surveys, not either changed production subsystem. The follow-up note
still says results will replace the in-progress note. `tune.txt` records a failed
CLI invocation. Relocation summaries lack full commands, tool identities,
fixture hashes, stdout/stderr, and the implementation identity for the successful
run. Oracle manifests name a local baseline executable and a moving ICX alias;
remote/local ICC records include failed work. Such records can be useful research
history but are not proof that this PR's source passed. Existing performance
caveats are good and must remain.

**Instructions:** replace the in-progress narrative with final scope/status and
explicit failed/unsupported work. Add a compact provenance manifest: tool
versions/IDs, source tree identity, executable hash, command, fixture/source
hash, exit status, stdout/stderr or content-addressed artifact links, and result
classification. Preserve baseline and after measurements as distinct states.
Separate unrelated codegen research into a clearly labeled archive or follow-up
PR according to repository policy; don't relabel baseline measurements as
post-change ones. Keep failure records, but index them as failures. Do not claim
that instruction counts or loop-density heuristics establish runtime speed.

**Acceptance:** another agent can identify exactly which input/tool/tree produced
each contract result; no successful claim depends solely on a path or moving
alias; final notes match delivered scope. The review's description-only update
improves transparency but does not itself repair artifact provenance.

## R7 — P3: harness reporting and scheduling can be more fail-safe

**Locations:** PGO test `counters()` and `main()`; relocation test `run()`;
PGO interposer `marker()` and lock/close tracking.

**Classification:** robustness/precision follow-up; do not misdescribe this as
proof that the current two-writer test is useless. The explicit-unlock mutation
is genuinely forced to fail by the existing handshake and unlocked-close marker.

PGO parsing accesses `fields[0]` before checking for blank records and does not
reset the current function when it sees a new header. PGO `--json` is written
only after all checks succeed, so failure output has no structured summary.
Both new Python tests use bare assertions and should guard against an optimized
Python invocation disabling correctness checks. The crash matcher does not
include LCCC's generic `ccc: internal error` form. On the corrected runtime,
A can finish before B is launched; the original-bug mutation is deterministic,
but a claim of forced overlap/contending writers is stronger than the handshake
establishes.

**Instructions:** use explicit check helpers or reject disabled assertions; make
record parsing state explicit and malformed records structured failures; write
JSON in success and failure paths. Centralize crash/error classification without
accepting a tool crash as a normal rejection. Add `B-attempting-lock` and
release/close-phase barriers if claiming guaranteed contention, bounding every
wait and preserving process cleanup. Add compiler-free harness self-tests
through injected subprocess responses and synthetic profile/ELF bytes.

**Acceptance:** blank/malformed records, missing tools, optimized Python,
internal errors, timeouts, and abnormal exits fail loudly with structured
context. Any guaranteed-overlap claim has a barrier trace proving it, not a
scheduling assumption.

## R8 — P3: encode metadata and generated-IR invariants once

**Locations:** `reloc.rs:58–72` vs `apply_one_reloc()`;
`src/pgo/instrument.rs:922–936, 1100–1197, 1563–1582`.

**Classification:** maintainability risk, not a proven SSA collision in this PR.

The independent extent and dispatcher matches can drift as kinds are added.
`None` represents an unsupported kind but becomes zero width during structural
validation; that distinction is intentional today but implicit. Hard-coded
PGO block IDs and hand-assembled error/acquisition tuples are readable at this
size but lack a mechanically checked relationship to `next_label`, SSA IDs,
function signatures, or successors. I inspected these new blocks and did not
find a duplicate definition or missing successor.

**Instructions:** represent fixed-width, variable-width, hint/no-write, and
unsupported categories explicitly. Use one metadata source or a unit invariant
that reconciles implemented dispatcher kinds and extent kinds. Reuse the shared
relocation bounds utility where semantics match. Add generated-helper IR checks
for unique block/value definitions, existing branch destinations, dominance of
`file`/`fd`, signature agreement, and `next_label` exceeding all labels. Avoid a
large cosmetic rewrite in the safety fix; a small local allocator/helper is
sufficient if it improves those invariants.

**Acceptance:** introducing a dispatcher arm without field metadata fails a
check; adding a generated block cannot silently collide with future allocations;
the validated error paths remain obvious.

## Adjacent inherited debt — explicitly not new regressions

### R9 — P2: archive parse errors can be swallowed before validation

`src/backend/riscv/linker/elf_read.rs:38–44, 71–75` silently omits members whose
`parse_object` fails. Several archive-loading call sites in `input.rs` also
consume errors with `if let Ok`. Thus the new validator protects **successfully
loaded objects**, not every malformed archive member. A malformed required
member may yield a generic missing-symbol failure rather than its parse error;
an ignored member need never reach the validator. Don't claim complete archive
hardening from these new entry-point checks.

**Separate-task instructions:** define lazy archive selection versus malformed
member policy, retain parse errors with archive/member identity, propagate errors
for required members, and test regular/thin archives plus linker-script inputs.
Do not accidentally force rejection of irrelevant members without documenting
that policy. Until fixed, state the narrower successfully-loaded-object contract.

### R10 — P2: storage bounds are not relocation value-range validation

Existing instruction patchers and `apply_one_reloc()` mask/cast branch, jump,
and narrow data values without comprehensive psABI range/alignment checks.
`TLS_GD_HI20` also retains an explicit shared-library dynamic-TLS TODO. These
problems predate this delta; checked storage offsets do not solve them.

**Separate-task instructions:** apply range/alignment rules per psABI using the
existing shared `RelocField` infrastructure where appropriate; retain legal
modular data operations instead of imposing indiscriminate checked arithmetic.
Add near-limit positive and just-out-of-range negative controls and document or
reject unsupported dynamic TLS paths. Don't claim support for the entire psABI
or for numerically correct relocation of every implemented kind in this PR.

## Execution plan for the implementing agent

1. **Safety commit:** R1 implementation and its focused regression matrix. No
   unrelated optimizer work. Tests must detect the old writer behavior.
2. **Contract commit:** R2 semantics, R3 valid boundaries, required oracle and
   archive/separate-object fixtures. Keep local/hosted invocations in parity.
3. **PGO commit:** R4 independent absolute expectations and R5 diagnostic/cleanup
   semantics. Keep the good close-under-lock approach; do not reintroduce unlock.
4. **Harness/invariant commit:** R7 compiler-free self-tests and R8 metadata/IR
   invariants. Document supported platforms for LD_PRELOAD, flock, and /dev/full.
5. **Evidence/docs commit:** R6 provenance and final report; separate unrelated
   surveys transparently. Track R9/R10 as bounded adjacent debt.
6. Delegate compilation/execution to GitHub CI under this session's no-local-
   compile constraint. Require both named new gates and the normal required
   checks to pass on the final branch head. Save transcripts as provenance, not
   just PASS counts. A pending check is not a passing check.

**Merge threshold:** R1 closed with adversarial and valid ULEB cases, no unresolved
new correctness failure, accurate limited claims, and required CI green.
**10/10 review target:** all eight scoped findings closed with their acceptance
criteria and inherited limitations explicitly tracked. This is a target for the
stated scope, not a claim that ten items make the whole compiler perfect.

## Validation and CI snapshot

- Original strict patch application: 68 hunks at exact original line positions
  (zero fuzz / zero offsets), all 59 resulting blobs matching expected hashes.
- Changed Python AST and JSON parsing passed; shell syntax validation passed.
- Supplied before/after relocation JSON arithmetic and record counts checked.
- `check_ci_gate_parity.py`: PASS, 217 commands / 22 registered invocation
  contracts. `check_ci_workflow_shell.py`: PASS, two workflow files. PyYAML was
  installed as a binary wheel outside the repository for these static checks.
- Standard original-delta whitespace check reports 19 assembly EOF blank lines;
  these were retained verbatim from the source patch.
- At the implementation-head snapshot: Clippy, MSRV, and Compiler Explorer oracle
  delta passed; Test Suite and LCCC vs GCC were pending. The two new contract
  steps were still pending when inspected. This is not a complete green CI claim.
  A documentation push can trigger a new run; inspect the final head before merge.
- **No local builds or compile tests were run.**
