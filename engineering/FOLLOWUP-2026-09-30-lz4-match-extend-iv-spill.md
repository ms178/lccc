# lz4_match_extend: the hot loop's induction variable lives in a stack slot

**Status:** root cause identified and reproduced; the register-allocator change it
calls for is NOT in this round (see *Why not fixed here*). The peephole half of the
gap — the missing memory-operand `cmp` — IS fixed and measured here.

**Date:** 2026-09-30
**Benchmark:** `tests/benchmark/programs/lz4_compress.c` built with
`-DMATCH_RICH=1 -DPASSES=2048`, registered as `lz4_match_extend`
(`run_benchmarks.py:155`).

## The measurement

`lz4_match_extend` is the single worst benchmark in the 39-benchmark corpus:

| benchmark | lccc | gcc | LCCC/GCC |
|---|---|---|---|
| **lz4_match_extend** | **2.1063 s** | **1.1203 s** | **1.903 (1.90× slower)** |
| binary_trees | 1.6736 s | 1.3927 s | 1.224 |
| expat_xml_scan | 50.18 ms | 40.16 ms | 1.249 |
| nbody | 391.58 ms | 313.88 ms | 1.252 |
| hash_table | 13.1425 s | 11.9776 s | 1.113 |

5 reps, paired median wall time, all `Correct: pass`. Full table in
`/tmp/ct/corpus5.log`.

**The static instruction-count census did not flag this benchmark at all.** It
ranked `zlib_ng_adler32` first (+149 instructions) and `nbody` second (+106);
`lz4_match_extend` was not in its top ten. Static counts and wall time disagree
sharply here, which is the reason this round measures runtime first and treats
the census as a hint for *where to look*, never as the objective.

## The hot loop

`lz4_compress.c:80` is the LZ4 match-extension loop:

```c
while (ip < iend && *ip == *match) {
  ip++;
  match++;
}
```

With `MATCH_RICH` the input carries 96-byte repeats every 256 bytes, so a match
runs ~96 iterations. The default pseudorandom input reaches no 4-byte matches at
all and never enters this loop — which is why `lz4_compress` (the same source,
default input) sits at parity while `lz4_match_extend` is 1.9× down.

## What lccc emits

```asm
.LBB14:
    movq 112(%rsp), %rcx        ; ← reload ip from a stack slot
    movzbl (%rcx), %r8d
    movzbl (%rbx), %r9d
    cmpl  %r9d, %r8d
jne .LBB16
.LBB15:
    addq $1, %rcx
    movq  %rcx, 112(%rsp)       ; ← store ip back to the same slot
    addq $1, %rbx
    cmpq 56(%rsp), %rcx         ; ← reload iend, loop-invariant
    jb   .LBB14
```

Seven instructions per iteration, **three of which touch memory**, including a
store-to-load round trip through `112(%rsp)` on the loop's own induction
variable. The store/reload pair is a serialising store-forwarding dependency:
`ip` cannot advance faster than the ~5-cycle forward, so the IV chain — not the
two byte loads — sets the loop's throughput.

## What GCC emits

```asm
.L10:
    movzbl (%r9), %edi
    cmpb   %dil, (%rbx)
    je     .L12
.L12:
    addq   $1, %rbx
    addq   $1, %r9
    cmpq   %r13, %rbx
    je     .L43
```

Four instructions, **zero memory traffic beyond the two byte loads**, and both
pointers plus `iend` live in registers.

## The IR is not the problem

`CCC_DUMP_IR=1 CCC_DUMP_IR_FUNC=main` shows clean SSA:

```
block 14 (.LBB14):
  Load { dest: Value(242), ptr: Value(429), ty: U8, ... }
  Cast { dest: Value(243), src: Value(Value(242)), from_ty: U8, to_ty: I32 }
  Load { dest: Value(245), ptr: Value(430), ty: U8, ... }
  Cmp  { dest: Value(247), op: Eq, lhs: Value(243), rhs: Value(246), ty: I32 }
  term: CondBranch { cond: Value(247), ... }
block 15 (.LBB15):
  BinOp { dest: Value(249), op: Add, lhs: Value(429), rhs: Const(I64(1)), ... }
  BinOp { dest: Value(251), op: Add, lhs: Value(430), rhs: Const(I64(1)), ... }
  Copy { dest: Value(429), src: Value(249) }
  Copy { dest: Value(430), src: Value(251) }
  Cmp  { dest: Value(533), op: Ult, lhs: Value(429), rhs: Value(158), ... }
```

`Value(429)` and `Value(430)` are proper loop phis; `Value(158)` (`iend`) is
loop-invariant. Nothing is memory-resident at the IR level. The defect is
entirely in `src/backend/x86/codegen/machinst_alloc.rs`.

## Root cause

`allocate_window` is a **windowed linear scan**, not a graph colouring allocator
with a spill-cost model. It assigns physical registers to values *defined within
the window*; anything whose live range spans the whole function keeps its
originating stack slot and is accessed through memory operands.

`Value(429)` (`ip`) is used after the loop as well as inside it
(`ip - anchor`, `iend - ip`, `ip - match`), so its web spans most of `main` and
it never gets a register. Inside the loop this costs three memory operations per
iteration. `Value(430)` (`match`) happens to be used only shortly after the loop
and *does* get `%rbx` — the difference between the two is purely how far their
webs reach, not any property of the loop.

The IR-level shapes that would fix it are the standard ones, and all of them
already have (or could have) a home in this pass list:

1. **Loop-IV promotion / live-range splitting at the back edge.** `Value(429)`'s
   live range *within the loop* is one block long. Binding it to a register for
   the loop's extent and saving/restoring it at the preheader and exit deletes
   all three memory operations. This is the high-value change.
2. **LICM for loop-invariant memory operands.** `Value(158)` (`iend`) is
   invariant across the loop yet is reloaded every iteration. `src/passes/licm.rs`
   already exists; the gap is that a value which LICM leaves in a *slot* rather
   than a register gains nothing, because the allocator re-spills it.

## Why not fixed here

Changing `allocate_window` to split live ranges across loop back edges is a
register-allocator redesign. This codebase has an established history of
miscompiles from allocator and unroller changes that looked correct in isolation
(the `int_alu` full-closedness experiment in the previous round miscompiled
`float.c`; the blanket trip-bound raise was rejected for the same reason). The
39-benchmark corpus plus 3891 unit tests would not be sufficient assurance for
an unvalidated allocator change made at the end of a campaign, and shipping a
possible miscompiler to chase 1.9× on one benchmark is a bad trade. The correct
sequencing is: implement (1) behind a flag, measure all 39 with and without, and
land it on its own evidence.

## What *is* fixed here: the missing memory-operand `cmp`

lccc emitted `movzbl (%rbx), %r9d; cmpl %r9d, %r8d` where all three oracles emit
`cmpb (%rbx), %r8b`. The existing `fuse_load_into_alu` cannot produce it: it
parses only same-width `movl`/`movq` loads and requires the consumer to carry
the load's exact width, and neither holds for a zero-extending load feeding a
wider compare.

`fuse_zero_ext_cmp` in `src/backend/x86/codegen/peephole/passes/load_op_fuse.rs`
folds the adjacent triple

```asm
movzbl (%rax), %r8d
movzbl (%rbx), %r9d
cmpl  %r9d, %r8d
```

into `movzbl (%rax), %r8d; cmpb (%rbx), %r8b`.

### Why the narrowing is flag-for-flag exact

Both operands are zero-extensions of the same narrow width, so `X` and `Y` lie
in `[0, 2^n)`. The full compare computes `X - Y`; the narrow one computes the low
`n` bits of the same difference, and `|X - Y| < 2^n` means no borrow escapes the
low half:

* **ZF** — `X == Y` iff the narrow difference is zero. *Identical.*
* **CF** — the unsigned borrow `X <u Y`. The narrow subtraction borrows on
  exactly the same condition. *Identical.*
