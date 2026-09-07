# Follow-up: loop rotation Guard G — profitability, not correctness

Session date: 2026-09-07 (second session this day)
Base: `2d1db599efed88fbc942021ae332a584e3d7265d` — upstream `main`, re-verified with
`git fetch origin main`. Upstream had absorbed the previous session's work as **PR #436** and added
**PR #437** ("Harden optimizer memory barriers and x86 allocation safety"). The previous
`ms178-1.patch` reverse-applies cleanly against this main, i.e. it is fully contained upstream; this
session's series starts from zero on top of it.
Snapshot head: `e5d963cc9f3ce1b136cb2d83ca9cf9a7d0ec8925` (`S05-loop-rotate-guard-g`)
Deliverable: `/home/user/ms178-1.patch` — 95101 bytes,
sha256 `2816380a8a46e31f4bf4ff838d5c781d7516be85280515b67be3b9159270dea0`, APPLIES-CLEAN.

---

## 1. The question, and why the recorded answer was wrong

`engineering/STATE.md` lists "Loop Rotation Default-Enable (PF-17)" as a top-4 backlog item, with
the rationale "eliminating the extra entry jump in hot loops". `src/passes/loop_rotate.rs` records
that v16 default-enabled the pass, hit 16 miscompiles, and was reverted to opt-in — implying the
blocker was correctness.

Measured first, before touching the pass (`scripts/perf_ab.py --env CCC_LOOP_ROTATE=1 --opt=-O2`,
5 interleaved AB/BA reps, min-of-5, 34 benchmarks above the startup floor):

```
Aggregate geomean B/A = 0.9980   VERDICT: NO MEASURABLE DIFFERENCE
  A faster on: arith_loop (+24.9%), tls_seg_access (+13.6%), lz4_compress (+1.7%), sieve (+1.6%)
  A slower on: sha256_transform (-27.1%), zstd_count (-6.4%), nbody (-3.4%),
               gzip_crc32 (-3.3%), glibc_strstr (-2.4%)
```

The aggregate is a wash **because a 27 % win and a 25 % loss cancel**. So neither "default-enable
it" nor "leave it off" is right, and correctness was never the issue on this revision (see §4).
The pass needed a cost model.

## 2. Root cause of the regression

`arith_loop` is 32 loop-carried ints, all 32 live out. Stepwise, all reproduced:

1. **Assembly.** Identical arithmetic in both arms (32 `imul`, 35 `add`) but `mov` 89 → 169 and
   stack references 107 → 164. Hot loop body **113 → 149 instructions, 22 → 47 spill/reloads** —
   the cost is *inside the loop*, not on the entry/exit edges.
2. **IR, per pass** (`CCC_DUMP_EACH_PASS=1`). `loop_rotate` itself takes the header phi count
   **33 → 98** (final 65 vs 33 unrotated) with `BinOp`/`Cmp` counts unchanged. The pass never
   constructs a `Copy`; `sccp` materialises 33 and `dce` removes them. So the extra work is
   structural, and the allocator pays for it later.
3. **Mechanism.** Rotation makes the exit reachable from two edges (0-trip guard and latch)
   instead of one, so each loop-carried value live out of the loop needs an exit phi merging its
   init with the body's last definition — which keeps that definition live from its definition to
   the exit. One extra loop-spanning live range per live-out value.
4. **Decomposition.** A variant with the same body but a single live-out value still costs
   +9 stack moves (22 → 31) against +25 for 32 live-outs (22 → 47). So the cost is a
   per-live-out-value term plus a small constant.

## 3. Guard G

Bail when the live-out loop-carried count exceeds the GPRs the target can spare after ABI
reservations: x86-64 12, i386 4, AArch64 27, RISC-V 28, unknown targets fail closed at 4.
`live_out_loop_carried_count` counts terminator operands as well as instruction operands, because
a live-out IV is normally consumed by an outside block's `CondBranch`; ignoring terminators would
under-count exactly the IV-heavy shapes the guard exists for.
`CCC_LOOP_ROTATE_IGNORE_PRESSURE=1` disables the guard for A/B work.

