# Joint audit: vector lowering and guarded scans

Integration base: `origin/main` `9a70f84a` (re-fetched during this audit).
Integrated heads: #862 `f44fe9d4`, #863 `20f22d6d`.
This is an audit checkpoint, **not closure of either follow-up backlog**.

## Reproduced defects and fixes

1. Guarded CTZ folding discarded volatile header instructions. Reject header
   effects outside the supported pure integer grammar.
2. The bit-test matcher accepted a nonzero comparison constant as zero.
   Require an actual zero operand; preserve comparison polarity.
3. CTZ replacement was inserted in the conditional preheader before its
   nonzero proof. Retain the original header as the guarded fast block, remove
   the other loop blocks, and leave the guard untouched. The zero edge never
   executes `CtzNonZero`.
4. Escape validation omitted switch terminators. Visit all terminator operands;
   replace the overflow-prone invalid-count sentinel with an explicit flag.
5. Exit phi rewrites did not consistently replace `Operand` uses. Use the IR's
   canonical replacement helper, including exit phis on the retained header
   edge. `for_each_value_use_mut` alone covers only bare `Value` fields.
6. Shared builds did not enable the external-inliner interposition restriction.
   Pass `pic || shared_lib`; verify executable symbol preemption with `dlopen`
   at `-shared`, `-shared -fPIC`, and `-shared -fno-pic`.
7. Byte/halfword vector SSA zero seeds were operand-less broadcasts. An opcode
   arity regression failed before the fix. Supply the scalar zero; classify
   parameter-load seeds by opcode rather than argument count so broadcasts do
   not interrupt the backend's required entry load prefix.

The first four targeted CTZ regressions all failed before their fixes. The
exit-phi regression additionally exposed the incomplete operand rewrite.

## Measured code quality

At `-O2 -mavx2`, whole-function instruction counts (including `ret`):

| Function | Integrated heads | Audited tree | GCC comparison in #862 notes |
|---|---:|---:|---:|
| `eq_v16i` | 4 | 2 | 2 |
| `lt_v16i` | legacy staging | 2 | 2 |
| `mix_sab4` | 7 | 6 | 4 |

Use the resolved AVX permission, not AVX2, for the pre-existing VEX.128
arithmetic fast path. Signed 128-bit byte/word/dword comparisons can use a
three-register VEX compare directly when both sources and destination are
register-homed. Unsigned and non-register-homed paths retain their existing
handling. Inverted masks use reserved scratch xmm1 and commit the destination
through the normal backend hook. Tightened instruction ceilings enforce the
measured improvements. These are static counts, not hardware cycle claims.

## Executed validation

- Rust 1.97.0, fastbuild, `RUSTFLAGS='-D warnings'`: **5190 passed, 10 ignored**.
- Fastbuild `lccc` and `lccc-ld`, warnings denied: pass.
- Clippy, all targets, fastbuild, warnings denied: pass.
- Formatting, `git diff --check`, audit census: pass.
- GCC 16.2.0 whole-vector runtime differential matrix: **11 runtime levels**,
  IR verification, instruction ceilings and FP contraction policy all pass.
  Replaced blanket GCC warning suppression with `-Werror -Wno-psabi` and made
  this gate reject other GCC versions.
- New guarded-scan runtime fixture: -O0/-O1/-O2/-O3/-Os, every 32/64-bit
  single-bit position, trailing-one masks, header volatile visit counts and
  explicit zero inputs pass against fixed expectations and GCC 16.2.0.
- Shared-object preemption runtime tests: all three modes pass.
- Full local regression runner: **998 passed, 9 failed, 4 skipped comparisons,
  14 skipped runs**. Every failure is `ccc: error: cannot find -lgcc` on a
  32-bit fixture. This is not reported as a fully green corpus.
- 8 GiB swap active throughout validation.

Local Clang 23.1.3, ICX 2026.2 and runtime execution on other architectures
have **not** been verified. Existing hosted workflows provision the pinned
oracles; their results must be checked on this integration head, not borrowed
from the source PRs. The new joint runtime gate is wired to local CI and the
hosted frontend job.

## Still open

The source documents remain the authoritative detailed backlog:
`FOLLOWUP-2026-10-17-ext-vector-grid.md` and
`../docs/FOLLOWUP-2026-10-18-ffs-ctz-and-extern-inline.md`.

Not closed here: packed 64-bit compares, byte multiplication expansion,
signed 64-bit variable arithmetic shifts, scalar-count splat staging, FMA
chain/two-result scheduling, struct/union wrapper copies, broader phi/call/
switch vector webs, volatile copies, vector lane-loop folding, static vector
helper inlining, shared ternary return homes, VEX.128 FMA staging, designated
vector initializers, general builtin shuffle, multilane compound swizzles,
vector-width preferences, runtime-trip partial unrolling plus overlapping-load
SLP for FIR, mutually exclusive global-store promotion, and bitreverse register
allocation. The mixed scalar/FMA case is improved, not optimal.

Also not established: sanitizer-clean execution of all existing vector matrix
inputs (random signed arithmetic requires a separate defined-semantics audit),
exhaustive property checking of all lowering combinations, or Intel 14700KF
cycle measurements. No zero-defect or universal performance claim follows
from the passing tests above.