* **SF** — bit `n-1` of a possibly-negative difference vs bit 31/63 of the wide
  one. **Differs.**
* **PF, OF, AF** — all differ, and none is selectable by any condition code.

So the fold is gated on a new `flag_consumers_are_zf_cf_only`, built on the
existing fail-closed `walk_flag_consumers`: the licence requires the exact {ZF,
CF} condition-code set (`e z ne nz b c nae nb nc ae be na a nbe`). A `js`/`jg`/
`jp` downstream vetoes it, as does any whole-word reader (`lahf`, `pushf`, inline
asm, an unrecognised mnemonic) and any flag flow the walk could not prove.

### Guards, and why each is load-bearing

* **Equal narrow widths only.** A 16-bit compare of an 8-bit load would read one
  byte *past* the address the deleted load touched, which can fault against a
  page boundary. Equality keeps the folded access bit-for-bit the original.
* **No sign-extending loads.** `movsbl` is not a zero extension; folding it would
  compare a sign-extended value against the byte.
* **Adjacency only.** A store between the loads could alias the folded operand.
* **The folded register must be dead at the compare** (`FileLiveness`).
* **Compare width must match the destination bank** — `cmpl` over `movzbl`
  destinations, `cmpq` over `movzbq` ones.

### A miscompile this pass shipped with, caught by its own test

The first implementation always deleted the *second* load and always used
`mem_b`. On the mirrored operand order `cmpl %r8d, %r9d` that emitted

```asm
movzbl (%rcx), %r8d
cmpb   %r9b, (%rbx)      ; %r9d no longer has a definition
```

— a use of an undefined register. The `folds_the_mirrored_operand_order` test
caught it before it reached a binary. Each arm of the operand-order match now
carries the family it deletes together with *that* load's index and memory
operand, so the kept register and the folded address cannot drift apart.

### A pre-existing walker bug this exposed

`walk_flag_consumers` skipped nops and directives but not **blank** lines. A blank
line has no mnemonic, so `flags_effect` fell through to its fail-closed default
and charged it with reading every flag. Any walk that reached the end of a
function — the padding after the last `ret` — therefore reported a whole-word
reader and vetoed *every* flag-divergent rewrite. Skipping blank lines is
strictly more precise: a line that emits nothing cannot observe EFLAGS.

### Measured effect

| | before | after |
|---|---|---|
| `lz4_match_extend` LCCC/GCC | 1.903 | 1.889 |
| `lz4_compress` LCCC/GCC | 1.014 | 0.997 |
| lz4 asm instructions (MATCH_RICH) | 305 | 304 |

**This is a rounding error, not a win.** It is reported as such. The fold is
still correct, general, and matches all three oracles, so it is worth keeping —
but the honest reading is that `lz4_match_extend` is limited by the induction
variable in `112(%rsp)`, and shaving one instruction off a loop whose cost is a
store-forwarding round trip cannot move it. **Do not read the 1.903 → 1.889 as
the fix.** The fix is the register allocator work above, and it is not done.

## Reproducing

```sh
cargo build --profile fastbuild -j2
S=tests/benchmark/programs/lz4_compress.c
./target/fastbuild/lccc -O2 -DMATCH_RICH=1 -DPASSES=2048 -S -o /tmp/l.s $S
gcc -O2 -DMATCH_RICH=1 -DPASSES=2048 -S -o /tmp/g.s $S
grep -n -A 8 '^\.LBB14:' /tmp/l.s      # the 3-memory-op loop
CCC_DUMP_IR=1 CCC_DUMP_IR_FUNC=main ./target/fastbuild/lccc \
  -O2 -DMATCH_RICH=1 -DPASSES=2048 -S -o /dev/null $S   # clean SSA phis

python3 tests/benchmark/run_benchmarks.py \
  --only lz4_match_extend --compilers lccc,gcc --reps 7
```