**Result, same harness, Guard G active:**

```
Aggregate geomean B/A = 0.9917   (was 0.9980)
  A slower on: sha256_transform (-27.0%), binary_trees (-6.5%), matmul (-3.7%),
               gzip_crc32 (-3.4%), arith_loop (-3.3%)
  A faster on: tls_seg_access (+13.2%), double_reduction (+5.4%), loop_patterns (+2.7%)
```

* The 27 % `sha256_transform` win survives intact.
* The 24.9 % `arith_loop` loss is **eliminated** — it now appears on the *faster* side at 3.3 %.
  Its hot loop is byte-identical to rotation-off (113 insns / 22 stack moves).
* Aggregate improves 0.9980 → 0.9917, i.e. the gate is worth ~0.6 pp on the corpus.

## 4. The 15 miscompiles are gone

`CCC_LOOP_ROTATE=1` runs `scripts/run_regression_suite.sh` at **PASS=651 FAIL=0, 0 AB-diff
failures** — and so does `CCC_LOOP_ROTATE=1 CCC_LOOP_ROTATE_IGNORE_PRESSURE=1`, i.e. with the gate
off and every high-pressure loop rotated. The v16 list in the pass header is a historical record,
not a current blocker; PR #437's allocation and barrier hardening is the likely reason. The header
now says so, with the measurement attached.

**Not yet done, and the reason the default is still opt-in:** the regression suite is a fixed
corpus, not a generator. `scripts/fuzz_diff.py` and `scripts/csmith_diff.py` exist and have not
been swept on this revision with rotation enabled. That sweep is the remaining gate for a default
flip.

## 5. Tests added

* `tests/regression/loop_rotate_pressure_gate.c` + `.env` — runs with **both** knobs set (pass on,
  gate off), the strictest configuration, so the 32-live-out loop really is rotated and its
  exit-phi machinery is exercised. When Guard G fires the loop is simply untouched, which the rest
  of the suite already covers. The reference is the same Gauss-Seidel recurrence as an in-place
  array sweep, sharing no code shape with the code under test; note the recurrence is
  Gauss-Seidel, not Jacobi (`v[30] += v[31]*v[0]` reads the already-updated `v[0]`), which is what
  makes a parallel-update reference disagree.
* `tests/regression/check_loop_rotate_pressure.sh` — pins the *decision*, which is compile-time and
  invisible to a run-and-compare test. Three assertions, because the first alone would also pass if
  the loop never rotated for an unrelated reason: (1) gate on == rotation off byte-for-byte;
  (2) gate off changes the code (102 → 172 insns, **25 → 64 stack moves**), so (1) is attributable
  to Guard G; (3) a loop under budget still rotates (14 → 22 insns), so the gate is not a blanket
  disable. Each shape gets its own TU — sharing one let `low_pressure`'s rotation perturb
  `high_pressure`'s extracted text and produced a false failure, which is how the coupling was
  found.

Gates: `cargo fmt --check` clean · `clippy --all-targets -D warnings` clean ·
`cargo test --all-targets` **2022 / 0 / 6 ignored** · regression suite **PASS=651 FAIL=0,
AB-diff 0** in all three rotation configurations · `check_loop_rotate_pressure.sh` PASS.

## 5b. The headline defect: the rotated loop is not bottom-tested

Found while chasing the residual `tls_seg_access` +13.2 %. Emitted block structure of `tls_pass`
(`-O2`, x86-64):

```
rotation OFF                          rotation ON
  .LBB1:  cmpq $64, %r9   (guard)       .LBB1:  cmpq $64, %r11  + jcc   <- test HERE
  .LBB2:  ...body...                    .LBB2:  ...body...
          cmpq $64, %r9   + jcc                 jmp .LBB1               <- extra jump
  .LBB3:  ret                           .LBB3:  ret
```

