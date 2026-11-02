# Follow-up 2026-09-20E — PR #568 review audit, second adjudication round

## Scope

The hosted Review AI's audit of PR #568 was adjudicated item by item against
the code, with live reproduction attempted for every claim that asserted a
behavioral defect. The reviewer has no build or execution capability; every
"confirmed" below was reproduced on this machine before it was fixed, and
every fix was verified both ways (negative control on the pre-fix compiler,
green on the fixed tree).

## Adjudication

### 1. Full-domain `||` branch-chain polarity — CONFIRMED, already fixed in-flight

The in-flight commit `9280ac25d` (unvalidated when the session was
interrupted) is correct and is now validated: a range covering the compare
domain makes the OUTSIDE test constant FALSE, so the `||` chain's live
continuation is Bbody (both comparisons false), not Bexit. The unified
surgery is sound: Bexit loses both chain edges in either form, and Bbody's
retargeted phi arms are value-safe because the check block is
single-predecessor through Bcond (a path to Bcond extended by the edge is a
path to Bcheck, so any def dominating Bcheck dominates Bcond). 152 range
unit tests and the extended branch gate pass.

### 2. NonZero bitcount cross-pass contract — CONFIRMED, and worse than reported

The reviewer flagged proof maintenance across CVP → if-conversion → LICM →
simplify. The audit trail:

* LICM hoisting and GVN keying are sound as-is: NonZero ops never trap on
  any backend (x86 BSR/BSF leave the destination undefined; RISC-V Zbb and
  AArch64 clz are defined at zero), and their results are only consumed
  where the original guard still discards them.
* if-conversion's blanket `UnaryOp` admission is sound for the same reason:
  the speculated instruction's garbage is discarded by the merge select/phi.
