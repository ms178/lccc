# Red-team audit: the zero-extended compare fold, and the round that produced it

An adversarial pass over my own work, written to be useful to a reviewer who
disagrees with every conclusion here. Where I agree with my earlier decision I
say why; where I do not, I say so plainly. The uncomfortable finding is
section 1, and it is about the round, not about a line of code.

---

## 1. The worst thing about this round is a measurement I did not take

**I shipped a peephole for an entire round without ever measuring it.**

The zero-extended compare fold was recorded as an optimisation — snapshot
`S10-zero-ext-cmp-peephole-and-lz4-iv-spill-rootcause`, and it was the
headline of the round that produced it. It was never once run against its own
kill switch. When I finally measured it this round, with a paired, interleaved,
order-alternating harness:

```
lz4_compress   ON  4.094 ms   OFF  4.092 ms   -0.96%   IQR spread 10.7%
zstd_count     ON 22.180 ms   OFF 21.279 ms   -2.80%   IQR spread 23.2%
```

**No measurable effect, and one delta pointing the wrong way.** The fold removes
one instruction from a 285- and a 117-instruction function, in workloads of 4 ms
and 22 ms where process launch dominates. Any sane prior said this; I just did
not apply it.

*Do I agree with the decision to keep it?* Yes, but on narrower grounds than
the round claimed, and I should not have let the round's framing stand:

- It is **correct**: the flag-for-flag argument in its own comment is right, the
  gate verifies it against gcc over all 65536 byte pairs, and the reject side
  holds on SF/OF readers and past a safe reader.
- It **matches every oracle**: GCC, Clang 23.1 and ICX all emit the
  memory-operand compare. It is a fidelity fix.
- It **never grows code** and it fires on 18 of 882 programs, so it is not dead.
- It is **not a speedup**, and the round's framing was wrong to imply otherwise.

The cost of the error was not just a wrong claim. It was opportunity cost: I put
a peephole on a 1-instruction delta while **the same benchmark file contains the
single worst result in the entire 41-program suite.**

| benchmark          | lccc/gcc |
|--------------------|---------:|
| `lz4_match_extend` | **1.898**|

and I attributed that to, in effect, the compare. It is not the compare. See
section 3.

**The generalisable failure.** A micro-optimisation is the easiest possible
thing to work on, and it is where a session goes when it has not yet established
which micro-optimisation matters. The correct order is: measure the whole suite,
find the worst ratio, root-cause *that*, and only then decide whether a peephole
is even the right layer. I inverted that. Next time I will read the benchmark
table before writing a single line of peephole.

---

## 2. The switch-sharing defect: I agree it was a real bug, and I should have found it by inspection

`fuse_zero_ext_cmp` and `fuse_load_into_alu` both sat under
`CCC_PEEPHOLE_SKIP=load_alu_fuse`. Any A/B using that variable measured the
pair. A differential I took under it attributed **21** firing programs to the
fold; splitting the switch gives **18**, and `linux_find_bit` — the largest
single diff at 4 instructions → 2 — belonged to the sibling all along.

This is the most defensible item in the round. One transform, one switch, is
obviously right, and the failure mode it creates is one I have now seen bite:
*two optimizations share a kill switch, so an A/B attributes the wrong one's
effect to the right one's name.* Nothing about this required measurement to
notice. It is a two-line diff visible from across the room, and I shipped the
other one first.

*Do I agree with the fix?* Unqualifiedly. The new switch is
`CCC_PEEPHOLE_SKIP=zero_ext_cmp`, and the regression gate asserts the switch has
a non-empty, code-shrinking effect — so if the switch ever stops gating the fold,
the gate fails instead of silently comparing two identical builds.

---

## 3. The thing I got wrong by a factor of two: I optimised the wrong loop in lz4

`lz4_match_extend` is 1.898x slower than GCC — the worst result in the suite,
and **1.5x worse than nbody**, which is the target I spent the previous two
rounds on. I did not have that number until this round's full run.

Root cause, from the two inner loops:

```asm
; GCC — 7 instructions, 0 stack operations
.L12:  addq $1, %rbx ; addq $1, %r9 ; cmpq %r13, %rbx ; je .L43
.L10:  movzbl (%r9), %edi ; cmpb %dil, (%rbx) ; je .L12

; lccc — 7 instructions, 3 stack operations
.LBB14: movq 112(%rsp), %rcx ; movzbl (%rcx), %r8d ; cmpb (%rbx), %r8b ; jne .LBB16
.LBB15: addq $1, %rcx ; movq %rcx, 112(%rsp) ; addq $1, %rbx ; cmpq 56(%rsp), %rcx ; jb .LBB14
```

Same instruction count. The difference is that lccc's induction variable `p` is
**loop-carried through memory**, so every iteration serialises on store-to-load
forwarding, ~4-5 cycles. In a loop that is two loads, a compare and two
increments, that is the entire runtime.