The unrotated loop is **bottom-tested**: one conditional branch per iteration, as the loop's last
instruction. The rotated loop is **top-tested**: the test sits in the header block and the body
block ends in an unconditional `jmp` back to it — **two branches per iteration**.

That is not what loop rotation is for. The defining benefit of rotation is precisely to move the
test to the bottom so the steady-state iteration pays one branch instead of two, and to hoist the
0-trip check out of the loop. This implementation clones the compare into the latch at the IR
level (the pass header documents `cmp'` there) but the emitted code still branches from the header
and jumps back from the body, so the benefit never materialises while the costs — an extra
per-iteration jump, plus phi copies — are paid in full.

Two further costs visible in the same hot loop (10 instructions unrotated, 13 rotated):

```
OFF:  addq %r15, %r8        ; acc += x, in place
ON:   leaq (%r8,%r14,1),%r15 ; t = acc + x      <- extra LEA
      movq %r15, %r8         ; acc = t          <- extra mov
      movq %r11, %rax        ; IV copy, unused in the loop
```

So the rotated loop-carried accumulator cannot be updated in place — the same coalescing failure
class as `arith_loop` §2, at smaller scale — and there is an apparently dead IV copy feeding the
exit phi.

**Consequence.** This, not register pressure, is why the corpus aggregate is a wash: the pass
removes the entry jump but adds a per-iteration jump, so on most shapes the two cancel and only
loops whose win comes from somewhere else (`sha256_transform`, −27 %) show a gain. Guard G (§3)
removes the pressure-driven losses and is worth keeping, but it is a mitigation. **R3 below should
be re-scoped: the first thing to fix is the bottom-test, because until the steady-state iteration
has one branch instead of two, rotation cannot be a net win in general.**

### IR-level confirmation and the precise open question

The same defect is visible in the final IR (`CCC_DUMP_IR=1`) for a minimal loop
`for (int i = 0; i < n; i++) s += i * 3;`:

```
rotation OFF (4 blocks)                rotation ON (7 blocks)
  b0: init; Branch(1)                    b0: Cmp(0<n); CondBranch(1,2)   <- 0-trip guard
  b1: Cmp; CondBranch(2,3)               b1: s=0; i=0; Branch(3)
  b2: Mul; Add; i+1; Copy s; Copy i;     b2: v21=0; Branch(6)            <- 0-trip exit
      Cmp'; CondBranch(2,3)  <== SELF-   b3: Mul; Add; i+1; Cmp';
      LOOP, bottom-tested, 1 branch          CondBranch(4,5)
  b3: Return(s)                          b4: Copy s; Copy i; Branch(3)   <== COPY-ONLY LATCH
                                         b5: v21=sum; Branch(6)
                                         b6: Return(v21)
```

Two facts this settles:

1. **The unrotated loop is already optimal.** Existing passes collapse it to a single
   bottom-tested self-loop (`b2`), one conditional branch per iteration. There is no entry jump to
   remove — so on this shape rotation has no benefit available at all, only costs.
2. **Rotation introduces a copy-only latch block** (`b4`: two `Copy`s and an unconditional
   `Branch(b3)`). The steady-state cycle is `b3 -> b4 -> b3`, i.e. two branches per iteration.
   The cloned compare *is* at the bottom of `b3` as the pass intends; the extra branch comes from
   the separate latch block holding the backedge phi copies.

So the fix is not "clone the compare into the latch" — that already happens. It is to **absorb the
copy-only latch into the body block**, so the cycle becomes a self-loop again:

```
b3: Mul; Add; i+1; Copy s; Copy i; Cmp'; CondBranch(3, 5)
```

