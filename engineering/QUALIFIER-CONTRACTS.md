# Qualified pointer semantics: contracts, ownership, and evidence

## Scope and non-claims

This change improves recorded CVR qualification in the shared frontend for
x86-64, i686, AArch64, and RISC-V. It is not a proof of complete C conformance,
absence of all compiler defects, or globally optimal generated code. Unknown
qualification remains explicit. `_Atomic`, full typedef diagnostic sugar and
construction paths without exact qualifier provenance are not reclassified by
guesswork. Existing independent read-only/volatile views are retained until
all their construction paths can be migrated safely.

## Decisions and rejected alternatives

| Decision | Reason | Rejected alternative |
| --- | --- | --- |
| Recover object CVR in expression inference, not conversion-only repair | Address-of subexpressions retain their records in cached types; array decay has an explicit value-context boundary | Repeating repairs at assignment, return, call and conditional sites creates expression-shape-dependent results |
| Preserve raw array inference and use `infer_value_ctype` for value contexts | `sizeof`, address-of and field selection must see arrays; arithmetic/comma/conditional conversions must retain element CVR | Unconditionally decaying raw inference changes array semantics |
| Compare recorded qualification through function signatures | Callback parameter/return pointees are semantically significant; parameter names and top-level parameter CVR are not | Qualifier-blind Eq or a pointer/array-only walker misses callback incompatibility |
| Keep Eq/Hash representation-oriented | Avoid a broad change to existing code-generation identity assumptions; the two traits remain consistent | Globally changing Eq without auditing every representation consumer |
| Give semantic callers explicit compatibility/conflict predicates | `ctypes_compatible`, `recorded_qualifiers_conflict`, and `has_known_cvr_conflict` state the intended operation | Implicitly assuming Eq compares all metadata |
| Join conditional immediate qualifiers while comparing inner types directly | C joins immediate qualification; no need to clone both outer pointer trees merely to strip CVR | Cloned temporary stripped trees or treating every qualifier difference as incompatibility |
| Store the colon in the existing conditional operator span | Constant-time lookup, no AST enlargement, survives preprocessor line markers; expression-start queries already traverse the condition | Scanning source text backwards or enlarging every conditional node with a redundant range |
| Use two stack slots for comparison operands | Removes a Vec allocation from each scalar comparison inference; no throughput claim is inferred from allocation count | Heap-allocating a two-element Vec |
| Fail closed on subprocess results | Error severity, exit status, driver summaries and unexpected stderr must agree | Searching only for expected primary diagnostics and dropping every other line |
| Require exact GCC by default | Accidental oracle absence/version drift must not weaken CI | Silent success without the specified oracle |

## Ownership map

- `PtrQuals`: complete recorded **pointee** CVR at one pointer level and its
  named address space. `None` is unknown; `Some(0)` is known-unqualified.
- `SymbolTable::object_cvr`: the visible object's own CVR, including the
  adjusted array parameter pointer. Lookup stops at the innermost binding.
- `TypeContext::typedef_top_cvr`: typedef own CVR with scoped undo; it is not
  a replacement for the typedef's resolved derived type.
- `StructField::own_cvr` / layout metadata: member own CVR. The expression
  checker combines this with containing-object CVR for member lvalues.
- Existing `ConstLookup`, pointer-level and volatile records: mutation,
  typeof and volatile-access views with coverage not yet identical to PtrQuals.
  They must not be deleted merely because the new diagnostic representation
  exists. Declaration construction remains responsible for populating views.
- `ExprTypeChecker`: authoritative expression qualification and explicit
  value conversion. `expr_types` memoizes enriched address-of/composite types;
  raw array types remain raw until the consuming value context.
- `ctypes_compatible`: semantic type compatibility with recorded pointee
  qualification, including callback parameter/return types.
- `recorded_qualifiers_conflict`: guards qualifier-blind equality shortcuts
  used by conversion policies. It reports only proven conflicts, not unknowns.
- `CType` Eq/Hash: representation identity; never a substitute for a semantic
  conversion verdict.

## Regression design

The original annotated corpus contains 115 primary diagnostics and checks
baseline, seven suppressions, and qualifier promotion. Every variant is now
compared with GCC for ordered wording/location as well as classification.
The oracle must report GCC 16.2.0; expectation-only use requires an explicit
local flag. Exit codes must match parsed error severity. Source excerpts and
known rendering/context lines are recognized deliberately; unexpected driver
failures, missing-column errors, panics, ICEs and foreign-file primary errors
are not silently dropped. LCCC summary counts must equal primary error counts.

The independent composition matrix has 26 hand-written cases plus all 64
CVR pairs at both immediate and nested pointer boundaries (154 cases total).
Each target/dialect/compiler combination also checks applicable suppression
and promotion, so warning-only and legal inputs independently exercise zero
exit status. The matrix runs GNU C90/C17/C23; `__restrict__` is used because
plain `restrict` is not a C90 keyword. GCC defines diagnostic policy; Clang
23.1.4 independently checks the hand-written portable cases with warnings
promoted. The authenticated vendor workflow runs ICX 2026.2 on those same
portable cases. Vendor wording/classification is not mistaken for GCC policy.

Rust contracts cover all 81 known/unknown CVR pairs, equality/hash consistency,
conditional qualifier joins, callback qualification, scope undo and cached vs
uncached inference. Offline gate tests exercise malformed subprocess results.
Diagnostic-position coverage includes nested/GNU conditionals, macros and a
#line boundary. The existing position gate now compiles its input translation
unit rather than accidentally compiling the file named in a diagnostic.

`qualifier_expression_composition.c` is a legal executable control for pointer
stepping, conditional/comma expressions, callbacks, array pointers and const
members. It is discovered by the existing target regression corpus.

## Recorded local observations

- The pre-change compiler missed 57 results in 78 hand-written
  case/dialect combinations; GCC matched every stored expectation.
- The fixed four-driver matrix passed 5,726 checks with GCC 16.2.0 and
  Clang 23.1.4 (154 cases, three dialects, suppression/promotion included).
- The annotated corpus passed 27 logical checks across its nine variants.
- Operand diagnostics passed; position diagnostics passed 16/16.
- Diagnostic red-team gate passed 1,431 contracts against GCC 16.2.0.
- The legal runtime control passed host LCCC, the baseline compiler, GCC and
  Clang at O0/O2/O3 with `-Wall -Wextra -Werror` (12 combinations).
- Host O2 objects for that control were byte-identical before and after the
  change. This is a code-generation non-regression witness, not a benchmark
  or a claim about every workload.
- The warning-compatibility gate passed while reporting its existing GCC-only
  known-gap rows; those are not claimed fixed by this feature.
- Local Intel PPA access is outside the sandbox's outbound host allowlist.
  Vendor execution belongs to the authenticated ICX workflow, not a fabricated
  local comparison.

Builds use the fastbuild profile with warnings denied. The first monolithic
library-test compilation was killed by the host with 1 GiB swap; the follow-up
uses 9 GiB active swap and fastbuild's debug/incremental overrides to constrain
memory. A killed build is not reported as a passing test.
