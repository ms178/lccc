# Red-team audit of the S15 increment (self-audit, 2026-09-12)

Scope: the three changes in `113adf119` — reload-reuse precision in
`passes/dead_writes.rs`, the symmetric web-wide flag in `live_range.rs`, and the
confounder isolation in `check_ra_web_inloop_use.sh`. Method: re-read each hunk as an
adversarial reviewer, enumerate what could make it wrong, and check the enumeration against
execution rather than against my own reasoning.

## 1. Reload-reuse precision

### What I verified rather than assumed

* **Width underestimation is the only way this miscompiles**, so I attacked it first. The
  suffix rule alone charges `vmovdqu %ymm0, 24(%rsp)` 16 bytes when it writes 32; `max`
  with the vector-register width closes that, and two tests pin `%ymm` (32) and `%zmm` (64)
  against a cached slot at 48 and 80 respectively. The estimate is over-broad on purpose:
  a `%xmm` appearing *anywhere* on the line forces ≥16 even when it is only a source.
* **Segment overrides.** `%fs:360(%rsp)` reaches `parse_frame_slot` with `disp_txt`
  = `"%fs:"`, whose `parse::<i64>` fails, so it returns `None` and the scan breaks. Safe by
  accident of parsing rather than by design — I would prefer an explicit `:` rejection, and
  have noted it below as residual risk.
* **String ops.** `movsl` with no operands: `store_destination` finds no space and returns
  `None` → break. `line_writes_memory`'s own `movs`-without-`%` branch still fires first.
* **Calls.** `call *(%rax)` would parse a destination, but `LineKind::Call` breaks the scan
  before `line_writes_memory` is ever consulted. Order matters here and is correct.
* **`xchgl %eax, 24(%rsp)`** — a read-modify-write of a *different* slot. Treated as
  non-aliasing, which is right: it writes 4 bytes at 24 and writes `%eax`, and the register
  half is handled by the separate destination-invalidation check.
* **All 34 differing TUs were executed**, not just diffed: identical exit status and stdout
  under both compilers. This pass has a documented miscompile history
  (`pic_indexed_store_static_global.c`), so asm-diff parity was not sufficient evidence.

### Where I think the criticism is fair

**(a) It is runtime-neutral, and I am shipping it anyway.** `+0.26 %` on `sha256_transform`
and `−0.27 %` on `fannkuch`+`linux_rbtree`, both inside the 1 % threshold. My justification
is that each rewrite replaces a load with a register move or deletes it, so it cannot be
slower *locally*, and 143 fewer frame-slot loads is a real reduction in memory traffic
(−2.0 %) that the timing harness cannot resolve on this corpus. A stricter reviewer would
say: an unmeasured benefit does not pay for touching a pass with a miscompile history. I
disagree, but only because the risk is bounded by 11 barrier tests plus executed-TU parity,
and because the alternative — leaving a *provably* over-conservative rule in place because
its cost is currently unmeasurable — is how such rules become permanent.

**(b) I overclaimed in the evidence README.** I wrote that the precision is "the enabler"
for the displacement-propagation increment. Re-deriving it: after propagation the epilogue
groups become `movq 360(%rsp),%rax; movl 4(%rax),%eax; addl 24(%rsp),%eax; movl %eax,4(%rax)`,
and the reloads are *still* separated by an indirect store, which this precision explicitly
refuses to reason about. So it does not enable that increment. Corrected here rather than
left in the record.

**(c) Same-destination deletion is the weaker half.** It accounts for a small minority of
the 100 removed instructions (the frame-slot disjointness rule does most of the work), and
its value is concentrated in exactly the shape that the aliasing rule then blocks. It is
sound and free, but if I had to drop one half to reduce surface, this is the one I would
drop.

**(d) Residual risk I am not closing now.** `parse_frame_slot` should reject a `:` in the
operand explicitly instead of relying on the integer parse failing. Two lines, no behaviour
change, and it removes a coincidence from the soundness argument.

### What I deliberately did not do

Extend to `%esp`/`%ebp`. They map to the same families (4/5) in `scan_register_refs`, so the
argument transfers and the change is two lines. I refused because this sandbox cannot
*execute* i686 binaries (no 32-bit glibc dev headers), and every claim above rests on
execution. An asm-diff-only validation of an aliasing refinement is weaker evidence than I
am willing to put in a compiler. The current state is i686-neutral and CI's i686 gates ran
against an `lccc-i686` built after both edits.

## 2. F3 — symmetric web-wide flag

**The monotonicity argument is the whole justification, so I checked it exhaustively.** For
a range `R`:

* `R` is an owner: old = `own(R) || ∃m∈members(R)`. New = same, plus a redundant term.
  Identical.
* `R` is a member that owns a range, owner `O`: old = `own(R)`. New = `own(R) || web(O)` ⊇
  old. Widened.
* `R` is in no web: old = `own(R)`; new = `own(R)` (map miss, both fallbacks empty).
  Identical.

So the change can only ever widen the flag. That matters more than it looks: **widening this
exact flag is what caused the −40.53 % `lz4_compress` regression** in an earlier round, so a
transform that can only widen deserves scrutiny. It is output-neutral on 805 TUs (0 diffs,
0 instruction delta), and the mechanism that keeps it safe is the decoupling already
upstream — the flag feeds the boolean, never the admission cap's counts.

**Kill-switch fidelity checked by hand, because the gate cannot see it.** With
`web_inloop_use == false`, `member_in_extent` and `web_in_loop_use_of` are both empty, and
the three-term OR reduces to `own(R) > 0 || ∃m∈members(R): uses_in_extents[m] > 0` — which
is exactly the pre-S09 base expression (`in_loop_uses = own + Σ members`, flag `> 0`). The
historical arm is bit-for-bit preserved, not merely similar.

**Fair criticism:** this is churn in a hot RA path for a defect that is unreachable in the
corpus. I accept the churn because the asymmetry is a *reasoning* error — two ranges of one
web disagreeing about a physically shared property is the kind of thing that becomes a real
bug the moment coalescing changes — and because the cost is one extra `FxHashMap` per
function, which I did not measure but which is dominated by the existing three maps. If
compile time ever becomes the binding constraint, this map is the first thing to fold into
`members_of`.

## 3. The gate change — did I weaken a safety net?

This is the change I would most want challenged, so let me state the strongest objection and
answer it.

**Objection:** the gate existed to prove the web-wide supply fires in the *shipping*
configuration. By compiling its structural arms with `CCC_NO_SAME_DST_RELOAD=1
CCC_NO_FRAME_SLOT_ALIASING=1` I made it prove something about a configuration nobody ships.

**Answer:** the structural arms were never about the shipping binary — they are a *proxy*
for "does the supply change allocation in the intended direction", and a proxy is only valid
with its confounders held constant. The alternative readings were both worse: relaxing the
threshold would have destroyed the gate's discriminating power, and deleting the structural
section would have removed the only check that the supply fires at all. The sections that
*are* about shipping behaviour — correctness against gcc on both arms, and the lz4
blast-radius byte-identity — were left on the shipping configuration untouched.

**What the evidence says the gate was right about:** with the precision on, the smallest
`sha256_transform` (192 insns, 47 slot refs) is the arm with the supply **off**, and that
arm is **6.06 % slower** at runtime (15 amplified interleaved reps, low3 1.062). Instruction
count and stack-traffic count both point the wrong way. The gate failing was the system
working; the fix was to give it a valid instrument.

**Residual weakness:** the isolation means the structural numbers (195 vs 208) no longer
describe the shipped binary (194). The comment says so, but a reader skimming the gate
output could still be misled. A cleaner design would print both configurations. Not done:
the gate is already long and the value is marginal.

## 4. Things I got wrong earlier that this round corrected

* I rejected "x86 slot-load dedup" as worthless after implementing it in the *wrong pass*.
  The capability already existed in `reuse_redundant_loads`; my version duplicated it and
  therefore measured 2 instructions. The correct question was never "does x86 lack this
  pass" but "why does the existing pass stop early", and `CCC_PEEPHOLE_TRACE` answered it in
  one run. Lesson recorded: trace the pipeline before writing a pass.
* I reported clang as having "0 in-loop slot refs" using a span detector whose caveat I had
  already documented. The honest whole-function numbers are clang 20 / gcc 22 / lccc 53.
  The conclusion (2.4× the stack traffic) survives; the "0" did not, and the evidence README
  now carries the whole-function figures.

## 5. Verdict

Ship. The precision is sound by a width argument that is deliberately over-broad, validated
by execution on every TU it touches, and honest about being runtime-neutral. F3 is monotone
by construction and preserves the historical arm bit-for-bit. The gate was repaired with a
valid instrument rather than a looser one. The oracle gap is **not** closed — lccc 194
instructions and 52 frame-slot references against clang's 126 and 20 — and the next
increment (displacement propagation through destructive `leaq D(%B), %B`) is specified with
its mechanism and its liveness prerequisites in `TASK-RA-06A`.