The copies may move above the conditional branch only if their destinations are dead on the exit
edge; in the example they are (`b5` forwards the pre-copy sum, and `s`/`i` have no use after the
loop), so a conservative "no use of any copy destination outside the loop" side condition makes
the rewrite sound. That condition is cheap to check and is exactly the case rotation creates.

**Open question I did not settle, stated plainly rather than guessed:** which pass materialises the
copy-only latch block. It is not `loop_rotate` (the pass inserts *phis* at the top of
`body_latch`, not copies), so it is phi elimination or a later CFG pass splitting the backedge. A
per-pass block-count trace gave inconsistent counts on this input, so I have no reliable answer
and will not assert one. The fix above can be implemented either at the point the split happens or
as a targeted cleanup immediately after rotation; deciding which needs that trace redone with a
trustworthy parser.

Verified by disassembly only; I did not fix it this session. It is a CFG/branch-placement change
with real miscompile risk and needs its own measurement and fuzz campaign.

## 5c. Prerequisite resolved: phi elimination splits the self-loop backedge — and a first fix attempt, reverted

The open question from §5b is answered, with a trustworthy per-pass trace (70 sections; an earlier
"0 sections" result was the source file missing from `/tmp` after a harness wipe, not a parser
bug).

**The middle-end already produces the optimal shape.** Last middle-end IR for
`for (int i = 0; i < n; i++) s += i * 3;` under `CCC_LOOP_ROTATE=1`:

```
block 0: Cmp(0<n); CondBranch(2,4)                       <- 0-trip guard
block 1: Phi v19, Phi v20; Mul; Add; i+1; Cmp'; CondBranch(2,4)   <- SELF-LOOP, bottom-tested
block 2: Phi v21; Return(v21)
```

Three blocks, one branch per iteration. Nothing to improve. **`src/ir/mem2reg/phi_eliminate.rs`
then destroys it**: `place_copies` routes copies through a trampoline whenever
`multi_succ[pred]` holds, and a self-loop block by definition has two successors (itself and the
exit). The result is the 7-block CFG with the copy-only latch from §5b. So the defect is not in
`loop_rotate` at all — rotation merely leaves real phis, whereas the unrotated loop arrives with
its phis already lowered to copies and so never triggers the split.

**The prize, measured with a working prototype.** Hoisting the backedge copies into the block
instead of splitting gives, for the same loop, `CondBranch(true_label: BlockId(3))` — a true
self-loop, trampoline gone. On `tls_seg_access`'s hot loop: **13 -> 11 instructions and
unconditional jumps 1 -> 0** (rotation-off is 10).

**Why it is not in this patch: it miscompiles, so it was reverted.** The prototype gated hoisting on
(a) self-loop, (b) commuting copies (no destination is also a source), (c) every destination used
only inside the block. That regressed `tests/regression/ra09_selfop_xor` (`-O3`):
lccc printed `430e880c96f10300` where GCC prints `226f0a7b8d22d0af`; reverting
`phi_eliminate.rs` alone restores the correct value, so the transform is the cause. Adding a fourth
condition — no destination may be read by the block's own terminator, since hoisted copies land
after every instruction but before the branch — was necessary but **not sufficient**: the test
still failed. There is at least one further unsoundness, so the change was dropped rather than
shipped. The tree in this patch has upstream `phi_eliminate.rs`.

Likely remaining holes, for whoever picks this up (both unchecked):
* `place_copies` is called per (predecessor, target) group, so the "commuting copies" test only
  sees one group. Two groups writing into the same block can still have a destination of one as a
  source of the other.
* `use_blocks` is computed once, before any copy is placed. It does not model the copies already
  appended to the block, nor values materialised by other trampolines.

A correct implementation needs the parallel-copy analysis the pass already has
(`find_conflicting_phis`) applied to the *union* of all copies landing in the block, and a liveness
query that accounts for them. `ra09_selfop_xor` is the reproducer; it fails fast and
deterministically at `-O3`.