* **The zero-guard select collapse was NOT sound.**
  `clz_ctz_zero_guard_replacement` accepted `ClzNonZero`/`CtzNonZero` arms
  and folded `select(x, ClzNonZero(x), W)` onto the arm — but that identity
  holds only for the defined-zero spellings. Reachability is real: on i686
  (no pre-CVP diamond conversion — the vectorizer's embedded if-conversion
  is x86-64-gated) and on x86-64 with `CCC_DISABLE_PASSES=vectorize`, the
  branchy guarded form survives to CVP, CVP specializes the arm under the
  branch proof, if-conversion then speculates the arm ABOVE its proof, and
  the collapse removes the only discard discipline left. Measured:
  `clz_if(0)` returned -846929913 (BSR's undefined destination) for source
  returning 32; the plain ternary reference computations miscompiled the
  same way. **Fixed** by rejecting NonZero variants in the collapse — when
  the nonzero fact does dominate the select, CVP's own fact-stack select
  folding (which runs both before and immediately after the dedicated pass)
  performs the identical rewrite soundly, so nothing is lost (verified on
  the dominating-proof shape: still a bare BSR under its guard).
* constant_fold's NonZero(0) refusal and the same-size-only condition-side
  cast peeling were re-verified sound and unit-pinned.

The gate `check_clz_zero_guard_fold.sh` now executes the C-level contract at
runtime over both pipeline topologies with shift-loop oracles that cannot
degenerate along with the intrinsic, plus the i686 assembly contract. Both
checks fail on the pre-fix compiler.

### 3. Peephole load hoisting (phi-diamond preinitialisation) — CONFIRMED, reproduced as a SEGFAULT

The one-move phi-diamond pattern hoisted the fallthrough arm's MOV above the
branch regardless of its source operand class. With a user-memory init,
`c ? *p : *q` compiled to an unconditional `movl (%rsi), %edi` before the
test, and `sel(0, NULL, &valid)` **segfaulted** on a pointer the source
never reads on that path. **Fixed** with `hoistable_mov_source`: only
register-to-register moves, immediates, and plain displacement-only stack
slots (`-N(%rbp)`/`N(%rsp)`, no index/scale — a scaled index can leave the
frame) may be hoisted. New unit tests pin the rejections and the surviving
hoists; the new `check_peephole_phi_hoist_safety.sh` gate runs the C-level
contract over both branch polarities, load/reg and 64-bit arm mixes, and the
indexed-address arm, plus the register-init positive control (the
optimisation survives its own safety fix). The gate segfaults on the
pre-fix compiler.

### 4. Assembly branch relax and relocation addends — VERIFIED SOUND (with a tooling foot-gun removed)

The relaxation engine and the short-only loop/jecxz patcher both resolve
`.Ltarget+K` through `jump_target_with_addend` (split with the same parser
grammar as relocation emission, applied exactly once, checked arithmetic
that can never wrap an invalid target into the section). The whole-object
i686 differential passes 21/21 including the reloc-addend casefile. The
session initially mis-verified this as failing: naming an i686 casefile
without `--32` assembles the GNU as oracle as ELF64 and reports spurious
encoding/relocation mismatches. `asmdiff.py` now refuses that invocation
with the remedy in the error message.

### 5. `RangePlan` shared i128 domain — VERIFIED SOUND

Domain-ordered emptiness (wrapped unsigned windows rejected), i128 span
arithmetic (no overflow for <=64-bit domains), explicit I128/U128 rejection,
narrowing only when both bounds fit the pre-promotion source domain (the
`[250,260]`-on-u8 low-byte alias class), full-domain detection before the
span-cap rejection, canonical `build_cfg` predecessors, and one-rewrite-per-
call with analysis restarts are all as documented and unit-pinned.

### 6. Kernel tooling scripts — previously adjudicated, unchanged

F2/F5/F6 (migration clean, simultaneous CC+LD invalidation, path
canonicalization) were fixed and documented in the first-round adjudication
(`FOLLOWUP-2026-09-20-ci-review-audit.md`, part of this PR). Re-verified
that the fixes are present in the current tree.

### 7. CI/local gate parity — CONFIRMED missing gate + semantics upgraded

The hosted failure was this branch's own parity checker working as designed:
PR #567 added `check_vector_copy_elimination.sh` to ci_local.sh, and the
pushed PR head did not carry it in any hosted workflow. The gate is now a
first-class hosted step (it is a three-part battery, not a compact grep
contract). The checker itself was upgraded from string-presence to
**execution semantics**: PyYAML parsing of the workflows, only `run:`
script bodies counted, shell comments stripped — a gate commented out of
its own run block now fails parity (verified both ways). Known residual,
documented in the checker: a quoted `echo` of a gate path inside a run
block still matches; catching that requires full shell parsing and is
sabotage-shaped rather than drift-shaped.

## Validation (final tree)

* Regression suite: 763 passed; 3 failures are the documented pre-existing
  environmental i686 multilib link failures; 20 skipped (boot gates etc.).
* ci_local --fast: 53 passed, 0 failed, 4 skipped.
* Benchmark output oracle: 204/204.
* cargo test: 3036 passed, 0 failed.
* i686 asm differential: 21/21.
* Gate parity: 47 commands, execution semantics.
* rustfmt clean; clippy (lib/bins/tests, -D warnings) clean.
* Fresh-clone tree-exactness: base `b9b2d7bb0` + ms178-1.patch reproduces
  the pr568 tree SHA exactly.

## Follow-ups recorded (not this PR)

1. The x86-64 asm-diff corpus (not a CI gate in either mirror) fails
   276/818 cases on assembler directive support (`.endif` without `.if`
   nesting et al.). Pre-existing; a full directive-state-machine effort,
   deliberately out of scope here.
2. Parity checker residual: quoted `echo` of a gate path inside a run block
   still satisfies parity (full shell parsing required to close).
3. The i686 branchy-select lowering emits `.Lsel_true/.Lsel_end` branchy
   selects where x86-64 uses cmov — the -Os size trade-off discussion in
   the pipeline applies; a tuning question, not correctness.
