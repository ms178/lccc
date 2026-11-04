# Kernel diagnostic successor: evidence and unfinished gates

Base: main dfb902b (merged PR1065). This is an incremental audit, **not** a
production-readiness or all-13-family completion claim. Prior sandbox processes,
kernel trees and tool installations did not survive the restore. No old process
or old-head kernel receipt is reported as a current-head validation.

## Confirmed defect fixed: typed, scoped condition truth

`try_eval_constant_bool` discarded casts under the false premise that conversions
preserve truth. `(unsigned char)256` is zero, not true. This hid return-type
warnings for if/while/for/do and produced the opposite error under negation.
Local enum conditions were queried after scope teardown, losing the enumerator.

Choices considered:

1. Extend the syntactic matcher: cheap, but duplicates target-width conversion,
   overflow and lexical-binding rules; still wrong after scope teardown.
2. Re-run the typed evaluator at the final return check: correct conversions,
   but the required bindings and local typedefs have already gone away.
3. Snapshot the existing typed ICE predicate when checking the scalar condition:
   chosen. One evaluation per checked condition, then a hash lookup at later
   flow queries. Unknown is stored explicitly, so a shadowing object cannot
   later become an outer enumerator. Facts survive nested-function analysis;
   file-scope function completion clears entries while retaining map capacity.
   Storage is bounded by the largest function tree, not accumulated across TUs.

This reuses `constant_truth_value`, including its overflow exclusions and literal
fast path. It does not enable new optimizations, change ABI metadata, or claim a
measured runtime improvement. No unsafe code, shared mutable global, locks or
new integer arithmetic were added. Keys follow the existing stable-AST ExprId
contract. A general CFG rewrite would be a larger, independent change.

## Executed evidence

Reviewed cache restored Rust1.99.1, GCC16.2.0, Clang23.1.4, binutils2.47 and mold3.
Active swap: 4GiB. Both compiler/linker builds use the normal fastbuild profile
and `-D warnings`, never an opt-level-zero substitute.

- Baseline: 260 oracle/candidate checks, **56 failures**, all in LCCC; the GCC
  and Clang reference rows pass. Raw before report is retained alongside after.
- Fixed and extended matrix: **350 checks, zero failures**. Four LCCC targets
  (x86-64/i686/AArch64/RV64), O0/O2; GCC16.2 native/i686, Clang23.1.4 all four.
  Positive and negative cases cover narrowing, nested conversions, negation,
  bool/wide conversions, scoped enums, shadowed enums and local typedefs.
  GNU nested definitions use GCC/LCCC only because Clang rejects that extension.
- Eighteen of the checks compile and execute native statement-expression
  fixtures, checking compiler status, executable status and exact empty output.
  This guards retained reachable values, not just warning counts.
- The old compiler's 56 failing checks are the mutation witness for the replaced
  truth implementation. The extra cases were added after that baseline run;
  do not claim the old compiler was tested on all 350 final checks.
- Existing scoped-noreturn gate: **31 passed**, including three emitted-control-
  flow checks and twelve GCC differentials.
- Full library units: **7862 passed, zero failed, 10 ignored** (fastbuild,
  warnings denied). Formatting and the unchanged audit ratchet pass.
- All-target Clippy with `-D warnings`: passed (fastbuild profile).
- Local ICX2026.2 authenticated provisioning was attempted; Intel HTTPS closed
  TLS under the sandbox allowlist. No older compiler was substituted. The
  existing authenticated ICX CI lane now runs the new matrix with all three
  exact oracles and retains its JSON report. That hosted result is still owed.

Local final compiler SHA256:
`a535a0d2afa0ea39b228bd92b333d7d7e1a8397daa4148b1bb113fb0253a6c50`.

Reproduce:

```sh
source /home/user/.cache/lccc/toolchains/env.sh
bash scripts/build_lccc_fast.sh
python3 tests/regression/check_constant_truth_diagnostics.py --json truth.json
CC=gcc-16 CCC=target/fastbuild/lccc bash tests/regression/check_scoped_noreturn_diagnostics.sh
```

## Remaining implementation and acceptance sequence

