# Follow-up: escaping induction variables, a vectorizer miscompile, and
# randomized MachInst stress

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `3442049e`
Snapshots: `S21`, `S22` (see `artifacts/SNAPSHOT_LEDGER.md`)

---

## 1. Accomplished

### 1.1 `iv_widen` now handles an escaping induction variable

The top item from the previous follow-up. `iv_widen` required the IV's only
uses to be addressing plus the trip compare, so this shape bailed:

```c
while (n < max && x[n] == y[n]) n++;
return n;                 /* the IV ESCAPES */
```

That is gzip's `longest_match` — the most common byte-compare loop in
compression code — and the bail left `movslq %ebx, %rbx` **on the loop-carried
dependency path**, the exact pattern whose removal was worth −35% in S08.

Escaping uses are now repaired with one `Cast I64->I32` per escaping block,
placed after that block's phi prefix. The block is *outside* the loop, so the
truncation runs once on the way out while the body loses an instruction — and
on `match_len` it folds into the `movl %ebx, %eax` the return already needed,
so it costs literally nothing:

```
    movslq %edx, %r8            # bound widened ONCE, in the preheader
.LBB2:
    movzbl (%rdi,%rbx), %r10d   # no movslq on the carried path any more
    movzbl (%rsi,%rbx), %eax
    cmpl %eax, %r10d
    jne .LBB4
    leaq 1(%rbx), %rbx
    cmpq %r8, %rbx
    jl .LBB2
```

Only blocks the loop header **dominates** are eligible — the widened phi must
reach the truncation on every path. An exit-*merge* phi is still refused: a phi
operand is evaluated on the edge, so its truncation would have to sit in a
predecessor, which for a loop exit is inside the loop and therefore once per
iteration. That trade is not worth making.

**A latent weakness this exposed.** With escapes allowed, the pass began
reaching plans it used to reject outright, and `apply_widen` — which trusted
the earlier `cast_dest_feeds_only_gep` approximation — dropped a `Cast` whose
dest still had a live consumer. Eight corpus tests died with *"value 40 has no
register, stack slot, Copy, or GlobalAddr definition"*. Casts are now dropped
only after a **liveness check at the point of removal**, which an imprecise
predicate cannot fool; at worst a dead Cast survives for DCE, which costs
nothing.

### 1.2 A silent miscompile in the AVX2 reduction vectorizer

The new regression test for §1.1 failed against the GCC oracle — and
`CCC_NO_IV_WIDEN=1` reproduced it, so it was **pre-existing**, not mine.

Both of the vectorizer's addressing schemes redefine what the loop counter
counts. The byte-offset scheme steps `elem_size * vec_width` **bytes** per
iteration; the element-index scheme divides the trip count so the counter
numbers **vector iterations**. Inside the loop only addresses read it, so
neither is visible there. A use *after* the loop reads a number that is no
longer the element index:

```c
for (; n < max; n++) acc += v[n];
return acc + (n >> 2);
```

| | result for `max == 32` |
|---|---|
| byte-offset scheme | 528 (`n` == 128) |
| element-index scheme | 497 (`n` == 4) |
| **GCC 16.2 / Clang 23.1 / ICC / ICX** | **504** (`n` == 32) |

A wrong answer, not a crash, on `for (i = 0; i < n; i++) ...;` followed by any
use of `i` — one of the most ordinary shapes in C.

Fixing the counter in the exit block is possible in principle (`trips *
vec_width`, plus whatever the scalar remainder advanced) but has to agree with
the remainder loop on every exit path, and getting that subtly wrong
reintroduces the same class of silent miscompile. The fix is therefore a
**precondition** — alongside the existing contiguity check, and for the same
documented reason ("aborting midway leaves the loop half-transformed") — that
declines to vectorize when the counter escapes.

`tests/regression/iv_widen_escaping_iv.c` covers six shapes: signed and
unsigned escaping IVs, an escape into arithmetic, addressing-only (guarding
against regressing the case that always worked), a two-exit loop escaping on
both paths, and the vectorizable reduction that exposed this bug.