## 5d. Self-loop latch absorption: shipped, with the soundness proof it took to get there

§5c recorded a prototype that was reverted for a reproducible miscompile. It is now landed,
correct, and firing. `src/ir/mem2reg/phi_eliminate.rs` gained `classify_placement`,
`BlockUses`, `self_loop_copies_can_hoist` and `resolve_self_loop_copies`.

**The transform.** When the phi's target *is* the predecessor (a backedge into the block holding
the phis), the copies are buffered per block instead of being routed to a trampoline, then
validated as a set. If the set is sound it is appended to the block; otherwise the trampoline is
built exactly as before. The transform is therefore a strict refinement of the old behaviour — it
can only ever remove a block, never add one.

**The four conditions, and why each is necessary.**

1. *A self edge is still present in the terminator* (including a switch arm or default). Without
   it the buffered copies are simply wrong.
2. *No destination is read by any terminator.* For the block's own terminator this is ordering:
   the copies land before it. For every **other** block's terminator it is the escape condition,
   and it has to be checked separately — a value can travel straight from the loop into another
   block's `Return` with no instruction reading it in between. Folding terminator reads into the
   instruction-read map is precisely what made the first attempt miscompile.
3. *No destination is read by any other block's instructions.* Reads inside the block are safe
   because they all precede the appended copies.
4. *Copy non-interference:* a destination may not be read at any instruction after its source's
   definition. This is the ordinary interference rule, and it is the condition that was missing
   from every earlier attempt.

Plus the parallel-copy rules: destinations distinct, and no destination also a source (a swap or
shift chain does not commute and belongs to the existing temporary machinery).

**Why condition 4 is the one that matters.** With 1–3 alone, `ra09_selfop_xor` at `-O3` still
produced `430e880c96f10300` against GCC's `226f0a7b8d22d0af`. The post-phi-elimination IR was
verified correct by hand — copies at the tail, after every read, destinations provably block-local
— so the defect was downstream. The emitted loop tail told the story:

```
before: lea -0x1(%r13),%r11d ; test %r13d,%r13d ; je exit ; mov %r11,%r13
after:  lea -0x1(%r13),%r13d ; mov %r10,0x70(%rsp) ; cmp $0x0,%r13d
```

The copy `v411 = v270` was collapsed into the instruction defining `v270`, so `v411` was
overwritten before the `cmp` that still needed it. A destination read sitting inside its source's
live range is exactly what a move coalescer wants to collapse. Condition 4 removes the opportunity
at the IR level rather than relying on every later pass to decline it — which is the right place
for it, because the invariant is a property of the copy, not of the backend.

**This costs nothing on the shape the transform exists for.** After loop rotation the
next-iteration values are distinct SSA values computed last, so every induction read precedes them
and condition 4 holds by construction.

**Results.**

| Gate | Result |
|---|---|
| `cargo test --all-targets` | 2040 passed / 0 failed / 6 ignored (13 new unit tests) |
| regression suite, default | PASS=653 FAIL=0 SKIP=15, AB-diff 0 |
| regression suite, `CCC_LOOP_ROTATE=1` | PASS=653 FAIL=0, AB-diff 0 |
| regression suite, `+IGNORE_PRESSURE=1` | PASS=653 FAIL=0, AB-diff 0 |
| `ra09_selfop_xor -O3` | `226f0a7b8d22d0af` — matches GCC |
| fmt / clippy `-D warnings` | clean |

`perf_ab.py --env CCC_NO_PHI_SELFLOOP_HOIST=1 --opt=-O2 --reps 5`: geomean **1.0060** in favour of
absorption (+0.60%, inside the 1.0% noise band), with `arith_loop` +9.5%, `zstd_count` +5.0%,
`expat_xml_scan` +4.5%, `fp_memfold_stencil5` +4.0%. On the counted-loop repro the rotated hot loop
loses its unconditional branch entirely: `jge <exit>; jmp <loop>` becomes a single backward `jl`.

