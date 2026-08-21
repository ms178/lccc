# 2026-08-21 session 36 — GlobalAddr CSE ultimate wiring and placement audit

Base: `dffe0b19bea3bf4d23effbf7c1a82e5cda49888a` (PR #162, containing the #158 pass and its #161/#162 wiring fixes).

## Wiring audit

The three production invocation points are necessary and retained:

1. before the first GVN, while duplicate GlobalAddr instructions still exist;
2. after structural inlining/SROA, which can clone new address sites;
3. after iter-0 vectorization/unrolling, which can duplicate loop bodies.

GVN remains class-aware, the pass honors both kill switches, alias canonicalization is module-aware, and operand rewriting covers ordinary operands plus hidden value-use fields. O0 correctly skips the optimizer.

## Defects found and fixed

### Unconditional entry hoisting

The pass moved even one cold non-entry GlobalAddr to function entry. That performs no CSE, executes work on the untaken path, lengthens liveness, and can force callee-save pressure. GCC 16.2, Clang 22.1, ICC 2021.10, and ICX latest unanimously keep the cold singleton branch-local.

Final policy leaves non-loop singletons in place. On a 30-million-call cold-path VM screen, treatment/base median ratio was **0.98284** (20 paired rounds). This is VM screening, not Raptor Lake hardware evidence.

### Mutually-exclusive branch merging

Two sibling branches were merged at their nearest common dominator, eagerly materializing a value even though at most one branch executes. All four Godbolt oracles keep separate branch-local materializations. The pass now refuses synthesized common-dominator values outside loops and reuses only an occurrence that already dominates every other occurrence. LCCC's sibling reproducer improves 28 → 26 instructions; remaining call/epilogue gaps are unrelated.

### Loop placement

A loop GlobalAddr should not remain inside the repeated body, but it also should not cross a cold guard. The pass now finds natural loops and places a loop-executed value at the innermost containing loop header's immediate dominator. This matches Clang's guarded-loop strategy and keeps the address after the guard on x86-64, i686, AArch64, and RISC-V.

A 20-million-iteration paired VM screen measured pass-on/pass-off median ratio **0.75792** (20 rounds). LCCC emits 24 instructions for the simple loop oracle, versus GCC 26, Clang 20, ICC 23, and ICX 19.

### Derived variable-index bases

The site-local safeguard only recognized a GlobalAddr used directly as a variable-index GEP base. Copy, same-size Cast, constant GEP, and constant Add/Sub chains could hide the root and let CSE/GVN destroy x86 symbol+index selection. The classifier now walks unique-definition parent chains back to the originating GlobalAddr. Cycles and multi-def values fail closed.

### Intrinsic-location scope

The previous safety fix disabled the entire pass for every multi-block function containing any intrinsic. With lifetime-minimal placement that is unnecessarily broad. The pass now remains active for safe same-block and non-loop opportunities, but refuses a newly synthesized loop-preheader home when the containing loop has intrinsics. This preserves rdtsc/rdtscp and deferred-SIMD correctness without discarding unrelated CSE opportunities.

### Compile-time and metadata quality

- Functions with no movable groups or only entry singletons skip CFG/dominator/loop construction.
- Parent edges in must-materialize propagation are deduplicated rather than appended once per fixed-point iteration.
- Only blocks actually receiving/removing an instruction are rebuilt; unrelated source-span arrays are untouched.
- Insertions occur after Phi/Alloca/ParamRef prefixes and all rewrites remain dominance-safe.

## Oracle evidence

Artifacts:

- `artifacts/r9-gaddr-cold`: all four oracles branch-local;
- `artifacts/r9-gaddr-guarded-loop`: selected preheader policy and instruction counts;
- `artifacts/r9-gaddr-siblings-final`: all four oracles retain branch-local siblings;
- `artifacts/r9-gaddr-pressure`: must-materialize address strategies;
- `artifacts/r9-gaddr-cross`: AArch64/RISC-V/i686 placement;
- `artifacts/r9-gaddr-placement-evidence.txt`: paired VM statistics and base/treatment assembly findings.

The existing pass-counter test is strengthened with cold-singleton, guarded-loop, and sibling-branch placement gates. Unit coverage adds loop-preheader placement, dominating cross-block reuse, branch-local siblings, cold singletons, and derived indexed roots.

## Full validation and codec evidence

- 982 unit tests passed, 6 ignored.
- 50/50 correctness.
- 377/377 lccc-only regressions.
- 600/600 phi CFG differential.
- 540/540 i686 alias differential.
- gzip 1.14: 30/30.
- `longest_match`: **330 instructions / 118 stack references**, improved from pristine base **331 / 119**.
- gzip paired VM compression, 20 rounds: treatment/base median ratio **0.94266**.
- gzip text: **111,582 vs 113,582 bytes (-2,000 bytes)**.

The timing is VM screening only; no Raptor Lake hardware claim is made.