### 1.3 Randomized MachInst stress — and the bug it found

The hand-written corpus from last session covers each variant once. The
interesting failures live in the **cross product** of operand shapes, widths
and registers, which no per-variant golden test reaches.

`random_corpus` generates 4000 instructions from a SplitMix64 PRNG with a fixed
seed — deterministic, so every failure is reproducible; a flaky fuzz test is
worthless. All 4000 are fed to the real assembler.

It immediately found: **there is no two-operand 8-bit `imul`.** x86 offers only
the one-operand `imul r/m8` (`AX = AL * r/m8`), so `AluOp::Imul` at `OpSize::S8`
emitted `imulb %al, %bl`, which GAS rejects. The 32-bit form computes the
identical low 8 bits — all an S8 multiply is defined to produce — and is what
every other compiler emits for `char * char`, so `Imul`/`S8` now widens to
`S32`. That `(op, width)` pair is exactly what a golden test cannot produce,
because the author writes one case per variant and picks a plausible width.

Two supporting improvements:

* The failure reporter now prints the offending **source line** next to the
  assembler's message. A fuzz failure that makes you go dig in `/tmp` is a fuzz
  failure nobody debugs.
* The generator was constrained to **well-formed** input: a `Mov` destination
  is a register or memory, never an immediate. Feeding the emitter
  unrepresentable instructions tests nothing and produces noise that hides real
  defects.

Three further invariants are pinned across the randomized corpus:
reproducibility, deterministic emission, and no unresolved vreg or empty
emission.

---

## 2. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1468 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=560 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3360 configs |
| MachInst differential + 4000-instruction fuzz | all accepted by GAS |
| `bench_kernels.py` | no regressions; geomean 0.676x vs GCC |

---

## 3. To do next, in priority order

### 3.1 Restore vectorization for loops whose counter escapes

§1.2 trades throughput for correctness, and that trade should not be permanent.
The counter's true final value is derivable (`trips * vec_width` plus the
scalar remainder's advance); materializing it in the exit block would let these
loops vectorize again. It must agree with the remainder loop on **every** exit
path — write the exit-value fixup first, then re-enable, with
`iv_widen_escaping_iv.c` case 6 as the guard.

### 3.2 `namechars` — the counting form of the classifier

Unchanged and still the worst kernel (0.44x). `if (pred) n++` does not produce
`Select`s, so `range_fold` and `set_membership` never see it while the
boolean-returning `k_classify` gets the full treatment. The two kernels
deliberately bracket this.

### 3.3 Vectorisation breadth

GCC's 149/243-instruction outputs on `scan`/`match_len` are SIMD. lccc's scalar
`memchr` ties it on this host, but that will not hold on wider data.

### 3.4 Encoding-level differential

The MachInst differential proves GAS *accepts* the text. The stronger property
is that the bytes match what GAS produces for the intended instruction;
`insndiff.py`/`encdiff.py` already exist and could be pointed at the MachInst
corpus.

### 3.5 Def-dominates-use in the IR verifier; linker oracle

Both unchanged from previous sessions.

---

## 4. Notes for whoever picks this up

- **A new test earning its keep on day one is the norm, not luck.** The
  regression test written for the `iv_widen` change immediately failed against
  the GCC oracle and uncovered a pre-existing silent miscompile in a completely
  different pass. Always diff a new test against the oracle before assuming
  your own change is the cause — `CCC_NO_IV_WIDEN=1` settled attribution in one
  command.
- **Verify deadness where you delete, not where you analysed.** The `iv_widen`
  breakage came from trusting an approximate predicate computed earlier. A
  liveness check at the point of removal cannot be fooled and costs a linear
  scan.
- **Fuzz the cross product, seed it deterministically, and print the offending
  line.** Both MachInst bugs found so far were `(op, width)` or
  `(operand-shape, width)` combinations that no author would have written by
  hand.
- **Constrain a generator to representable input.** Nonsense inputs produce
  nonsense failures that mask the real ones.
