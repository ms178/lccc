# PR #664 follow-up: typed constant decisions and oracle integration

## Scope and decision

Base reviewed: `81667ca4fab3bae877308885af059cb86aa44119` (main after #664).
The review AI identified real omissions, but its counts, manifestation labels,
and proposed evaluation contract are not authoritative. This follow-up keeps
changes bounded to CFG evaluation, constant truthiness, and validation tooling.
It does **not** establish that every optimizer is safe for wide integers.

Historical operational details were moved to
[the session archive](../../engineering/session-logs/2026-09-28-cfg-redteam.md).
This document records durable contracts, disagreements and remaining hazards.

## Audit adjudication

| Finding / claim | Decision and evidence |
|---|---|
| F1: CFG still narrows wide Cmp/Select/Switch values | Agree. Five new CFG tests fail on the base: local/global comparison, local/global Select and Switch. Their failures were obtained through the fast CI unit gate, not inferred from source alone. |
| FU-1: comparison signedness must come from `ty` | Disagree as an IR implementation rule. `IrCmpOp` explicitly encodes Slt/Sle/Sgt/Sge versus Ult/Ule/Ugt/Uge. Type determines width; opcode determines ordering; the constant variant determines neither. C usual arithmetic conversions happen before this instruction. |
| FU-1: reject a Switch constant with bits above bit 63 | Unnecessarily pessimistic. The existing O0 path already compares masked full-width patterns. Both paths now use one helper, including narrow unsigned case normalization. A wide value that matches no representable case can legally select default. Cases themselves remain i64 in this IR; this patch does not add arbitrary 128-bit case labels. |
| F2 / FU-0: repo-wide scope | Agree that triage was missing. A revision-pinned lexical inventory and source-reviewed hazard table are now included. The exact quoted 413/68 total is not reproduced by our explicitly documented method: 431 method-call candidates in 67 files on this base. Candidate count is not defect count. |
| Credit #3: CFG Select-arm merging at old line 773 | Incorrect location/description. `would_create_phi_conflict` uses equality to protect jump threading through **Phi** inputs; it is not Select-arm merging. A real Select-arm equality risk exists in `constant_fold.rs`, independently of that CFG consumer. |
| F4: `(unsigned __int128)u64max` sign-extends on this base | Disagree with the example. `IrConst::from_i64(_, U128)` already zero-extends through u64. The new test for the exact example passes **before** the fix. Other directions do fail by source reasoning: U64→I128 and I64(-1)→U128 need source-aware extension. Both are covered by the new cast matrix. |
| F3: Python test discovery unwired | Agree; fixed in hosted workflow, fast local gate, parity metadata and mutation tests. Discovery covers `test_*.py`, not just today's file. |
| F5: malformed Phi fixture | Agree. It is now a four-block diamond with real predecessor blocks and a structural-verifier assertion. |
| F6: runtime taxonomy loses triage data | Partly agree. The prior JSON already retained separate `gcc_rc`/`ccc_rc`; raw attribution was not lost. Dedicated `reference-fail`, `candidate-fail`, `both-fail` status buckets improve summaries and are now tested. |
| F7: blanket ignore rule | Agree; `test_*` is scoped to repository-root scratch outputs. Subdirectory source tests no longer need individual negations. |
| F8: documentation placement | Agree on separating operational history from contracts. The earlier report is archived under engineering/session-logs; durable findings live here. |
| “Strictly monotone ⇒ provably safe” | Too strong as a whole-pass proof. #664 rejects more equality pairs, which is a good local argument; it does not prove the existing IR representation, pass ordering or other consumers correct. Tests are evidence, not a universal proof. |
| Review's CI statuses / “local harness cannot build” | This workspace can build. The public check-runs API for PR head `25a3730d` now reports Clippy, Test Suite and LCCC vs GCC completed/success. This is distinct from our local validation and does not expose a per-test execution log. |

## Implemented contracts

### CFG integer evaluation

1. Operand resolvers return `Option<IrConst>`; no i64 extraction/evaluation
   remains in `cfg_simplify.rs`.
2. Mask integer operands to their operation width before comparing. Signed
   predicates sign-extend the masked bits to i128; unsigned predicates use
   u128 ordering through `IrCmpOp::eval_i128`.
3. Casts normalize **source** width, sign/zero-extend according to source type,
   then reduce modulo destination width. Unsigned sub-64-bit results retain
   the established nonnegative I64 carrier convention. Pointer operations
   use target pointer width; floats and Void are not integer-folded.
4. Select's integer truth predicate observes all 128 bits. Unsupported float
   Select/Cmp/Cast evaluation returns None, not a guessed integer result.
   Existing direct constant branch truthiness is retained.
5. Direct and resolved Switch evaluation share the same width normalization.
6. Local recursive resolution has the same depth bound as global resolution,
   preventing self-referential Select IR from unbounded recursion. This is a
   stack-safety bound, **not** a proof of linear work on shared DAGs.
7. Phi identity remains distinct from typed comparison; #664's full-width
   cross-carrier numeric equality and exact floating bit keys are preserved.

### LongDouble truth

The full 16-byte binary128 payload is authoritative. Masking its sign bit
recognizes both zeros and no other encoding; normals, subnormals, infinities
and NaNs are nonzero. The approximate f64 cache is not consulted. The new test
fails on the base and covers both signs and a deliberately inconsistent cache.
This fixes the constant representation predicate, not every target-specific
rounding or long-double arithmetic issue elsewhere in the compiler.

### Regression evidence

The red run of `ci_local.sh --fast --only cargo-test` reports six failures:
five CFG probes plus LongDouble truth. The exact unsigned-widening example
from F4 passes, serving as a useful control. New boundary tests cover all ten
integer types, both signed/unsigned opcodes, literal and Copy-fed casts,
Switch widths, float no-fold behavior and bounded local cyclic resolution.
The cast matrix's additional directions were added after the red run; do not
claim a separate measured red run for every matrix cell.

Fuzzer taxonomy tests fail in seven subcases before the status change and pass
after it. They assert the four-call contract and command roles. Parity mutation
tests reject commented/echoed discovery, a pinned single-test pattern, `|| true`,
and moving the local gate from fast to slow.

All local validation uses `ci_local.sh --fast` (including selected gates while
iterating), rustfmt and Clippy. The slow suite and standalone benchmark/fuzzer
runs are intentionally left to GitHub CI per the session request. The C test
`cfg_phi_i128_high_bits.c` remains in the regression runner's `*.c` corpus;
this session does not claim to have inspected a hosted Test Suite execution log.

## Repo-wide narrowing inventory and hazard register

[Machine-readable base inventory](../../engineering/evidence/pr664-followup/integer-narrowing-base.csv).
Regenerate it with:

```sh
python3 scripts/inventory_integer_narrowing.py --revision 81667ca4fab3bae877308885af059cb86aa44119
```

The script enumerates tracked `src/**/*.rs`, matches `.to_i64(` / `.eval_i64(`
(with optional whitespace), and excludes whole-line comments. It emits revision,
path, line, column, method, broad review category and nearest preceding function.
Strings/block comments and the first-`#[cfg(test)]` heuristic mean this is a
**candidate inventory**, not a Rust AST reachability proof. There are 320
semantic-priority candidates, 99 backend candidates and 12 other candidates;
137 occur after a test marker. No candidate is declared safe merely by its path.

| Priority | Location / consumer | Source-reviewed disposition | Required follow-up acceptance |
|---|---|---|---|
| Closed here | CFG operand/Cmp/Cast/Select/Switch paths | Narrowing removed; integer width contract tested | Preserve boundary matrix and fast CI |
| Closed here | `IrConst::is_zero` LongDouble | Approximation replaced by payload | Both signed zeros, smallest subnormal, normal, infinity, NaN |
| P0 | `constant_fold::try_fold_with_map`, Select equal arms | Still compares numeric arms using to_i64; wide alias risk. This is the actual Select identity site, unlike review's CFG citation | Base-failing tests with unknown condition and arms 0 / 2^64, both orders; preserve floating signed-zero/NaN behavior |
| P0 | `sccp::evaluate_terminator` and `fold_terminator` Switch | Both still narrow scrutinee before matching; executable-edge selection and final rewrite must use the same typed rule | Test both SCCP discovery and rewrite, default/case edge reachability, 32-bit unsigned and I128 |
| P0 | `ipcp::eval_const_function` Select/CondBranch/Switch | Three remaining i64 truth/dispatch decisions; Cmp/Cast already delegate to typed constant-fold helpers | Evaluate pure callees with high-half-only conditions and dispatch; interpreter and generated runtime agree |
| P0 | `if_convert::same_value_or_both_zero`, boolean-shape predicates | Unchecked low-half 0/1 tests can accept high-half nonzero constants. Reachability/type preconditions require targeted tests | Reproducer for each transformation family, no predicate changes based solely on grep |
| P1 | `constant_fold::fold_cast_i128` | Unsigned sub-128 source is extended from u64 rather than its actual width; normalize U8/U16/U32 carriers before widening | Tests for alternate valid carriers and every source width; preserve sign/zero extension |
| Guarded, not blanket-defective | `constant_fold::eval_binop_const`, `eval_cmp_const` | Explicit `ty.is_128bit()` dispatch precedes i64 fallback | Keep full-width dispatch and test narrow opcode/type interactions separately |
| Guarded, representation assumptions remain | `common::const_arith::eval_const_binop` | I128 operands dispatch to full-width evaluator before generic i64 path; i64 extracts in i128 helper are for explicitly non-I128 operands | Confirm promoted operand types/carriers; do not bulk-replace deliberate zero extension |
| Width-specific review | `common::const_eval` builtins | Many extracts feed explicitly u16/u32/u64 byte-swap/count operations | Check each builtin's declared operand width; preserve truncation where the operation requires it |
| P1 | `copy_prop` GEP offset combination; `ir/lowering/const_eval` | Offset representation and front-end type/range contracts determine legality | Boundary tests before asserting that an emission-size extract is a semantic bug |
| Separate audit | backend candidates | Encoding/materialization often intentionally selects low bits; not changed | Audit instruction width and high-half paths, not a global conversion replacement |

Remaining P0 rows are explicitly **not fixed** by this patch. The inventory is
not represented as 431 individually proved miscompilations, nor as a complete
whole-codebase audit. A reusable typed integer representation could eventually
serve all passes, but first its contract must reconcile existing canonical
constant-fold helpers, decimal/Float128 carriers, and target pointer widths.
A broad helper migration without per-consumer red/green tests would merely
move the same risks behind a new abstraction.

## Performance and follow-up policy

No runtime speedup, PMU result, i7-14700KF result, or superiority over other
compilers is claimed. Correct constant decisions remove wrong-folding paths;
that is the acceptance criterion here. Remaining opportunities include shared
DAG memoization/work budgets and merging the typed integer evaluator contract
across passes after adversarial tests. Measure phase interactions and emitted
code rather than promising gains from a wider integer type or cleaner IR.

Issue-ready requests, with acceptance criteria, are in
[the follow-up queue](../../engineering/issues/PR664-followups.md). They are
local documents, not claimed GitHub issues: no authenticated GitHub issue-write
interface was available in this session. The patch is cumulative against the
base above; commits/snapshots are reviewable independently, but separate PRs
were not created remotely.