The decisive detail: lccc's prologue pushes **all six** callee-saved registers
and `main` uses 8-34 references to each, yet the hot loop allocates `p` to
`%rcx` — caller-saved — and gives it a stack home while five already-paid-for
callee-saved registers sit idle. This is a **callee-saved allocation failure**,
and it is worth more than everything the peephole work delivered.

*Do I agree with this analysis?* Yes, with one caveat I want on the record: the
instruction counts and the callee-saved availability are **direct observation**
and are certain. The attribution to a *spill-cost model* rather than, say,
allocation ordering is an **inference** — it is the most economical explanation
of the evidence, but distinguishing it from alternatives needs the allocator's
own decision trace, which I have not read. I am recording it as a strong
hypothesis with a specific acceptance test, not as a diagnosis.

---

## 4. The gate: I disagree with my first version of it, and I rewrote it

My initial `check_zero_ext_cmp_fold.sh` included an "accept" kernel that the
fold declined, and the fold was **right to decline**. The kernel was two `u8`
loads compared in a small function; register allocation if-converted it to a
`cmov` and phi-copied the loaded value before the peephole ran.

*Do I agree with the first version?* No, and it is worth being precise about
why, because the failure was not a bug in the gate — it was a category error.
The gate was measuring **register allocation** and reporting the result as
"feature absent." A synthetic kernel is only a valid witness if the transform
provably sees the shape you intended, and I had not checked that.

The rewrite pins the shape on `zstd_count` and `lz4_compress`, the two programs
where the fold measurably fires, and asserts the memory-operand compare is
present with the fold on and absent with it off. Same for the switch: a
differential that must be **non-empty** and **shrinking**, so a broken skip
variable is a failure rather than a vacuous pass.

*Is the new gate non-vacuous?* Verified, not assumed: disabling the fold makes
it report 4 failures, and restoring it makes it pass. A gate that has never been
observed to go red is a gate of unknown value — the same defect that made the
earlier peel gate worthless.

---

## 5. The peel: I still agree it must stay reverted

-0.11% paired, faster in 13 of 31 rounds, against a noise floor calibrated at
±3%. It changed exactly one binary in the suite, and made it 40% larger. Two
independent measurements now corroborate: unrolling nbody's outer loop grows the
body 293 → 1048 instructions and costs 48% wall time, purely in fetch
footprint.

*Do I agree with reverting?* Yes, without reservation. And I want to note the
*reason* I got there, because it generalises: the peel's own diagnostic —
running it inside Pass A's fixpoint preempted Pass A's outer-first cascade —
showed a 47% regression on `struct_copy` that a per-target A/B would never have
revealed. A null result on the target was concealing a large loss elsewhere.

---

## 6. What I am still not claiming

- **I am not claiming a speedup.** The honest summary of this round's shipped
  change is: one correct peephole, correctly scoped as codegen fidelity, worth
  0% measured; one real bug fixed (switch sharing); one real gap closed (a gate
  that did not exist); and one much larger performance defect found and
  documented but not fixed.
- **I am not claiming the SLP work is close.** `adj.c`: lccc 15 instructions,
  Clang 23.1 12, ICX 11. The missing piece is a grouping rule for two adjacent
  scalar F64 loads, and the difficulty is aliasing — a 128-bit access where the
  program performs two 64-bit ones can fault across a page boundary and can
  clobber an adjacent field. I have the exact target instruction sequences from
  the oracle. I do not have a safe implementation, and I am not going to ship a
  guessed one. A rewrite inferred from instruction counts is how compilers get
  miscompiled.
- **I am not claiming the benchmark deltas outside the noise floor.** The suite
  run has per-benchmark CVs from 0.8% to 25%; `nbody` at 17.3% and
  `lz4_match_extend` at 5.2% are the ones I lean on, and both are large enough
  to survive their own spread.
- **I am not claiming CI equivalence.** `ci_local.sh --fast` is green — 120
  passed, 0 failed, 5 skipped — and that is the first time this work has reached
  `ALL GATES GREEN` rather than `PARTIAL-NOT-DELIVERABLE`. But `--fast` is
  stamped "NOT CI-equivalent and not delivery-grade until `--slow` also passes",
  and `--slow` was not run this round, per the standing instruction to keep to
  fast gates.

---

## 7. If a senior reviewer asked me "what should you have done differently?"

Answer, in order of how much it would have mattered:

1. **Read the benchmark table before choosing a target.** The worst ratio in the
   suite was 1.5x larger than the one I picked, and it sat in the same file as
   the peephole I was shipping.
2. **Measure every optimization before recording it as one.** One line of
   harness against a 4 ms workload would have told me the fold was worth
   nothing and freed the round for the thing that was worth 1.9x.
3. **Make every transform independently killable from the day it is written.**
   The shared switch was invisible to me for a full round and silently
   corrupted an attribution.
4. **Prove every new gate goes red.** Both times I wrote a gate this session it
   was wrong first: the peel's was vacuous, the fold's measured the allocator.
   Non-vacuity is a test result, not a property one hopes a gate has.

Every one of those is cheap. None of them happened. The correction is procedural
rather than technical, which is the least satisfying kind of answer and probably
the most useful one.