### The backend defect was then root-caused and fixed; condition 4 was removed

Condition 4 was a workaround and it is gone. The defect was found by bisecting with
`CCC_PEEPHOLE_SKIP` and `CCC_NO_PEEPHOLE_PHASE1..7`: with **every** peephole disabled the
miscompile persisted, and `CCC_VERIFY_REGALLOC=1` was silent, which localised it to the emitter
rather than to allocation or peepholing. Disabling the phases left the loop tail intact and
readable:

```
mov %r13d,%r11d ; sub $0x1,%r11d ; mov %r11,%r13 ; mov %r10,0x70(%rsp)
mov %r13d,%eax  ; cmp $0x0,%eax  ; jne <loop>
```

The compare is emitted at the *branch*, after the copies overwrote `%r13`. That is
**COMPARE-REPLAY** (`emit_int_cmp_replay_insn`): when a `Cmp`'s single use is a same-block `Select`
or the block's `CondBranch`, the emitter skips the `Cmp` and re-emits it at the consumer. Its
soundness argument is that operands are reloaded from canonical stack slots, and `operand_links`
keeps the allocator from handing the register to a later value.

Neither protects against an **IR-level redefinition of the operand itself**. A phi-elimination
latch copy absorbed into the block legitimately writes the loop-carried value between the `Cmp` and
the branch, so the replay compares the next iteration's value. The old trampoline shape never hit
this because the copies lived in a different block.

`compute_cmp_replay_scan` now refuses to register a `Cmp` whose operand is defined by any
instruction between the `Cmp` and its consumer; such a compare is emitted at its own position
instead. One extra instruction on a shape that was previously a miscompile, and it fixes a latent
bug reachable from any IR with a redefinition in that window -- not just from latch absorption.
Four unit tests in `src/backend/x86/codegen/comparison.rs` pin the guard, including the rhs case
and the case where the redefinition precedes the compare.

With the backend correct, condition 4 was deleted: the two shapes it rejected now absorb, which is
strictly more optimization for no correctness cost. `ra09_selfop_xor -O3` matches GCC with the
absorption unconditional.

### Condition 4 returned as a profit test, not a correctness test

With the emitter fixed, the old copy non-interference condition was no longer needed for
correctness, so it was deleted. Measuring that deletion was instructive:
`perf_ab --env CCC_NO_PHI_SELFLOOP_HOIST=1 --opt=-O2 --reps 5` gave geomean **1.0060** with the
condition and **0.9987** without it. The condition was also protecting branch-compare fusion.

The reason is now explicit. Absorbing appends the copies right before the terminator. If the
block's branch condition is a `Cmp` in that block and a copy redefines one of its operands, the
copy lands between the `Cmp` and its consumer, the new redefinition guard makes compare-replay
decline, and the compare is emitted at its own position — an operand reload every iteration, which
costs more than the removed branch saves.

So condition 4 came back formulated against that mechanism directly rather than against a liveness
proxy: decline only when a copy destination is an operand of the `Cmp` feeding this block's
`CondBranch`. That is strictly narrower than the old rule — it no longer rejects loops whose
induction values are read after the next-iteration values are computed, only loops where the
branch condition itself would be split — and two unit tests pin both sides of it.

### The dead register load, fixed

`for (int i = 0; i < n; i++) s += i * 3;` rotated and absorbed used to emit

```
mov %edx,%r9d ; lea (%r9,%r9,2),%r9 ; add %r9d,%esi ; add $0x1,%edx
mov %rdx,%rax                     <-- dead
cmp %edi,%edx ; jl <loop>
```

Compiling with every peephole phase disabled exposed the origin:

```
add $0x1,%edx ; mov %rdx,%rax ; mov %rdi,%rcx ; cmp %ecx,%eax ; jl <loop>
```