No unsupported option is relabeled as implemented by this change.

1. **Control flow:** audit loops with reachable break/continue before using the
   return-path machinery for implicit-fallthrough. Statement transfer targets
   are now modeled by the continuation graph described below; remaining
   expression-level transfer coverage must still be audited.
   Preserve explicit fallthrough attributes rather than discard them; implement
   numeric warning levels and source/pragma scope semantics with oracle cases.
2. **Format metadata:** preserve format contracts through declarations, typedefs,
   function pointers and lexical shadowing. Then implement missing-format-
   attribute plus overflow/truncation bounds. Do not dispatch solely by callee
   basename or claim unknown bounds are a proof of safety.
3. **Pointer/string diagnostics:** retain parameter-object restrict qualifiers
   separately from pointee qualifiers in callable metadata. Implement restrict
   overlap and stringop-truncation checks with zero-length, offset, overlapping,
   unknown-size and NUL-termination controls. Re-test existing warning families,
   not only the six currently missing families.
4. **Actual option semantics:** handle remaining stack, scheduling/IV, padding,
   flex-array and data-race options with observable backend/IR tests. Reject
   unsupported positive modes; accepting flags without effects is not a fix.
5. **Performance:** use correctness-gated paired canonical workload samples;
   enforce the gzip stack-memory veto after codegen changes. Compare the exact
   GCC16.2/Clang23.1.4/ICX2026.2 toolchains. No universal efficiency claim.
6. **Final binary validation:** refresh main, then canonical patched Linux6.18.58
   boot-size gate before full default-config link; real objtool and QEMU smoke.
   Only LCCC compiles kernel target code. Preserve diagnostics and require zero
   real production warnings; no validators or security gates disabled.
7. Re-run hosted CI on the actual final head, replay-verify and snapshot
   ms178-1.patch. Do not mark the successor PR ready until required gates pass.

This list includes confirmed pre-existing defects, not promises of universal
correctness or imaginary expert approvals. Full final-head kernel/QEMU evidence,
all remaining families/options and final green CI are **not established**.

## Confirmed validation gap fixed: kernel workflow selection

The inherited kernel workflow selected changes to seven harness files, but no
compiler-source or Cargo/toolchain changes. A source-only compiler PR therefore
could be green without rebuilding/booting a kernel. This is a coverage defect,
not evidence that a kernel had passed for such a PR.

Add compiler/linker sources, build inputs, toolchain locks and direct boot-harness
dependencies to the positive path filters. Keep documentation-only changes out:
we do not pay for a full kernel build just to edit this audit note. The filter
contract test file itself is included in the trigger. The existing workflow's real
link, objtool and QEMU steps are unchanged; no condition is weakened.

Regression witness: the new contract suite fails **21 subcases** against the
inherited filter; the expanded **six tests pass** after the change, including compiler
changes on all four backends, linker/build-input coverage and negative doc cases.
The existing CI lane-plan check passes. Full pinned-toolchain Python corpus
revalidation is pending at this note's first write.

Pinned-toolchain Python corpus run completed: **1150 tests, zero failures, five
skips**. The no-negative-glob contract was added after that process loaded its
suite; the expanded six-test filter suite was run separately and passed. CI
will discover the full expanded suite from the committed files. No combined
count is fabricated by summing overlapping runs.

## Successor checkpoint: continuation graph and main integration

Integrated main `9bed91bec6c044d8a7fadfc7e062fc62b4e57a31` (including its
count-builtin/backend fixes). Replaced the independent loop/switch return
heuristics with `analysis/flow.rs`. The previous implementation treated every
break/continue/goto as divergence and ignored the destination. For example,
`while (1) { break; }` reaches the function end, whereas an inner loop's break
must not escape the outer loop. A goto can re-enter code after an otherwise
terminating statement.

Decision: construct statement continuations once, with target-identity stacks,
then resolve labels and run a visited-node worklist. Switch cases dispatch to
actual nested case entries; statements before the first case are not the
switch entry. Named targets survive synthetic C2y initializer scopes. Asm goto
retains both normal fallthrough and listed label edges. Direct computed label
addresses retain their exact destination; unknown computed destinations use a
conservative shared dispatch hub. That hub requires G+L edges for G unknown
computed gotos and L labels, rather than cloning G*L label edges. Worklist
traversal is O(nodes+edges); named-target lookup still depends on nesting depth.
This is diagnostic analysis, not an optimizer proof or measured runtime gain.

