# Follow-up: register-copy folding perfected, and MachInst put under test

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `91ecf5b2`
Snapshots: `S18`, `S19` (see `artifacts/SNAPSHOT_LEDGER.md`)

---

## 1. Accomplished

### 1.1 `narrow_copy_fold` rebuilt — 12x more effective

The previous session built this pass, measured it at **10 instructions removed
across 220 files (0.03%)**, and deleted it as not worth its complexity. That
was the right call for *that* implementation. It was the wrong conclusion about
the opportunity.

The rebuild started from data rather than intuition — a census of what actually
survives to final assembly at `-O2` across the 559-test corpus:

| form | count |
|---|---|
| `movq` cross-family | 7 943 |
| `movl` cross-family | 2 463 |
| `movl %X, %X` self-move | 261 |
| `movq %X, %X` self-move | 3 |
| `movw` | 1 |

**11 055 copies** survive both copy propagation and dead-write elimination. The
old pass reached almost none of them for one reason: it handled only 32-bit
copies and refused every memory operand.

Two transforms now:

**A. Self-move elimination.** `movq %rax, %rax` is a pure no-op and three of
them reached final assembly. `movb`/`movw` self-moves likewise. `movl %ebx,
%ebx` is *not* a no-op — it zero-extends into `%rbx`, and is the idiomatic
64→32 truncation — so it is removed only where the upper half is provably zero,
with the fact dropped at labels and calls.

**B. Copy folding at all four widths.** The copy's width bounds which uses may
be rewritten, and this is where miscompiles live:

| copy | `%D` shares with `%S` | rewritable uses |
|---|---|---|
| `movq` | all 64 bits | any width, **including address operands** |
| `movl` | low 32; bits 32..63 forced to 0 | ≤ 32 bits |
| `movw` | low 16; bits 16..63 **stale** | ≤ 16 bits |
| `movb` | low 8; bits 8..63 **stale** | ≤ 8 bits |

The `movq`/address-operand row is the whole difference. An address is read as
64 bits, so it is rewritable under `movq` and nothing narrower — and `movq` is
72% of the surviving copies.

**Result: 292 instructions removed across 559 files, 104 files improved
(0.35%)** — 12x the old reach — plus `memchr` −3.3% at runtime, no regressions.

Legality rests on `FileLiveness` plus rules for source clobber, destination
redefinition, read-modify-write of the destination, barriers, implicit-register
instructions, variable shift counts (pinned to `%cl` even when spelled out —
renaming produced `shrq %r9b, %rsi`, which GAS rejects), high-byte aliases, and
frame registers. All matching is boundary-aware: `%si` is a prefix of `%sil`
and `%r8` of `%r8d`, and a naive `replace` produced `%dxl`.

27 tests, 15 of them negative controls, including a catch-all asserting that no
output line names a register the assembler does not know.

**Two draft tests turned out to be wrong rather than the pass**, and are
documented as such in place: one placed a `movl $7, %eax` intending to clobber
the source, but `%eax` was overwritten before any read, so it was a genuine
dead store that was legitimately removed; the other expected a `%rsp` copy to
survive when copy propagation correctly collapses it without renaming anything
away.

### 1.2 MachInst had 946 lines and zero tests

That is how a silent `_ => "rax"` fallback survived in the **64-bit** register
table while the 32/16/8-bit tables all trap. An unexpected register index
produced a syntactically valid instruction naming the *wrong* register — a
silent miscompile in the most frequently used of the four tables, invisible to
every downstream check because the assembler is perfectly happy with it.

`machinst_tests.rs` adds 25 tests in four layers:

1. **Table integrity** — the four name tables must agree and be injective.
2. **Type mapping** — `OpSize` must classify every `IrType`; suffixes must match.
3. **Golden emission** — each variant and operand shape emits the expected text.
4. **Assembler differential** — every constructible instruction is fed to the
   *real* system assembler.

Layer 4 is what makes the rest hard to fool. A golden test only proves the
emitter matches what the test author expected; `as` proves the text is a valid
x86-64 instruction. It skips **loudly** when no assembler is present rather
than passing vacuously.

It immediately found two real bugs:

* **`shift_mnemonic` folded S8 and S16 into the 64-bit mnemonic.** The table
  read `(Shl, S32) => "shll", (Shl, _) => "shlq"`, so a byte shift emitted
  `shlq %dl` and a word shift `shlq %cx` — both rejected outright by GAS
  (``%dl' not allowed with `shlq'``). Latent only because instruction selection
  does not yet route narrow shifts through MachInst; the moment it did, the
  build would break. All twelve `(op, width)` pairs are now spelled out.
* **`MachInst::Alu` and `Imul3` never staged wide immediates.** x86 ALU
  immediates are sign-extended 32-bit, so `addq $0x1234_5678_9abc, %rax` is
  `operand type mismatch` — verified directly against GAS, so a hard build
  failure rather than a silent truncation, but a real defect. Both now stage
  through the scratch, and `Imul3` falls back to the two-address form because
  there is no three-operand register `imul`.