That is `emit_int_cmp_replay_insn` verbatim — it stages *both* operands through `%rax`/`%rcx`
before comparing. A later peephole then rewrote the staged compare back onto the operand homes,
producing the tidy `cmp %edi,%edx` while leaving the `%rax` load behind as dead code inside the
loop. So the visible symptom was a peephole cleanup gap, but the cost was created upstream: three
instructions were emitted where one sufficed.

The staging was only ever needed for operands with no register home. For homed operands the home
is already valid at the consumer: the IS-09 block in `prologue.rs` feeds `operand_links` into the
allocator precisely so a register-homed replay operand's interval extends to the consumer
position, and the redefinition guard added above rejects any `Cmp` whose operand an intervening
instruction rewrites. `emit_int_cmp_replay_insn` now compares in place through
`emit_int_cmp_insn_typed` when both operands have non-XMM GPR homes, and keeps the slot /
accumulator staging only for the operands that genuinely need it.

The rotated loop is now six instructions with no trampoline and no dead move — identical in shape
to the unrotated optimum, which is the point of the whole exercise.

### A/B results

Both changes measured in isolation, `scripts/perf_ab.py --opt=-O2 --reps 5`, 34 benchmarks, each
against a build with only that change disabled by a temporary knob (since removed):

| Change | geomean | Largest wins |
|---|---|---|
| direct-home compare in compare-replay | **1.0051** | `hash_table` +11.3%, `double_reduction` +9.5%, `arith_loop` +1.2%, `nbody` +1.2% |
| self-loop latch absorption | **1.0036** | `double_reduction` +13.7%, `arith_loop` +11.7%, `loop_patterns` +1.1% |

Both aggregates sit inside the harness's 1.0% noise band, which is expected: the transforms only
touch loops with a self-loop backedge, so most of the 34 benchmarks are unaffected and dilute the
geomean. The per-benchmark wins are large and consistent, and neither change regresses a benchmark
by more than the noise floor.

Gates after both changes: 2046 unit tests, regression suite **655/0** with 0 AB-diff failures in
the default, `CCC_LOOP_ROTATE=1` and `IGNORE_PRESSURE` configurations, fmt and clippy clean.
`tests/regression/cmp_replay_direct_home.c` covers the replay shape differentially against GCC.

## 6. Remaining work, ordered by expected value

1. **[R0] Make the rotated loop bottom-tested** (§5b). Until the steady-state iteration pays one
   branch instead of two, rotation cannot be a general win and no cost model will change that.
   Evidence and the exact emitted shapes are in §5b.
2. **[R1] Fuzz sweep for a rotation default flip.** Partially done this session: 600 synthetic
   differential seeds x 4 opt levels, `CCC_LOOP_ROTATE=1`, **600 PASS / 0 FAIL**, with a matched
   rotation-off control also at 600/0 (the generator does emit counted loops, so the sweep is
   meaningful). `csmith`/`yarpgen` engines and `stress_suite --config-env` remain unswept. Given
   R0, the default should stay opt-in regardless. `fuzz_diff.py` / `csmith_diff.py` with
   `CCC_LOOP_ROTATE=1`, then flip the default and re-run the corpus. Guard G makes this worth
   doing: the pass now has a cost model and a 27 % win sitting behind an opt-in flag.
3. **[R2] `tls_seg_access` +13.2 %.** Root-caused to §5b (top-tested loop plus a non-coalesced
   accumulator); it is not pressure-bound, so Guard G correctly does not catch it. Reproduces in both runs (13.6 %, 13.2 %), so it is real and
   Guard G does not catch it — the loop is not register-pressure-bound. Suspect the TLS base
   register interacting with the rotated CFG. Smaller prize than R1 but a genuine remaining loss.