Direct statement expressions share the graph, including declaration/for
initializers, conditions, increments and return expressions. Cast/comma,
conditional and short-circuit transfers are represented. Logical left-operand
truth is captured before lexical scope disappears, just like scalar conditions.
**Remaining expression scope:** this is not a complete expression CFG. Transfers
buried in other operands (e.g. call arguments, assignments), VLA/type-name
operations and selector-value-sensitive switch dispatch still need audit and
implementation. Unknown computed destinations may overapproximate reachable
labels. These are tracked limitations, not a claim of fully repaired flow.

Evidence (non-kernel fixtures, exact GCC16.2.0/Clang23.1.4):

- Frozen pre-fix compiler SHA256
  `a535a0d2afa0ea39b228bd92b333d7d7e1a8397daa4148b1bb113fb0253a6c50`:
  **1346 checks, 216 LCCC failures**, no reference failures.
- Repaired compiler SHA256
  `ed3cd070addead6f42f0f5c1c8f921233811d482a54321dc04107c2e0a480e69`:
  **1346 checks, zero failures**, four targets at O0/O2. Includes **54 native
  execution checks**, validating build status, exit status and empty output.
- Actual source mutation `Stmt::Break(..) => DIVERGES`, built with the same
  fastbuild script: **64 LCCC failures**; no reference failures. Correct source
  restored and rebuilt, then the entire matrix passed again. Raw before,
  repaired and mutant rows are committed beside the earlier truth evidence.
- **7866 library tests passed, 10 ignored**; all-target Clippy `-D warnings`,
  formatting, diff checks and the unchanged audit ratchet passed. Added unit
  coverage for named transfers and expression/initializer control transfers.
- GCC emits an additional `-Wswitch-unreachable` in four rows. The test requires
  that exact diagnostic as well as the return warning; no warning is disabled,
  and arbitrary extra warnings still fail.
- Six separate C2y native probes: LCCC agrees with GCC on every return count.
  Clang misses three named-loop exit warnings, crashes (signal 11) generating a
  named-switch break, and rejects the switch-initializer probe. Raw observations
  are in `named-flow.json`. These vendor defects are **not** used to weaken
  LCCC expectations; permanent LCCC unit checks follow the GCC/language result.
  These six probes are not claimed as an all-target three-oracle pass.

The previous hosted fast-shard failure was CI/local gate parity: the hosted
constant-truth matrix had no local mirror. Added `typed-constant-truth` to the
fast local gates, without allowlisting the omission. Parity now passes **529
commands / 27 contracts**; lane plan passes at **2928 estimated seconds**.
The old-head authenticated ICX2026.2 lane passed the earlier 350-check matrix;
new-head hosted ICX and full CI are still owed. The previous kernel job was
cancelled during full bzImage compilation after boot-first gates passed; QEMU
was skipped. The API did not identify who or what cancelled it.

All-13 completion, final-head local full kernel/real objtool/QEMU, zero kernel
warnings, measured host-runtime/code-size gains and final green hosted CI remain
**open acceptance gates**. This checkpoint must remain a draft.

## Hosted kernel / QEMU validation update — 2026-11-04

