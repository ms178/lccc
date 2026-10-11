# PR #789 — critical audit and implementation acceptance brief

**Recommendation: do not merge the current implementation.** Preserve the useful
compiler fixes, but close the five P1 findings below and obtain green CI on the
resulting code before merging. Green tests do not waive demonstrated holes in
what those tests measure.

This audit reviews the original 51-file compiler/tooling change in
[PR #789](https://github.com/ms178/lccc/pull/789). The subsequent documentation
commit adds this report and repairs a malformed README table row; it does **not**
implement the compiler or tooling fixes requested below.

## 1. Scope, evidence, and grading

### Quantified scope

| Area | Files | Added lines | Removed lines |
|---|---:|---:|---:|
| Compiler source | 15 | 1,076 | 148 |
| Harness / CI / oracle tooling | 9 | 458 | 80 |
| Regression tests / workloads | 16 | 1,083 | 2 |
| Documentation / provenance / licensing | 11 | 874 | 2 |
| **Original change total** | **51** | **3,491** | **232** |

The change adds **16 Rust unit tests**, **20 Python tests**, four C regression
fixtures, and two mixed-compiler ABI gates. The aggregate gate specifies
`2 architectures × 5 optimization levels × 13 sizes × 3 compiler pairings = 390`
executions; the empty-aggregate gate specifies another `2 × 5 × 2 = 20`.
These are **410 executable configurations**, not 410 independent bug classes.
The SipHash kernel retains **64 upstream known-answer vectors** and adds
**8,385 streaming-split comparisons** over lengths 0 through 128.

### What was actually verified in this harness

- The original import was fetched through the GitHub Contents API and applied
  with strict indexed Git checks and an explicit GNU `patch --fuzz=0` dry run:
  zero fuzz, zero offsets, all 51 preimage/postimage blob checks matching.
- A complete whitespace check passed. All nine changed Python files parsed via
  `ast.parse`; all seven changed shell scripts passed `bash -n`.
- **64 inspected, compiler-free contracts passed:** 5 GCC corpus provisioning,
  15 native-torture/evidence, 12 process/publication, and 32 mocked Callgrind
  contracts. The linker-version shell contract also passed using stub tools.
- **11 adversarial witnesses** were reproduced: one pass-plus-reference-SIGKILL
  matrix; five malformed/unsupported report cases; two unequal Expat test-set
  cases; one included-header identity case; one worker-exception case; and one
  dispatcher raw-string boundary case. They expose defects or limitations;
  reproducing them is **not** a passing correctness gate.
- Expat's upstream `siphash.h` was retrieved through the GitHub API. Its SHA-256
  matches the recorded provenance. From `SIP_ULL` through `sip24_valid`, the
  extracted macros, algorithm, and vectors are identical after removal of
  comments and whitespace. The upstream header explicitly declares CC0-1.0.
- Binutils 2.47's upstream `gas/read.c`, `tc-riscv.c`, and `tc-riscv.h` were
  inspected through the GitHub API to check the alignment claims.

**No C/Rust builds, compile tests, assembler runs, or compiled-runtime tests
were performed locally.** Python witnesses use the actual runner/consumer
entry points or the actual final Expat validation block, with compiler/XML tool
operations mocked. Static compiler findings are explicitly distinguished from
executed Python findings; they are not claimed as locally observed miscompiles.

### Live CI evidence at audit time

On the audited compiler change:

- **Clippy: success.**
- **Compiler Explorer oracle delta: success.**
- **LCCC vs GCC: success.**
- **Test Suite: still in progress** at the last pre-publication observation.
  Its build, both new ABI gates, and `Run tests` steps had already succeeded.
- GitHub's step timestamps record approximately **2 seconds** for the
  empty-aggregate gate and **32 seconds** for the aggregate-copy gate.

The audited CI run is [here](https://github.com/ms178/lccc/actions/runs/38138582853);
the benchmark run is [here](https://github.com/ms178/lccc/actions/runs/38138582836).
A later push must be checked independently. No completed result for an older
head should be represented as green CI for the latest head.

### Grades and severity tally

These are subjective review grades under an explicit rubric, not measured
coverage percentages or probabilities of correctness.

| Dimension | Weight | Grade / 10 | Rationale |
|---|---:|---:|---|
| Frontend / call-ABI correctness and design | 30% | 7.5 | Strong exact-byte and ABI fixes; a new unscoped prototype update can contaminate another function's metadata. |
| Assembler correctness and contracts | 20% | 7.0 | NEON checks are well scoped; new RISC-V assertions generalize a special-case oracle rule incorrectly. |
| Verification and evidence integrity | 25% | 6.0 | Excellent differential gates, but three reproduced classes of validation weakness remain. |
| Reproducibility / operational safety | 15% | 8.0 | Good cache publication, output limits, and identities; dependency/mutation coverage and fatal-worker handling are incomplete. |
| Maintainability / communication | 10% | 8.0 | Useful provenance and explicit limitations; brittle source scans and small documentation/CI cleanup remain. |

**Code quality: 7.3/10. Engineering: 7.0/10. Weighted overall: 7.2/10.**

Findings: **no confirmed P0; five P1; four P2; two P3**. F11 is repaired by the
documentation revision, leaving **ten open findings**. Three P1s have executed
Python witnesses; two have high-confidence static/source evidence. F02's main
runtime behavior predates this PR, but the newly added tests and explanation
enshrine its incorrect generalization. It is not labeled a newly introduced
runtime regression.

## 2. Choices worth keeping

1. **Separate body metadata from callable prototypes.** A K&R body's declared
   `float` must remain available for body narrowing while the call ABI receives
   a default-promoted `double`. A real prior/later prototype must constrain calls.
   This is the correct conceptual separation; F01 concerns entity/scope identity,
   not the decision to introduce it.
2. **Treat zero as a real regparm override.** Leaving the enclosing function's
   register budget behind for a cdecl callee is wrong. Unconditionally assigning
   the selected callee budget is simpler and correct.
3. **Copy object bytes, not rounded ABI slots.** Stack reservation and source
   readability are different quantities. The guard-page fixture attacks that
   distinction directly, including fastcall and partial-word tails.
4. **Bound emitted instruction count for large arguments.** A 2-KiB switch to
   `REP MOVSB` is a reasonable engineering cutoff for pathological code size.
   It bounds the emitted copy sequence, **not** runtime copy work, and should
   not be sold as an optimal performance threshold. Source staging before
   temporary pushes and preservation of argument homes are appropriate.
5. **Preserve i686 ESI/EDI at function entry when needed.** Temporary saves
   alone would describe current local homes rather than the caller's original
   registers during unwinding. The added prologue saves fit the existing
   post-peephole CFI design.
6. **Restrict arrangement checks to NEON three-same leaves.** A blanket rule
   would reject legitimate widening/indexed operations. Shared checks plus
   both negative and positive examples are the right direction.
7. **Use typed alignment errors, not diagnostic substring matching.** The
   error variants improve control flow while retaining established text.
   They do not, however, justify ignoring errors that GAS actually reports.
8. **Keep expansion-work accounting conservative and observable.** Testing
   nested macro billing without materializing megabytes is good engineering.
   Describe the ceiling as an expansion-work/resource policy, not an exact
   maximum on final output lines or complete GAS acceptance equivalence.
9. **Provision corpus sources, not a compiler.** Resolve the upstream ref once,
   verify selected Git blobs, lock publication, and preserve the old tree on
   failure. The cache exclusion and sparse checkout respect repository size limits.
10. **Avoid same-compiler-only ABI proofs and performance overclaiming.** The
    GCC/LCCC directions, volatile inputs, noinline boundaries, licensed upstream
    kernel, and distinction between simulated instruction counts and speed are
    all worth retaining.

## 3. P1 — required before merge

### F01 — prototype refinement still uses an unscoped function name

**Location:** `src/frontend/sema/analysis.rs:653–687`, with downstream use at
`src/ir/lowering/lower.rs:1254–1260`.

`analyze_declaration` updates `self.result.functions[init_decl.name].prototype`
for every function declaration, regardless of the declaration's owning scope or
function entity. Nested definitions use a mangled key, but their forward
prototypes still reach this plain-name update. The scope-aware call lookup
therefore does not make the whole metadata pipeline scope-aware.

A valid GNU C counterexample to cover in CI is:

```c
__attribute__((noinline)) int f(int x) { return x + 1; }
int g(void) {
    auto double f(double);
    double f(double x) { return x * 2; }
    return f(1.5) == 3.0 ? 0 : 1;
}
int main(void) { return f(41) != 42 || g(); }
```

The local `auto double f(double)` can replace the global `f` record's prototype
while leaving its body's `int` parameters in `params`. Lowering then chooses
`fi.prototype` for a non-K&R definition. This is a direct route from a distinct
nested entity into the file-scope function's argument metadata. The exact
compiled consequence needs the new CI regression; the erroneous metadata
mutation is visible in the source and was not present in the old update logic.

**Implementing-agent instructions:**

- Keep body information separate, but bind prototype refinement to semantic
  entity and scope identity, not a basename. Preserve GNU `auto`/nested identity
  in the AST or symbol metadata if the parser currently discards that distinction.
- A nested forward declaration must refine its own nested definition, never a
  same-named file-scope `FunctionInfo`. Do not replace a scoped declaration type
  with a type reconstructed from an unrelated TU-global entry.
- Preserve the type visible at each call site. A final TU-wide prototype table
  must not retroactively impose an out-of-scope/later declaration on a call.
- Add semantic assertions that global `f` retains `int`/`int` and nested `g.f`
  retains `double`/`double`. Cover two same-named nested functions in different
  scopes, local forward declarations, prior/later K&R prototypes, and function
  pointer shadowing.
- Add a mixed GCC/LCCC boundary test for the file-scope function, not merely a
  same-LCCC executable that could agree on the same wrong ABI. Run target and
  optimization matrices in GitHub CI, not in this harness.

**Acceptance:** the new negative metadata regression fails on the audited
implementation; scoped metadata and two-way external ABI checks pass on the fix;
all eight existing K&R semantic tests remain intact.

### F02 — new RISC-V tests certify an over-broad cap rule

**Location:** `src/backend/riscv/assembler/parser.rs:407–452,1378–1407`;
`src/backend/asm_expr.rs:806–817`; writer state at
`src/backend/riscv/assembler/elf_writer.rs:548–580`.

The parser discards `.p2align`'s maximum unconditionally and treats a `.balign`
`BadMax` as success. The new tests insist on those outcomes, including dropping
the maximum for an explicit fill and accepting an undefined `junk` maximum.

The upstream source says something narrower:

- [`gas/read.c:1562–1675`](https://github.com/RTEMS/sourceware-mirror-binutils-gdb/blob/binutils-2_47/gas/read.c#L1562-L1675)
  reads the maximum before invoking alignment emission.
- [`tc-riscv.h:57–62`](https://github.com/RTEMS/sourceware-mirror-binutils-gdb/blob/binutils-2_47/gas/config/tc-riscv.h#L57-L62)
  bypasses generic alignment only for **no explicit fill, a code section, and a
  successful RISC-V alignment hook**.
- [`tc-riscv.c:5439–5460`](https://github.com/RTEMS/sourceware-mirror-binutils-gdb/blob/binutils-2_47/gas/config/tc-riscv.c#L5439-L5460)
  declines that hook when relaxation is disabled or the requested alignment is
  no larger than instruction alignment.
- [`gas/read.c:812–853`](https://github.com/RTEMS/sourceware-mirror-binutils-gdb/blob/binutils-2_47/gas/read.c#L812-L853)
  passes the maximum to the generic data, explicit-fill, and code-alignment
  fragments when the hook did not handle the directive.
- [`get_absolute_expr:534–551`](https://github.com/RTEMS/sourceware-mirror-binutils-gdb/blob/binutils-2_47/gas/read.c#L534-L551)
  reports a non-absolute expression. An undefined `junk` maximum is not made
  valid by a later emitter choosing to ignore the numeric cap.

For example:

```asm
.data
.byte 1
.p2align 4,,2
next:
```

The generic maximum is 2, but the required padding is 15: GAS skips the padding;
LCCC has erased the cap before the writer can examine it. The same problem
applies to an explicit-fill code case and non-relaxed code. This source-level
counterexample moves a subsequent symbol by 15 bytes; it is not merely a
wording disagreement.

There is an additional early-error trap: a `BadMax` returned before the arity
check can be swallowed, hiding a fourth operand. Returning `None` also loses a
previously parsed explicit fill. Those issues existed in the old string-based
handling too; the enum conversion did not cure them.

**Implementing-agent instructions:**

- Preserve the parsed fill and maximum in the RISC-V IR. `.balign` needs to
  carry its cap too; it cannot store only `{ bytes, fill }` if generic paths
  require the maximum.
- Keep structural validation separate from target-specific emission. Check
  extra operands and invalid expressions even when a valid numeric cap will
  later be ignored by an applicable target hook.
- Make the writer choose cap behavior from section flags, explicit-fill state,
  relaxation state, and instruction-alignment conditions. Do not decide it
  unconditionally in the parser.
- Replace the incorrect expected outcomes and correct the misleading comments
  and follow-up notes. Keep typed errors and accurate diagnostic pins.
- In CI, cover at least the **48-cell** matrix:
  2 directives (`.p2align 4`, `.balign 16`) × 2 sections (`.text`, `.data`) ×
  2 fills (absent, `0xff`) × 2 relaxation states × 3 caps (0, 1, 20), at a
  deliberately unaligned offset. Also test bad/undefined maxima, surplus
  operands, and the unsigned-maximum overflow boundary used by GAS.
- For generic paths, compare bytes **and symbol offsets**, not just success.
  For true RISC-V relaxation paths, distinguish relocatable-object padding
  from final linked layout. Do not copy worst-case GAS padding into LCCC's
  current non-relaxing layout indiscriminately or hide differences in a broad
  exception bucket.

**Acceptance:** generic data/explicit-fill/non-relaxed caps match the pinned
oracle; malformed maxima and fourth operands reject; the special relaxation
case has a narrowly documented, independently checked contract.

### F03 — reference execution signals become eligibility skips

**Location:** `scripts/x86_gcc_torture.py:510–516,791–813`.

`reference_status` treats negative return codes as `reference-fail` only in the
compile phase. A reference executable killed by SIGKILL, SIGSEGV, or SIGABRT
instead becomes `reference-run-skip`.

**Executed witness:** the actual `main`, with tool operations mocked, processes
one pass and one reference SIGKILL, exits **0**, writes `complete:true` and
`fatal:null`, and publishes a row with **1/1 pass, 0 validation failures,
0 reference failures, 1 skip**. Preflight does not protect against a later OOM.

**Implementing-agent instructions:**

- Treat signal termination/resource failure as reference failure in **both**
  phases. If any exceptional signal is deliberately considered ineligible,
  require a narrow, justified eligibility rule rather than a blanket negative
  exit-code allowance. SIGKILL must never be an unsupported-test verdict.
- Keep reference failure separate from an LCCC defect, but make the overall
  validation exit nonzero and retain its diagnostics in failure output.
- Expand the classification contract across both phases for signals and
  124/125/126/127. Add a mixed pass-plus-reference-failure `main` test; an
  all-skipped test alone cannot detect this bug.

**Acceptance:** the witness no longer exits 0 or hides the failed oracle in the
denominator; the evidence row counts one reference/validation failure.

### F04 — report length is not report integrity

**Location:** `scripts/torture_evidence.py:38–70`.

The consumer checks completion only when `schema == 2`, then checks length
without checking unique expected case keys or declared configuration membership.
It summarizes only flags listed in `data['flags']`.

**Five executed witnesses:**

1. `schema:3`, `complete:false`, and an inconsistent expected count are accepted
   and summarized as a passing row.
2. Two identical passing `(test,flags)` results can replace a missing case and
   become **2/2 pass** because the lengths agree.
3. A failing `-O3` result in a report declaring only `-O2` is omitted from the
   numeric rows. A note mentions the failure, but the row remains **1/1, 0 fail**.
4. A completed, non-fatal all-skipped schema-2 report is accepted and described
   as having zero reported failures.
5. Boolean case counts are accepted because Python's `True == 1`.

The current producer rejects duplicate selections; these are consumer tests of
corrupt, mismatched, or future-format reports, not a claim that its normal writer
produces all five malformed inputs. A fail-closed consumer must nevertheless
refuse them rather than reconstruct successful-looking evidence.

A compact duplicate-key fixture is:

```json
{"schema":2,"complete":true,"fatal":null,"mode":"execute",
 "expected_cases":2,"completed_cases":2,"flags":["-O2"],
 "results":[{"test":"a.c","flags":"-O2","status":"pass"},
            {"test":"a.c","flags":"-O2","status":"pass"}]}
```

**Implementing-agent instructions:**

- Explicitly accept supported schemas and reject unknown/missing schemas. If
  schema 1 remains needed, identify it as archival/legacy evidence, not as
  completion-verified validation equivalent to schema 2.
- Require real integer counts (not booleans), valid mode/configuration/status
  values, unique configuration labels, and unique `(test,configuration)` keys.
- Have the producer record a source/configuration manifest from which the exact
  expected case set can be derived. Validate set equality, not just length.
- Reject undeclared flags and missing/extra/duplicate cases. Ensure all results
  reach a row; never let a malformed declaration hide a failed result.
- Recompute counts consistently and check provided counts against the results.
  Validate other numeric result fields for type/finite range.
- Reject all-skipped validation as such while still allowing a genuinely
  completed **failed** run to be summarized honestly. Failure data is useful;
  absence of eligible evidence is not success.
- Put the validation in a reusable function, exercise the real consumer CLI,
  and use the existing atomic publication helper for output rather than a
  shared fixed `.json.tmp` name.

**Acceptance:** all five witnesses fail closed; valid success and valid failure
reports remain usable; legacy support is explicit and cannot bypass current
completion checks.

### F05 — Expat differential validation compares success, not the test set

**Location:** `tests/workloads/expat-2.8.5/run.sh:101–107,142–145`.

Both JUnit reports must be nonempty and contain no failure/error/skip children.
However, their testcase identities are never compared. Both CTest processes can
succeed while the candidate configuration has omitted an upstream test.

**Executed witnesses using the actual final validator block:**

- Reference tests `{alpha,beta}`, candidate tests `{alpha}`: summary still says
  **`status:pass`, `complete:true`**, with counts 2 versus 1.
- Reference `{alpha}`, candidate `{beta}`: also passes, even though counts match.

XML tool calls were mocked to isolate the JUnit logic; no Expat compilation or
binary execution occurred in this harness.

**Implementing-agent instructions:**

- Extract the Python validation logic into an importable, compiler-free module.
- Compare stable testcase identity **multisets** (not just counts), require
  meaningful identities, and handle duplicates explicitly. Cross-check declared
  suite totals and failure/skip state with the actual entries.
- Persist an expected/discovered CTest inventory and relevant configuration
  manifest. Compare the intended Expat feature settings, allowing only genuine
  compiler/path differences. Equal test names alone must not excuse a weaker
  candidate configuration.
- Add omitted-test, equal-count/different-test, duplicate, empty, skipped, failure,
  and configuration-mismatch fixtures plus a valid positive control.
- Keep the existing five nonempty canonical XML comparisons and three specific
  rejection-status checks. They complement upstream coverage; they do not prove
  it is complete.

**Acceptance:** both witnesses fail before a passing summary is published;
identical complete configured suites pass in CI; differing features/test sets
are explicitly rejected or reported as non-comparable, never success.

## 4. P2 — necessary for a 10/10 evaluation

### F06 — recorded identities are not immutable execution inputs

**Location:** `scripts/x86_gcc_torture.py:565–600,731–734`.

The source digest covers selected C files, not included headers. Binary hashes
are captured once, but compiler execution uses the original mutable paths.

**Executed witness:** changing an included `fixture.h` from `VALUE 0` to `VALUE 1`
leaves the **entire `run_identity` result unchanged** when the C file and tool
identities are unchanged. This is a reproducibility limitation, not evidence
that the selected-C-file hash itself is computed incorrectly.

**Implement:** bind reports to verified corpus/dependency manifests and relevant
SDK/header identities; use stable/private tool copies or reject changed binaries
and sources with post-run checks/appropriate locks. Keep the identity capture
before execution; adding only an after-run hash would mislabel earlier cases.
Test included-header changes, source edits, compiler replacement during a mocked
matrix, and unaffected stable runs. State the remaining system-library boundary
explicitly if a complete dependency inventory is not implemented.

### F07 — worker exceptions leave poor evidence and do not stop queued work

**Location:** `scripts/x86_gcc_torture.py:752–779`.

`future.result()` is not guarded. The executor context waits for submitted work
on exception, and no updated fatal report is written.

**Executed witness:** with one worker and four cases, the first raises `OSError`;
all four cases still execute, the exception escapes `main`, and the checkpoint
retains **0 completed cases, `fatal:null`, `complete:false`**. The incomplete
checkpoint is correctly not green; this is a diagnosis/resource-control defect.

**Implement:** record fatal infrastructure exceptions with case/phase identity,
cancel work not started, drain/terminate active work under existing bounded
process policies, publish the final incomplete/fatal checkpoint, and return a
controlled nonzero code. Do not turn harness errors into compiler defects.
Tests must verify cancellation, retained completed results, no leaked processes,
and preservation of previous report data on publication failure.

### F08 — textual censuses still admit blind spots

**Location:** `scripts/gen_aarch64_dispatcher_goldens.py:214–245`;
`src/backend/arm/assembler/encoder/mod.rs:2770–2843,3190–3290`.

The first column-zero `}` is not necessarily the function's closing brace. The
NEON census also uses substring presence to stand in for an executed helper call.

**Executed dispatcher witness:** a valid Rust function with two mnemonic arms
and a preceding raw string containing a column-zero `}` returns an **empty arm
set**, without error. The current fixed TSV detects drift, but a generator and
consumer sharing the same blind spot can certify it after regeneration. The
new source bound is an improvement over EOF fallback, not a proof of coverage.

**Implement:** derive boundaries and calls from structured metadata or at least
lexical scanning that ignores strings/comments and tracks braces. Require
nonempty coverage in producer and consumer. Add raw-string, commented helper,
visibility/formatting-change, and new-leaf fixtures. Behavioral acceptance tests
must exercise all registered three-same families, and widening/indexed positive
controls must continue passing. Do not cure the problem by editing a magic count
or regenerating a smaller golden file until the gate goes green.

### F09 — ABI matrix is strong but not the complete contract claimed by a 10/10

**Location:** `tests/regression/check_aggregate_call_abi.sh:20–44`;
`i686_regparm_cdecl_transition.flags`; i686 prologue/copy additions.

The gates cover both widths, five levels, all relevant byte tails, threshold
edges, multiple aggregates, guard pages, and both GCC directions. The transition
fixture's comment calls for TU-default regparm 0–3, but its committed flags select
only the ordinary `-m32` configuration. Explicit frame-pointer/PIC controls and
unwinding during the temporary-save window are not directly pinned by these
new gates. Bulk aggregate varargs are also not covered by the new fixture.

**Implement:** add bounded targeted cases for TU-default regparm 0–3,
regparm(0) and unannotated callees, explicit FPO/frame-pointer/PIC modes,
bulk aggregate varargs, and live values across direct/indirect bulk copies.
Check unwind metadata during REP with the existing CFI contracts or a dedicated
CI-only unwinding probe. Verify bounded assembly size at/above the cutoff while
keeping all guard-page tests; do not choose a cutoff from an invented speedup.
Use a documented fast/extended split if needed without weakening required ABI
coverage. Current GitHub timing suggests the existing gates are affordable.

## 5. P3 — cleanup

### F10 — duplicate multilib installation

**Location:** `.github/workflows/ci.yml:28–32,121–130`.

The workflow already installs `gcc-multilib`, `g++-multilib`, and
`libc6-dev-i386` before the build. The newly added second installation repeats
part of that contract. It is not a missing-dependency bug; the observed duplicate
step took approximately zero seconds in this CI run.

**Implement:** keep one explicit setup step before all consumers, make the ABI
gates depend on that contract, and correct the comments. Do not remove multilib
or turn infrastructure failures into skips.

### F11 — malformed README row (fixed in the documentation revision)

**Location:** `scripts/README.md`, script catalog.

The `torture_evidence.py` row was appended to the chroot row with `||` instead of
a newline. The revision separates the rows. Add/retain lightweight Markdown
structure checking if this catalog is extended further. No compiler behavior
changes in this repair.

## 6. Implementing-agent execution order and 10/10 bar

1. **Stay on the existing PR branch.** Use focused commits per root cause; do not
   reapply the original import, erase negative results, or mix unrelated optimizer
   work into these fixes.
2. **Close F01 and F02 with real semantic/oracle contracts.** These concern valid
   program types and emitted layout. Source models or tests that merely repeat
   the implementation's assumption are not sufficient.
3. **Close F03–F05 using compiler-free failure injection first.** Reproduce the
   existing defects, then make the witnesses reject/fail appropriately while
   preserving positive controls and honest failed-run reporting.
4. **Close F06–F09 for the full grade; remove F10 duplication.** Each needs a
   negative contract and a stable-run control, not only explanatory prose.
5. **Keep the no-local-compilation policy.** Run inspected Python/shell contracts
   and syntax checks locally. Add C/Rust/assembler coverage for GitHub CI to run.
6. **Require green CI on the final code head.** Confirm both ABI gates, Rust
   tests, Clippy, oracle checks, and the benchmark workflow; separately label the
   manual Expat workload until a supported CI job runs it. Do not compare only
   return codes where byte layout, case identity, or configuration is the claim.
7. **Update evidence and the PR description truthfully.** Separate observed CI
   results, prior local campaign results, static reasoning, and unavailable
   measurements. Preserve the distinction between simulated instructions and
   performance, between reference failures and candidate defects, and between a
   licensed kernel benchmark and an end-to-end application result.

My **10/10 bar for this focused PR** is: no open P1/P2 findings from this audit;
all new negative cases demonstrate the intended failure mode; all positive
controls and existing protections remain; exact entity/layout/report contracts
are checked independently; and CI passes on the final head. This is not a claim
that the entire compiler has no remaining bugs.

**Minimum merge bar:** all five P1s resolved or convincingly refuted with
specific counter-evidence, plus green final-head CI. The P2s can be sequenced as
follow-ups only if their limitations remain explicit; doing so would not earn
10/10. The current description/documentation improvement alone does not meet
that bar.