4. **[R3] Make exit phis free.** The deeper fix behind Guard G: if the exit phi's destination were
   coalesced with the body's last definition and the single copy placed on the 0-trip edge, the
   per-live-out-value cost would disappear and Guard G would become a rare safety net rather than
   the thing standing between the pass and `sha256_transform`-class wins on high-pressure loops.
   This lives in phi elimination/coalescing and needs its own measurement campaign.
5. **[R4] ChaCha20 quarter-round SIMD.** `engineering/STATE.md`: LCCC 592 insns vs GCC 232,
   Clang 176, ICX 74. The largest single codegen gap in the matrix.
5. Carried over: real-mode defects D2/D3/D4/D5 and scalar FP under `-mno-sse`
   (`FOLLOWUP-2026-09-07-kernel-boot-c6-os-agg-frame.md` §3), and the kernel compressed-payload
   boot blocker B1 from the same document.

## 7. A published patch shipped junk — cause and permanent fix

The S05/S06 deliverables were 104 KiB for 521 lines of real change. Auditing the published patch
found three classes of junk, none of them intended:

| junk | count | effect if applied |
|---|---|---|
| mode-only hunks (`old mode 100755`) | 76 scripts | strips the exec bit from every script |
| deletions of tracked files | 5 | **removes `src/backend/x86/assembler/encoder/core.rs` and `src/backend/i686/assembler/encoder/core.rs`** |

Both come from the harness wipe interacting with `git add -A`:

1. The restore strips exec bits, so the next `git add -A` stages `100755 -> 100644` tree-wide.
2. **Five tracked files match patterns in this repo's own `.gitignore`** — `core.*` (line 12) covers
   both assembler `core.rs` files, and `test_*` (line 39) covers
   `tests/integration/test_progressive.py`, `tests/test_ivsr_indexed.c` and
   `tests/test_ivsr_indexed_before.s`. Tracked files stay tracked regardless of `.gitignore`, so
   this is invisible in normal use. But once a wipe deletes them, `git add -A` stages the deletion
   *and* ignored paths are omitted from `git status`, so the tree reports **clean**. The loss never
   surfaces until the published patch deletes two backend encoders. It survived two snapshots
   because nothing inspected the diff.

Fixed permanently rather than per-instance: `scripts/lccc-snapshot.sh` now runs
`snapshot_guard_staged_diff` between `git add -A` and the commit, and refuses to publish a diff
that deletes tracked files or changes modes, printing the paths and the restore commands.
`LCCC_SNAPSHOT_ALLOW_STRUCTURAL=1` is the explicit opt-in for a snapshot that genuinely removes or
re-modes files. The guard was negative-tested by staging a real deletion
(`git rm --cached tests/test_ivsr_indexed.c`): it fired and the snapshot aborted.

After rebuilding the series on a pristine upstream tree the deliverable is 5 files / 521
insertions / 0 deletions / 0 mode changes, plus this guard.

Two upstream observations worth a separate look, not fixed here because they are not mine to
silently change:
* `.gitignore`'s `core.*` and `test_*` patterns shadow tracked source and test files. Narrowing
  them (e.g. `/core.*` at the repo root, `tests/test_ivsr_indexed*`) would remove the trap.
* The same patterns mean a *new* test file named `test_*.c` would be silently untracked.

## 8. Harness notes

The workspace was wiped twice during this session, each time removing `.git`, `target/`, swap, the
Rust toolchain and (once) the kernel tree; `/home/user/ms178-1.patch` and `artifacts/` survived
both times. Recovery is: `swapon` the 8 G `/swapfile`, `chmod +x ~/.cargo/bin/*` (**the wipe strips
exec bits — `rustup` itself becomes unrunnable, and `cargo` vanishes from PATH**),
`rustup toolchain install stable`, then `git init && git fetch origin main && git reset --soft
FETCH_HEAD`, which anchors history while preserving the working tree. Verify with
`git diff --cached --summary | grep 'mode change'` before committing: a wipe followed by
`git add -A` stages 77 exec-bit deletions.
