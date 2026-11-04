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