At PR head `8e881bb9e39c9e6fa62c6e58feb66b6f3dc3794e`, GitHub Actions
run **41735943201** (kernel workflow run #45) completed successfully. The
hosted job enabled the 4 GiB swap, built the fastbuild compiler and linker,
prepared the pinned patched Linux 6.18.58 tree with LCCC, selected x86-64
`defconfig`, passed the boot-first / 32 KiB gate, linked the complete
`defconfig` `bzImage`, and completed the QEMU smoke test
(`DEFCFG QEMU SMOKE PASS`). The workflow uploaded the kernel log, QEMU
artifacts, setup ELF, `.config`, and preparation/build provenance.

**Scope of this result:** full hosted kernel link and QEMU boot are now
established for this exact PR head. This does **not** establish zero production
diagnostics or completion of the remaining warning families/options. The
compiler log still reports that `-Wimplicit-fallthrough=5` is tolerated but
not implemented; this remains an option-semantics gap. The workflow's separate
diagnostic-surfacing step was skipped, so no zero-warning claim is inferred
from the green job. Local swap remains unavailable in the restricted sandbox;
this was hosted-runner evidence, not a local reproduction.

## Restored-state audit and scoped choose_expr repair — 2026-11-04

Recovery found HEAD at the old base, with recovered source files. All remote
tracked files were content-compared before reconciliation. Three older recovered
files were separately backed up; the remote PR's newer fbb4043f4 source was then
restored without discarding unexamined changes. Main 5a3b2087c was merged without
conflicts after deepening the shallow history. The compiler/linker toolchains
were fetched from the reviewed harness-cache-7 lock; GCC16.2.0, Clang23.1.4,
Rust1.99.1, fastbuild and an active **4 GiB** swap were used.

Confirmed new defect: return-flow `__builtin_choose_expr` re-evaluated its ICE
condition after a declaring inner block disappeared. Both true and false scoped
enumerators caused spurious return warnings. Six new fixtures also check local
and shadowed `_Generic` typedef selections; those four controls already passed
and are not claimed as defects fixed. Before: **174 checks, 16 LCCC failures**,
no GCC/Clang failures in that scoped-selection subset.

Decision: reuse the existing scope-live `condition_truth` map rather than add
another cache, retain entire scope environments or duplicate the ICE evaluator.
Capture the typed truth (including unknown) after argument analysis, and consult
it at the graph query. Clear it at the existing file-scope function boundary;
nested-function analysis keeps the parent's facts. Four new library assertions
cover true/false selections and normal/diverging selected arms. No optimization,
ABI, target instruction selection or hardware-speedup claim is made.

Full validation: **7897 library tests passed, 10 ignored**; all-target Clippy
`-D warnings`, fmt/diff checks, 31 scoped-noreturn checks, i686 select-cmov
runtime/reference/folds-disabled checks at O0/O1/O2/Os passed. Parity passes
530 commands/27 contracts and the unchanged lane plan passes at 2929 estimated
seconds. The expanded matrix records **1678 passing expected observations**.
This is NOT 1678 semantic agreements: it explicitly reports **18 reference
semantic disagreements**, all in Clang23.1.4, never a waiver for LCCC.

Reference red-team findings: GCC and LCCC warn for returns buried in unevaluated
constant_p/classify_type operands; Clang misses those warnings (16 rows). New
native controls also show Clang executing the nested `return 99` inside
`__builtin_constant_p` at O0/O2, while GCC and LCCC retain the enclosing value 7
(two rows). `classify_type`'s native control passes. The test records normative
and pinned reference-observation outcomes separately and will flag observation
drift. These vendor defects are NOT fixed by this PR, not hidden by warning
flags, and not evidence of universally correct references. Raw uncalibrated
observations are retained alongside the before/after reports.

Authenticated ICX2026.2 provisioning was attempted locally again but Intel TLS
closed under the sandbox allowlist. Old-head hosted ICX is red in this matrix;
its signed log storage is inaccessible locally. Workflow dispatch returned 403;
the existing diagnostic-export PR label was added for the next push so the
hosted transport can expose the actual failure through GitHub blobs. No ICX
expectation was guessed from Clang, and no three-oracle pass is claimed.

A four-file kernel-priority source audit is retained separately under
`engineering/evidence/kernel-priority-2026-11-04`. It identifies AMDGPU ring
fills, ixgbe descriptor cleanup/wrap, VM address arithmetic and bitmap reductions
with mandatory synchronization/feature guards. It is a partial lexical census,
not workload profiling or a measured optimization. The host is a Xeon, not the
user's 14700KF; no NIC/GPU/board performance measurement is available.

Remaining warning families/options, vendor-reference discrepancies, local final-
head full kernel/QEMU, measured performance gains and green final-head CI remain
OPEN. This is a validated recovery/fix checkpoint, not production readiness.


The diagnostic transport subsequently completed successfully at the new head.
Its SHA256-verified blob exposed the original authenticated ICX2026.2 failure:
**four missing return warnings** for unevaluated constant_p/classify_type operands
(two cases at O0/O2), matching the observed Clang diagnostic discrepancy. Those
four reference expectations are now pinned from actual ICX evidence, not an
assumption. LCCC/GCC normative expectations remain unchanged. A compact log
excerpt and transport identity are committed. ICX behavior on the newly added
constant_p native return-99 witness is still unverified and is NOT assigned a
speculative expected result. New-head full CI remains pending.

## CI evidence retrieval repair — 2026-11-04

Current-head ICX still fails the value matrix. Exact exit-code evidence is
required before changing a runtime reference contract. The diagnostic exporter
now includes the SHA-named Intel provenance artifact (matrix JSON), not merely
its log, and can select a completed cancelled workflow containing an already-
failed job. It still refuses in-progress/foreign-repository runs, cross-repo PRs,
expired artifacts and unrelated artifact names. It creates content-addressed
blobs only, never updates refs/check outcomes or runs kernel/compiler builds.

A separate label-only `CI diagnostic evidence` workflow avoids coupling a log
request to host-package downloads or a restart of the kernel job. It neither
skips nor replaces the kernel's validation check. Existing kernel build events
and security gates are unchanged. The request label is `ci-diagnostic-transport`.
The tests also exposed an unclosed event JSON file; it now uses a context manager.

Validation: seven new transport tests pass with Python warnings treated as
errors; separate failure-only-selection and omitted-Intel-artifact mutants both
fail the intended test. CI/local gate parity passes unchanged at 530 commands /
27 contracts. This is an observability/infrastructure repair, NOT resolution of
the outstanding ICX value discrepancy or a green-CI/production-readiness claim.

The first dedicated export also exposed an API-order assumption: the completed-
run response placed an older cancelled retry before a newer failed run sharing
its creation second. Selection now explicitly sorts by creation time and numeric
run ID before applying the per-workflow cap. Two new tests cover that observed
ordering and timestamp precedence. All nine transport tests pass with warnings
as errors. The first transport-only run completed successfully without starting
or replacing any kernel job; exact ICX artifact retrieval continues.

## Exact ICX runtime cause and non-waivable candidate contract

Dedicated export run 41814140259 recovered Intel artifact 13313650379 from
failed CI run 41812128042, job 126515336537. Its archive SHA256 is
`270be67b3d04981fa0a0aaee38126ace424b50b3c06c458503c7a69554bc2326`.
The verified `constant-truth.json` has 1860 records and exactly **two** failed
records: ICX's native constant_p predicate, build status 0 / executable status 1
at O0 and O2. All other observations in that artifact match their contracts.
A compact receipt retains the failed records and content-addressed transport
identity. The predicate only proves the subject was not 7; no exact subject
return value or unrecorded output is inferred for ICX.

The reference runtime profile now pins that measured predicate status. LCCC and
GCC still must exit zero with empty output; unknown statuses, signals, output,
compilation failures or unlisted reference discrepancies fail. Reference profile
drift also fails. A hard guard prevents reference-profile entries from overriding
LCCC, even if such an entry were accidentally inserted. Ten new contract tests
cover those cases, exact extra warnings and disabled/duplicate-warning controls.
Two mutants (leaking a reference override into LCCC, and labeling a matched
reference failure as semantic success) are killed by the tests.

JSON now records `semantic_passed` independently of matching the pinned reference
observation, plus native build stderr and executable stdout/stderr. The console
prints LCCC semantic failures and actual reference semantic failures separately.
Local matrix: **1678 observations, zero unexpected; LCCC semantic failures=0,
reference semantic failures=18**. A hosted three-oracle run is expected to record
24 reference semantic failures (the prior 18 plus six measured ICX discrepancies),
NOT zero compiler issues or universal semantic agreement. The new hosted result
must still be observed; replaying the receipt is not a fresh ICX execution.

No Rust/backend instructions or kernel warning options were changed by these CI
contract/infrastructure fixes. The existing 7897-test Rust/Clippy validation
applies to the unchanged compiler source; the transport's nine tests and matrix
contract's ten tests pass with Python warnings treated as errors. The API-order
mutation is killed too. Main 5a3b2087c remains an ancestor. No production-readiness,
measured performance or all-warning-family completion claim follows from this.

## Confirmed shadowed-record selection defect fixed

Further audit found a genuine LCCC defect beyond the CI reference discrepancies:
`_Generic` association resolution was repeated after an inner struct/union tag
scope disappeared. The controlling expression retained its inner CType while
the association name resolved to the outer tag. Three selected-arm cases warned
spuriously; the reverse case missed a required warning. Pointer-to-record
associations were affected too. GCC16.2 and Clang23.1.4 agree on all four cases.

Before: expanded **1770 observations / 32 LCCC semantic failures** over four
LCCC targets at O0/O2. After: **1770 / zero LCCC failures**, zero unexpected
observations; the existing 18 reference semantic failures remain explicitly
reported. Two native shadowed-tag controls retain the enclosing value 7; these
already passed before, so no native miscompile fix is claimed.

Choice: cache the type checker's selected index while its lexical scope is live,
not a second tag-name resolver or a retained scope environment. Explicit None
is kept distinct from a cache miss. The index lookup reuses the current AST
ExprId invariant; facts survive nested-function analysis and are cleared with
capacity retained at the file-scope function boundary. Storage is bounded by the
largest function tree's selections, not accumulated over the translation unit.
This is a diagnostic correctness fix, not an instruction-selection speedup.

Seven new library assertions cover struct, union, pointer-to-record, inverse
selection, local-only controls and a nested-function arm. **7898 library tests
passed / 10 ignored**, all-target Clippy -D warnings, fastbuild, formatting,
31 scoped-noreturn checks and parity passed. The first census invocation only
recorded the inventory: a subsequent strict `--check` found that `analyze_expr`
had grown to 502 lines, crossing the existing >500-line limit. Extracting its
Generic-selection arm into a focused helper restored the count to **97/97**
without changing the baseline or relaxing the validator. The expanded matrix
includes 78 native compile/execute checks. Raw reports remain at
`/home/user/generic-scope-{before,after}.json`; the compact checked-in receipt
records their SHA256s, counts and all observations for the four new cases.
Baseline compiler identities are retained alongside it.

## Confirmed callable-object / standard-name binding defect

Three independent non-kernel probes reproduced missed `-Wreturn-type` warnings:
function-pointer parameters named `abort` and `exit`, and a block-local pointer
named `abort`. GCC16.2 and Clang23.1.4 both warn. A native callback probe returned
its correct result on all three compilers before the fix: again, this is not
claimed as a demonstrated native miscompile.

Four permanent diagnostic cases cover abort, exit, quick_exit and a pointer
hiding an explicitly attributed user function. Baseline with the Generic fix:
**1922 observations / 32 LCCC semantic failures** over four targets at O0/O2;
the same **18 reference semantic failures** remain reported. Three positive and
negative controls require real standard/attributed noreturn calls to terminate
flow, and an ordinary call to retain the warning. Two added native controls
call the returning libc abs function through locally shadowing pointer names
and retain value 7. Both already passed the baseline.

Choice: retain only the **object-binding kind** in a private ExprId set while
scope is live, rather than allocate a frozen boolean/string record for every
call or retain whole lexical environments. Captured object and nested-function
bindings outrank bare-name library knowledge. The set shares the existing
function-tree lifetime; nested walks do not clear their parent's facts. No
reference override or diagnostic expectation was relaxed. Twelve library
assertions exercise parameter/block binding, standard and user declarations,
returning and noreturn nested functions, and parent-fact survival. In particular,
a captured nested function must outrank an outer pointer parameter after the
nested function's declaring block has popped. Strict final-source validation
and mutation-control results are recorded after completion, not inferred from
the earlier successful source build.

The original red CI condition is now actually green on head `6bc405e61`:
CI run **41814938313 completed/success**, including the authenticated ICX2026.2
job **126525143400 completed/success**. This is evidence for that exact head,
not for these later source changes; they require another final-head run.
Latest fetched main remains `5a3b2087c`, already an ancestor of this branch.

### Final local validation for both scope fixes

Both deliberately broken variants were killed by assertion failures in their
specific library tests (not compile errors): disabling captured Generic indices,
and disabling captured object-binding lookup. Sources were restored in `finally`
blocks before rebuilding every compiler binary and rerunning validation.

Final source: **7899 Rust library tests passed / 10 ignored**, Clippy all-targets
with `-D warnings` clean, **1922 matrix observations / zero unexpected and zero
LCCC semantic failures**; 18 reference semantic failures remain explicit. All
90 native compile/execute checks passed their measured reference profiles, not
all language semantics (the two Clang constant_p executions remain failures of
the latter). The 31 scoped-noreturn checks, including three emitted-flow and
12 GCC differential checks, passed. Strict audit `--check`: >500-line count
**97/97**, all metrics held; no baseline update. Parity: **530 commands / 27
contracts**. Formatting and diff checks passed. Strict Python-Werror corpus:
**1170 tests / 120.675 s / PASS, 5 skipped**. Compact differential receipts,
compiler identities and mutation-log hashes accompany these results; full raw
reports/logs remain outside Git in `/home/user`.

Neither fix substantiates measured throughput/code-size gains, hardware-specific
readiness, all remaining warning/option families, or the final-head local full
kernel/real-objtool/QEMU gate. Kernel host prerequisite recovery has verified the
original Git-blob, part, archive and all 93 Debian package digests; those are host
tools only, never GCC-compiled kernel objects. Final-head hosted checks are still
required after pushing these fixes.

## Actual local package preparation and stronger configuration refusal

Restored the complete Linux tree using the existing preparation script and
verified tag v6.18.58 / commit `2926d88b`, archpkgbuilds `df9c9172`, recipe
`b3fe4c7c4a8f82c7692d4077e99876693fcbcb82cef493018b43e68282d1bcaa`.
Tag identity was verified, not its GPG signature. Target preparation used LCCC;
GCC16.2 was HOSTCC only. All 93 recovered Debian host-package SHA256s passed.
The package-derived configuration still produces the previously disclosed
unsupported-option warnings: this is **not zero-warning kernel evidence**.

At compiler head `747fc3c2a`, local standalone boot gate: **31088 bytes / limit
32768 / headroom 1680**, `ld.bfd` flat-image byte identity PASS, **24 objects /
zero forbidden ISA instructions**. No controlled comparison with older setup
headers is available; do not interpret the difference from historical 31024 as
a measured code-size regression or gain.

An actual `LCCC_RECONFIG=1` full-package attempt correctly refused a dropped
sched-ext feature. Audit then found that the drift list omitted LTO, BTF and
other important compiler/config-dependent features. A new adversarial harness
case deliberately dropped only CONFIG_LTO: the old harness incorrectly
**succeeded with exit 0**. This is a confirmed validator gap, not a theory.

The list now preserves the enabled LTO modes, BTF and module BTF, Clang CFI,
and x86 kernel IBT. Nine negative tests use the **real** drift checker (not
its older injected TEST_DRIFT stub), with nine unchanged-feature positive
controls. Individually removing each new guard entry kills its corresponding
test; **all nine mutants were killed**, without changing production sources.
The harness remains a mocked Kbuild contract test, not kernel-build evidence.

The strengthened real package attempt refuses **CONFIG_LTO, CONFIG_LTO_CLANG,
CONFIG_LTO_CLANG_FULL, CONFIG_DEBUG_INFO_BTF, CONFIG_DEBUG_INFO_BTF_MODULES and
CONFIG_SCHED_CLASS_EXT** being absent. `LCCC_ALLOW_CONFIG_DRIFT` was never set.
No substitute defconfig, removed validator or disabled security feature is
counted as satisfying the original package configuration. The absence of these
features is measured; their individual implementation/provisioning root causes
have not all been investigated. The full package kernel, local QEMU, all
remaining options/warning families and hardware readiness remain unfulfilled.
A separately labelled defconfig integration build can find further compiler
failures, but cannot waive this package-config blocker.
