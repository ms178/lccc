# Strength reduction: which memory consumers a derived IV can serve

This note records a measurement campaign on `iv_strength_reduce` (IVSR) and the
numbers that decide how far its replacement may reach.  It exists so the three
dead ends below are not walked again: all of them look like wins on paper and
all three are losses on the machine.

## The shape that motivated the work

`tests/benchmark/programs/nbody.c`, built with `-O2 -march=x86-64-v3 -DSTEPS=2000`.
The inner pair loop walks `bodies[i]` and `bodies[j]` (the two C-level loop
variables of `advance`; the IR renumbers them after inlining, so the sections
below name the C variables and quote IR block labels where it matters) where
`bodies` is a static array and the byte offset is `index * 56`.  IVSR's derived-expression finder
does locate those multiplies (`[IVSR]` reports `derived=1` for both loops), and
yet the multiply survives into the emitted loop.

Root cause, established by instrumenting the derived-use matcher: the collector
accepts exactly one consumer spelling,

```text
GetElementPtr { offset: Operand::Value(mul_dest) }
```

while the SLP vectorizer rewrites the loop into

```text
Copy        { dest: v242, src: Value(v250) }
Copy        { dest: v32,  src: Value(v250) }
Intrinsic   { op: VecLoadF64x2,  args: [base, Value(v250), Const(I64(24))] }
Intrinsic   { op: VecStoreF64x2, args: [value, base, Value(v250), Const(I64(24))] }
```

So the byte offset is read by a `Copy` and by a vector memory intrinsic, and the
GEP-count is zero: the group is never formed and no recurrence replaces the
`movslq; imulq $56` pair.

## What was tried, and what it cost

Three implementations, each measured against the unmodified pass with two
independent instruments.  "Program instructions" is the sum of the executed
block sizes in a `qemu-x86_64 -d in_asm,exec,nochain` log, restricted to the
program's own address range; the callgrind column is a `--dump-instr=yes` run,
which counts retired instructions exactly.

| variant | hot loop | program insns (qemu) | callgrind total | callgrind `main` |
| --- | --- | --- | --- | --- |
| shipped pass (baseline) | 44-45, `imul` in loop | **1,132,825** | **1,252,502** | **1,100,994** |
| index recurrence (new value, base kept live) | 44, no `imul` | 1,202,829 | 1,311,939 | 1,140,994 |
| pointer recurrence extended to the intrinsics | 42, no `imul`, pointer walk | 1,154,820 | 1,272,498 | 1,108,990 |
| `gcc -O2` reference | 28 | - | 883,838 | - |

Both instruments agree on the sign and the magnitude, and the baseline
reproduces bit-identically after the reverted work.

### Why the index recurrence loses

The recurrence is cheap (one `add` per iteration), but the pass pays for it in
registers, and the multiply it removes was almost free to begin with: the
backend's SIB addressing absorbs an index register at no instruction cost, so
`base + (k*56)` is one memory operand either way.  Measured in the emitted loop,
the transform replaces `movslq; imulq $56` (2 instructions, both in the SIB
path) with `leaq base+idx; addq $56` plus one more live value across the whole
nest.  Register pressure then shows up as an extra spill in the surrounding
loops, and the net is +6.2 %.

### Why extending the pointer form still loses

The pointer form is the right shape -- it is what GCC emits (`addq $56, %rax`
with `disp(%rax)` accesses) -- and it does shrink the hot loop from 45 to 42
instructions with the multiply gone entirely.  It loses anyway, because the seed
for a nested loop's pointer is computed in the *enclosing* loop body: the
inner loop's start address depends on `i`, so `(i+1) * 56 + bodies` is
materialized once per `i`-iteration where the old code recomputed it inside the
inner loop, and the new pointer is live across the whole nest.

The cost decomposition is measured, not modelled, because the two obvious
per-iteration guesses do not reconcile with the counters.  For
`NBODIES = 5`, `STEPS = 2000` the inner pair loop runs
`C(5,2) * 2000 = 20 000` iterations while the `i`-loop runs `5 * 2000 = 10 000`
(8 000 of which enter the inner loop).  A "saves 2 per inner iteration, adds 3
per outer iteration" reading would predict a *net win* of roughly 10 000-16 000
instructions; the instruments measure the opposite sign, so that reading was
wrong and is replaced here by the dump-attributed delta:

| region (callgrind, `--dump-instr=yes`) | baseline | pointer form | delta |
| --- | --- | --- | --- |
| `main` (caller loops) | 1 100 994 | 1 108 990 | **+7 996** = +4 per step, −4 |
| everything outside `main` (`advance`, the hot nest) | 151 508 | 163 508 | **+12 000** = +6 per step |
| total | 1 252 502 | 1 272 498 | +19 996 (+1.60 %) |

qemu agrees on sign and size (+21 995, +1.94 %).  So the regression is *not*
mostly in the hot loop at all: two thirds of it is +6 instructions per step
inside `advance`, i.e. work attached to entering and leaving the nest rather
than to the 20 000 inner iterations that the transform was supposed to make
cheaper.  That is the accurate statement of why the pointer form loses, and it
is the one to trust; the earlier per-iteration sentence counted instructions in
the wrong regions.

## Consequences for the pass

IVSR keeps its pointer form and its existing consumer matcher.  The matched
consumer set is deliberately not widened until a replacement pays for itself:
with the current backend, a new loop-carried value costs more than the address
arithmetic it removes, unless the value replaces *multiple* separate
computations or the loop is register-rich.

## Where the nbody gap actually is

The multiply is 2 instructions of a 45-instruction loop; GCC's loop is 28.  The
difference is elsewhere, and it is worth 4 instructions per iteration alone:

```asm
; lccc: lane 0 and lane 1 of the vector are extracted through the stack
movsd  %xmm15, -144(%rbp)
pshufd $0x0E, %xmm15, %xmm0
movsd  %xmm0, -152(%rbp)
movsd  -144(%rbp), %xmm5        ; reload
movsd  -152(%rbp), %xmm1        ; reload
```

GCC keeps the extraction in registers (`vunpckhpd`/`vmovddup`).  Two further
items of the same order: the loop-invariant `lea bodies(%rip), %rcx` is
re-materialized every iteration instead of being hoisted, and the scalar `dz`
component is reloaded from the array rather than derived.  Those three are the
next lever; each has a measured or counted cost, unlike the multiply this pass
chases.

## Measurement methodology notes

Two traps cost most of the debugging time in this campaign and are worth
recording:

* Translation-cache tracing attributes one loop iteration to *two* blocks when
  qemu splits or re-translates a block (`45 x 12000` plus `48 x 8000` for a
  20,000-iteration loop).  A block-level profile from such a log can therefore
  report a loop as *larger* than it is.  Totals are reliable; per-block sizes
  across two builds are not comparable unless the block structure is identical.
* `callgrind` profile files compress positions (`+N` deltas) **and** names (an id
  table that a bare `fn=(id)` refers back to), and they contain both `fn=` self
  sections and caller-context sections that repeat the same instructions.  A
  hand-written reader that ignores either detail double counts the program.
  `callgrind_annotate` is the authoritative reader; use it for totals.
