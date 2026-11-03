# Narrow counts: the extension is a copy, and RISC-V gets a narrow scan

Round 9 of the count-width work (2026-11-03).  Round 8 (`2026-11-02-count-width`)
made the *cross-width* shapes narrow; this round is about what happens to the
narrow result afterwards, and about the two backends that had no narrow
sequence to run.

```
python3 measure.py probe.c <baseline>/fastbuild target/fastbuild
```

`measure.py` compiles the probe with each driver's `-S`, takes the instruction
lines of every function up to its `ret`, and prints `before -> after` per
target.  The baseline here was `target/fastbuild` at `03b3aea` (round 8's
merge); the "after" column is this round.

## What changed

1. **The extension back to `int` is a `Copy`, not a `Cast`.**  A count of an
   n-bit operand cannot exceed n, so the result already fits the narrow type.
   The `Cast` asked every backend to re-normalise a clean value, which is why
   the rule had to be gated to x86 — the one target where the extension
   happened to be free.  `Copy` says the property instead, the gate is gone, and
   `value_types` widens both sides to `int` (0..64 is below every sign
   boundary, so nothing else has to care).
2. **RISC-V has narrow `Popcount`/`Parity`.**  A byte's SWAR masks fit `andi`,
   so the lane scan needs no constant loads; a byte's parity is three folds
   rather than folds plus the `0x6996` nibble table.
3. **Both narrow emitters mask their operand to its type first.**  They read
   whole-register lanes (`cnt v0.8b`, the 32-bit SWAR), so a sign-extended
   narrow value must not reach them — the promoted form used to guarantee that
   cleanliness and the narrow form has to establish it.
4. **A checksum gate executes all of it** on AArch64/RISC-V under QEMU and
   natively on x86-64 (`tests/regression/check_narrow_count_runtime.py`).

## Measured

`-O2`, instructions inside the function body including `ret`:

| probe | x86-64 | i686 | AArch64 | RISC-V | gcc 16.2 | clang 23.1.4 |
| --- | --- | --- | --- | --- | --- | --- |
| `pc_u8` | 8 -> **7** | 10 -> **9** | 13 -> **12** | 24 -> **15** | 5* | 6 |
| `pc_u16` | 17 = 17 | 19 = 19 | 13 -> **12** | 26 -> **22** | 5* | 18 |
| `pc32` | 17 = 17 | 22 = 22 | 12 = 12 | 24 = 24 | 5* | 16 |
| `pc64` | 21 = 21 | 26 = 26 | 12 = 12 | 24 = 24 | - | 20 |
| `par_u8` | 5 = 5 | 6 = 6 | 14 -> **13** | 16 -> **12** | 4 | 4 |
| `par_u16` | 5 = 5 | 7 = 7 | 14 -> **13** | 18 -> **16** | 5 | 5 |
| `par32` | 8 = 8 | 13 = 13 | 13 = 13 | 17 = 17 | 7 | 7 |
| `par64` | 11 = 11 | 16 = 16 | 13 = 13 | 17 = 17 | - | 10 |
| `np2` | 9 = 9 | 17 = 17 | 16 = 16 | 31 = 31 | 6 | 7 |

\* GCC's five instructions are a libgcc call: `subq $8,%rsp`, `movl %edi,%edi`,
`call __popcountdi2`, `addq $8,%rsp`, `ret`.  It is not an inline count, and
comparing an instruction count against it flatters GCC by about a dozen
instructions of inlined work.  **Clang is the oracle for these rows**, and
against Clang the x86-64 rows are at parity or one behind (the byte count's
`movzbl` prologue, the scratch staging for a 32/64-bit count).

## The residual gaps, honestly

* x86-64 `pc32`/`pc64`: one instruction behind Clang — `push %rcx`/`pop %rcx`
  where Clang spends an argument register the value has already left.  A
  scratch-allocation question, not a count question.
* `np2` on x86-64: 9 against Clang's 7 — a dead `movl $63` zero-arm seed and a
  `32 - (31 - bsr)` reduction that should be `+1`.  Recorded in
  `engineering/llvm-missed/SCALAR-PREDICATE-FOLDS.md`, "What is not done".
* AArch64 and RISC-V: 4-6 instructions of accumulator copies and home slots
  around a body that is now the same length as Clang's.  `f(void) { return 1; }`
  pays the same frame.

## Rejected: a `clean_width` fact

The first attempt at the ARM/RISC-V ext/copy problem was an analysis
(`collect_clean_widths`, a worklist fixpoint over narrow loads, parameters and
zero-extending casts, proving "bits above n are zero" per SSA value) so the cast
emitters could skip a mask they knew was redundant.  It compiled and its unit
tests passed, and then the measurement said no:

* the masks it would remove are mostly *not* redundant — they are the
  materialisation of a narrow value (`movzbl`, `lbu`, the prologue's `and`), and
  the ABI lets a caller pass the high bits of a narrow argument dirty;
* where a mask really was redundant, a one-instruction text peephole already
  removes it (`check_redundant_self_zext.sh` pins that shape);
* the actual cost was the *IR-level* `Cast`, which the pass that creates it can
  simply write as a `Copy`, on every target, with no analysis at all.

An unused analysis is a liability, so it was deleted rather than landed.  The
lesson matches round 8's: measure the emitter you are about to optimise for
before you build the machinery to inform it.
