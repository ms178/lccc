# 2026-08-21 session 33 — sema constraints, per-loop PGO, and copy64 parity

Base: `00321224f347cbecd0e717194c9315609e56901c` (PR #158 merged).

## Delivered results

### Six-instruction 64-byte copy

DCE may remove dead positional parameter homes only in a proven 64-bit,
single-block leaf where every `ParamRef` occurs in the leading prefix and no
call/inline asm can clobber incoming ABI registers first. This removes the
24-byte frame and redundant `%rsi -> %rax` materialization from the isolated
64-byte assignment while preserving parameter homes everywhere else.

Compiler Explorer, `-O2 -march=x86-64-v3`:

| compiler | instructions |
|---|---:|
| LCCC | 6 |
| GCC 16.2 | 6 |
| Clang 22.1 | 6 |
| ICC 2021.10 | 9 |
| ICX latest | 6 |

The six instructions are four YMM moves, `vzeroupper`, and `ret`. The existing
32/48-byte XMM policy is unchanged.

### Active per-loop PGO vector cost

The dead function-level `vectorize_gate` was replaced with a gate called from
the natural-loop traversal. Exact profiled trip counts below 8 veto
vectorization; bodies over 80 instructions require at least 32 iterations.
Missing profile data preserves the static policy, and PGO never overrides
legality. Unit tests cover both thresholds; `LCCC_PGO_NO_VECTOR_GATE` is the
A/B switch.

### Sema constraint enforcement

Semantic analysis now carries current return type and rejects valued returns
from void functions, valueless returns from non-void functions, named aggregate
assignment mismatches, incompatible pointer assignments, fixed-prototype arity
errors, and the same arity errors through function pointers. Empty legacy
`f()`/fallback declarations remain permissive, and anonymous SIMD aggregate
identity remains fail-closed against false diagnostics. The regression includes
seven independent negative diagnostics and a valid variadic/void-pointer control.

### Target discrimination correctness

The shared FP-home selector previously identified x86-64 solely from
`PhysReg(1)` plus pointer width. RISC-V also has `s1 == PhysReg(1)`, so F64 values
could receive nonexistent x86 XMM homes (`PhysReg(20+)`). Requiring x86's
caller-saved `%r10` marker (`PhysReg(10)`) prevents cross-target register-class
leakage; a standalone RISC-V F64 pointer kernel compiles successfully.

## Validation

- 971 unit tests passed, 6 ignored on rebased PR #158 main.
- 50/50 correctness passed in the validated pre-snapshot run.
- x86-64 regression corpus passed; three `-m32` executable checks depend on a
  working host 32-bit runtime and fail identically with upstream DCE.
- gzip 1.14: 30/30; `longest_match` retained 119 stack-memory references.
- phi CFG differential: 600/600.
- i686 alias differential: 540/540.
- Godbolt artifacts: `/home/user/artifacts/r6-copy64-parity`.