Also hardened: `reg_name_pub` stringified an unallocated vreg as
`VREG_UNRESOLVED` (with an unused binding), blaming the assembler for a
register-allocation bug; it now fails where the invariant actually broke. And
`assert_scratch_free` makes the `%rax`-as-scratch assumption inside
`materialize_large_imm` checkable rather than implicit — if a wide immediate
ever has to be staged while `%rax` is already an operand, staging would clobber
it silently.

Removing the `_ => "rax"` fallback with the whole corpus still green is the
evidence that it was masking nothing.

### 1.3 Two neighbouring tests corrected, not weakened

`copy_coalesce::copy_is_kept_when_the_source_is_read_again` and
`relay_and_lea::relay_is_kept_when_target_is_read_again` began failing. Both
asserted exact whole-pipeline text that `copy_fold` now legitimately improves —
three instructions instead of four, semantically identical, with the copy's
destination provably dead. Rather than disable them or loosen them to
meaninglessness, each was rewritten to assert the invariant it actually guards:
that coalescing does not rename a source family that is read again, and that
both consumers of a relayed value read one and the same register.

### 1.4 Oracle standing

Instruction counts at `-O3 -march=x86-64-v2`, against all four oracles:

| kernel | lccc | clang 23.1 | icc 2021.10 | icx | gcc 16.2 |
|---|---|---|---|---|---|
| `scan` (memchr) | **19** | 10 | 22 | 27 | 149 (vectorised) |
| `match_len` | **15** | 14 | 11 | 31 | 243 (vectorised) |

lccc now **beats ICC and ICX on `scan`** and **beats ICX on `match_len`**, and
sits within one instruction of Clang on `match_len`. Where GCC's counts explode
it is because it is vectorising — and on `memchr` runtime lccc is at **1.01x**,
i.e. scalar lccc already matches GCC's vectorised code on this host.

---

## 2. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1465 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=559 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3354 configs |
| `bench_kernels.py --baseline` | memchr −3.3%; no regressions |
| MachInst assembler differential | all probed instructions accepted by GAS |

---

## 3. To do next, in priority order

### 3.1 `iv_widen` declines when the induction variable escapes

Diagnosed precisely this session. `match_len`'s inner loop still carries
`movslq %ebx, %rbx` **on the loop-carried dependency path** — the exact pattern
that was worth −35% when removed from the gzip compare loop (S08).

The cause is not the loop shape. A two-way probe settles it:

```c
int a(const unsigned char *x, int max) { int s=0; for (int n=0;n<max;n++) s+=x[n]; return s; }   // WIDENS
int b(const unsigned char *x, const unsigned char *y, int max) {
    int n=0; while (n<max && x[n]==y[n]) n++; return n; }                                        // DECLINES
```

`iv_widen` requires the IV's only 64-bit-context use to be addressing. In `b`
the IV also *escapes* — it is the return value, and it feeds the 32-bit trip
compare. Widening it therefore needs the loop-invariant bound sign-extended in
the preheader and a truncation at each exit. That is a real extension, and
given the S08 measurement it is the highest-value remaining scalar work.

### 3.2 `namechars`: the counting form of the classifier

Unchanged and still the worst kernel (0.45x). `if (pred) n++` does not produce
`Select`s — the hit edges converge on a shared increment block before the join
— so `range_fold` and `set_membership` never see it, while the
boolean-returning `k_classify` gets the full treatment. The two kernels
deliberately bracket this.

### 3.3 Vectorisation

GCC's 149- and 243-instruction outputs are SIMD. lccc's scalar `memchr` already
ties it on this host, but that will not hold on wider data. This is the
remaining order-of-magnitude item, not more scalar peepholes.

### 3.4 Extend the differential test to instruction *encoding*

The MachInst differential proves GAS accepts the text. The stronger property is
that the bytes match what GAS would produce for the intended instruction —
`insndiff.py`/`encdiff.py` already exist in `scripts/` for this and could be
pointed at the MachInst corpus.

### 3.5 Def-dominates-use in the IR verifier; linker oracle

Both unchanged from previous sessions.

---

## 4. Notes for whoever picks this up

- **Census before you build.** The copy-fold rebuild worked because it started
  from 11 055 measured surviving copies and a breakdown by form, which pointed
  straight at the missing capability (64-bit address operands, 72% of the
  pool). The first attempt guessed and reached 0.03%.
- **A deleted pass is not a closed question.** The old measurement was correct
  and the conclusion — for that implementation — was right. What was wrong was
  treating it as a verdict on the opportunity.
- **Differential-test the layer, not your expectation of it.** Two real bugs in
  MachInst had survived because there were no tests at all, and both are of the
  kind a golden test would have missed: the author would have written the same
  wrong mnemonic into the expectation.
- **When a neighbouring test starts failing, first prove which side is wrong.**
  Four tests failed across this session; in every case the pass was right and
  the test was asserting stale pipeline text or a premise that did not hold
  (a "clobber" that was really a dead store). Each was rewritten to assert its
  actual invariant — none were weakened or disabled.
