# Follow-up D — memory RMW + index-scale folding, and the measured vectorizer targets

Round scope: finish the in-flight work, rebase on `ms178/lccc` main (`8db75621`),
harden, benchmark, red-team.  This note is the evidence trail for what shipped
and for what the *next* round should do, measured rather than guessed.

## 1. What shipped (series on top of `8db75621`)

| commit | subject |
| --- | --- |
| `d6c1baf2` | vectorize: refuse the FMA transform when ANY entry edge is not isolatable |
| `a4df7427` | loop-rotate: prove the affine fold's non-wrap obligation, and run it on every loop |
| `fe30483e` | x86 peephole: fuse the sign-extended boolean into `cmovcc` |
| `d9b01046` | x86: fuse `table[idx]++` into one scaled read-modify-write |
| `6904e88a` | x86 peephole: fold the narrow (byte/word) counters through their copy |

All four carry: a regression gate, a differential fixture against GCC, unit
tests where the transform is a local text rewrite, and a kill switch that the
gate proves is *effective* (see §3.2).

## 2. Measured effects (Callgrind, `-O2 -march=x86-64-v3`, Ir)

Same ISA for both compilers, `scripts/callgrind_ab.py`, 11-kernel subset first
(histogram, ascii_case_fold, binary_search, moving_stats, csv_field_sum,
conv_u8_3x3, matmul, affine_countdown, bitops, tce_sum, libm_round_family):

| kernel | GCC 16.2 | lccc before | lccc after | ratio |
| --- | ---: | ---: | ---: | ---: |
| histogram | 1,667,734 | 4,179,149 | **3,261,659** | 2.506 → **1.956** |
| ascii_case_fold | 1,189,088 | 2,522,590 | **2,211,294** | 2.121 → **1.860** |
| others (9 kernels) | — | — | unchanged (±16 Ir) | — |
| **geomean of the 11** | | 1.19662 | **1.16997** | |

Both shipped folds are visible in the emitted body of `bins[src[i]]++`, which
is now instruction-for-instruction the same shape GCC emits (the residual
difference is the loop-invariant `lea` GCC hoists and lccc rematerialises):

```asm
    movzbl (%rdi,%rsi,1),%edx
    addq   $1,(%r9,%rdx,8)
```

Cost per element went from 8 instructions (load, `lea`, store, plus the
`shl`/`lea`/`add` address chain) to 3.

### 2.1 The narrow (byte/word) counters

The first cut of `fold_memory_rmw` only saw a store that named the incremented
register's family directly, so `counts[c]++` kept its relay while
`bins[i]++` collapsed.  The backend reaches a byte/word store through a narrow
copy of the low bits:

```asm
    movzbl (%rdx), %r8d      ; load: memory width B, register width L
    addl   $1, %r8d          ; increment (32-bit)
    movzbl %r8b, %r9d        ; narrow copy of the low byte
    movb   %r9b, (%rdx)      ; store
```

`6904e88a` folds that to `addb $1, (%rdx)` -- and to `addw $1, (%rdx,%rsi,2)`
for the 16-bit form, where the index-scale fold then also applies.  Measured on
`rmw_bump_u8` / `rmw_bump_u16` in `tests/regression/rmw_sib_fold.c`: 6 and 7
instructions respectively, now 4 and 3.

The obligations that make it sound: the fused `add` only reproduces the low
`mem_width` bits (both the store's truncation and the `add{b,w,l}` wrap agree,
which is why sign- and zero-extension are interchangeable here); the copy must
read the EXACT spelling of the value family at that width (a wider read would
carry the incremented high bits into a value the fused `addb` never computes);
and the copy's destination joins the deadness obligations as well as the
address-text exclusion.

## 3. Red-team findings on this round's own code

Every one of these was found by running the code, not by reading it.

### 3.1 The emitted operand without its `%` (a real segfault, caught by the differential)

The first cut of `fold_shift_into_sib` formatted the index operand as
`(%r9, r11, 8)`: I stripped the `%` when slicing the register name and only
added it back for the first operand.  lccc's assembler accepts bare register
names, so the encoded bytes were *correct* — while this module's own register
bookkeeping (`LineInfo::reg_refs`, every liveness scan, `is_full_write`) uses
the full spelling and saw **no read of `%r11` at all**.  A later
`eliminate_dead_pure_writes` therefore deleted the copy that fed the index
(`movl %eax, %r11d`) and the kernel dereferenced `bins[i*8]` → SIGSEGV, while
`objdump` showed the intended `lea (%r9,%rdx,8),%r9`.

Lessons encoded in the gate: (a) emitted text must be canonical — both
operands keep their `%`; (b) the gate greps the *text* for a bare register in a
SIB operand, because the bytes cannot show this class of bug; (c) a differential
that only compares compiler outputs on *simple* shapes would not have caught it
(see 3.3 for the shape that finally did).

### 3.2 The inert kill switch (a gate that tested nothing)

`check_rmw_sib_folds.sh` originally asserted oracle parity with and without
"the kill switch" set as `CCC_NO_MEM_RMW` / `CCC_NO_SHL_SIB`.  Those names do
not exist: this module's knob is `CCC_PEEPHOLE_SKIP=<pass>` (the skip-set names
are `mem_rmw` and `shl_sib`).  Three of the four parity arms silently rebuilt
the *default* configuration and proved nothing.

Fixed, and the gate now carries **mutation arms**: with `mem_rmw` skipped, the
memory RMW operand must be ABSENT; with `shl_sib` skipped, the scaled SIB add
must be ABSENT.  A knob that stops working now fails the gate instead of
quietly passing it.

### 3.3 The RMW address hole (found by re-deriving soundness, then pinned)

`fold_memory_rmw` originally checked only that the *incremented* register does
not appear in the address text.  Two shapes need more, and both are now
rejected explicitly:

* `movq (%r9),%r9; leaq 1(%r9),%rax; movq %rax,(%r9)` — the load overwrites its
  own address register, so line 3's `(%r9)` and line 1's `(%r9)` (equal as
  *text*) denote **different addresses**.  Text equality alone is not
  sufficient; the loaded register is now required to be absent from the address
  too.
* `movq (%r9),%r13; leaq 1(%r13),%rax; movq %rax,(%r9)` is the valid shape and
  stays folded.

### 3.4 The fixture bug the oracle caught

Extending `rmw_sib_fold.c` with signed-byte counters, I wrote
`sc8[(signed char)(a ^ i)] += 3` -- a subscript that is negative for half the
inputs.  GCC's binary segfaulted; lccc's happened to survive.  The oracle-first
rule is what made this a five-minute fix instead of a "lccc survives where GCC
crashes, ship it" mistake.  The index is masked and the comment in the fixture
records the incident.

### 3.5 Hand-rolled deadness replaced by the shared oracle

The first cut combined `FileLiveness::live_after` with a local text scan by
hand.  That disjunction is weaker than the module's audited
`provably_dead_lv` (`LazyDeadness`): the syntactic proof is only sound when it
runs over the whole function with the caller's owned-line list, and the
block-local scan returns "dead" when it meets a full redefinition inside the
same text region — which is not the same statement.  Both new passes now use
`LazyDeadness::dead_after` + `invalidate`, the same oracle every other fold in
the module consults.

### 3.6 What stayed a refusal (deliberately)

* The `shl` fold keeps `%rsp`/`%rbp` out of the index field (encodings collide
  or force a displacement byte) and requires `addq` — a 32-bit `addl`
  zero-extends where `lea` does not.
* The window between the shift and the add exists only because the backend
  rematerialises a `GlobalAddr` `lea` in between; inside it, no line may mention
  the shifted register or READ the flags, and every barrier ends the window.

## 4. The remaining corpus gaps are vectorizer (and one loop-collapse) work

Measured, with the compiler's own diagnostics as evidence.  These are the next
round's targets, ordered by payoff.

### 4.1 Load-free map loops: the map vectorizer bails (`[VEC-MAP] BAIL: no loads`)

`histogram.c`'s fill loop

```c
for (unsigned i = 0; i < N; ++i) bytes[i] = (unsigned char)((i * 2654435761u) >> 24);
```

is 1.83 M Ir of my 3.15 M in that kernel; GCC runs it on AVX2 and spends ~0.06 M.
`LCCC_DEBUG_VECTORIZE=1` gives the exact reason:

```
[VEC-MAP] ENTER: header=1, body={1, 2}
[VEC-MAP] BAIL: no loads
```

`analyze_map_pattern` requires at least one loaded source stream
(`src/passes/vectorize.rs`, the `load_infos.is_empty()` bail), and `MapExpr` has
no induction-variable node at all (`Load`, `Invariant`, `BinOp`, `Sqrt`, `Cmp`,
`Select`, `MinMax`).  Supporting `a[i] = f(i)` needs: a `MapExpr::Iv` variant,
lane-index materialisation in the preheader (`base + {0..w-1}`), and the
matching tail/remainder plumbing.  That is the single highest-value vectorizer
item measured this round, and it applies to every initialisation/fill loop in
real code.

### 4.2 `ascii_case_fold` (1.860): vectorization coverage, not the select

The select lowering is now optimal (`cmovbe`, no `setbe`/`testl`/`movsbq`
relay).  GCC's remaining 1.86× lead on `main` is that it vectorizes the byte
kernel with AVX2.

### 4.3 `moving_stats` (1.682) and `binary_search` (1.444)

* `moving_stats`: the 16-iteration inner window is recomputed from scratch; GCC
  vectorises the sum/min/max window.  Same class as 4.2.
* `binary_search`: the search loop costs 18 instructions per iteration against
  GCC's 10.  The gap is a *peephole* opportunity, not a vectorizer one — see
  §5.
* GCC also vectorizes `binary_search`'s `init` loop (`vpaddd`/`vmovdqa`) and
  `loop_patterns` (1.320) / `double_reduction` (1.151) — all the same
  "vectorize the simple loop" story.

### 4.4 `ring_fifo` (2.740): loop collapsing, not a loop-body fix

GCC collapses the recurrence to a closed form (15 instructions total); lccc
keeps a 14-instruction loop.  A loop-collapsing pass (SCEV-style closed form for
a simple arithmetic recurrence) is a new feature, not a peephole.

### 4.5 The Godbolt oracle agrees (static insn counts, `--rank --all-functions`)

`scripts/codegen_oracle.py` against {gcc16.2, clang, icc, icx}, `-O2
-march=x86-64-v3`, whole functions, worst gap first (evidence:
`engineering/evidence/ORACLE-RMW-1/rank.json`):

| gap | benchmark | function | lccc | best oracle | vec insns (lccc) |
| ---: | --- | --- | ---: | ---: | ---: |
| 86 | moving_stats | main | 217 | gcc16.2=131 | 0 |
| 52 | loop_patterns | main | 231 | gcc16.2=179 | 69 |
| 43 | ring_fifo | main | 56 | gcc16.2=13 | 0 |
| 43 | double_reduction | main | 135 | gcc16.2=92 | 44 |
| 25 | matmul | matmul | 46 | gcc16.2=21 | 12 |
| 20 | binary_search | main | 71 | gcc16.2=51 | 0 |
| 20 | csv_field_sum | main | 130 | gcc16.2=110 | 0 |
| 4 | ascii_case_fold | main | 111 | clang=107 | 0 |

8 behind, 0 tied, 2 ahead; total static gap 293 instructions.  `ascii_case_fold`
is down to +4 static instructions (from +40 before the select fusion), and the
rows that remain are the same vectorizer/loop-collapse classes as §4.1-4.4 --
except `binary_search`, whose gap is the per-iteration cmov staging of §5.

## 5. Sized but NOT shipped: the if-conversion cmov inversion

`find()` in `binary_search.c` emits, per search iteration:

```asm
    leal 1(%rdx), %r8d          ; mid+1
    subl $1, %edx               ; mid-1
    cmpl %r11d, %esi
    cmovll %r8d, %r10d          ; lo = v<key ? mid+1 : lo     (already optimal)
    movq %rdi, %rcx             ; <-- copy hi out of the way
    movl %edx, %edi             ; <-- stage the false value
    cmovll %ecx, %edi           ; hi = v<key ? old hi : mid-1
```

Three instructions where one suffices, by inverting the condition:

```asm
    cmovgel %edx, %edi          ; hi = v>=key ? mid-1 : hi
```

The rewrite is **bit-identical** when the cmov width is 64-bit: the deleted
copy is a full-width copy of the destination, so "keep the destination" and
"move the saved copy" agree in every bit.  For a **32-bit** cmov it is not
bit-identical: the original always zero-extends the destination (both
`movl` and the 32-bit `cmov` zero-extend), while the inverted form leaves the
destination's previous upper 32 bits in place on the not-taken path.  That is
unobservable iff nothing ever reads the family at 64-bit width while it holds
this I32 value — an obligation that must be *proved*, not assumed, and the
proof needs a width-aware "reads the 64-bit spelling" scan that this module does
not have yet (`line_refs_family` is width-blind, and `dest_is_full_width`
answers the opposite question).

Expected value: ~2 instructions per select, i.e. ~11 % of that loop.  Decision:
not shipped this round — the 64-bit-only variant would be free of the
obligation but fires nowhere in the corpus, and shipping the 32-bit variant
without the proof is exactly the kind of "probably fine" this project does not
accept.  Next round: add `mentions_reg64(family)` (token-exact, read positions
only) and gate it with a fixture whose destination family carries dirty upper
bits.

## 6. Verification run this round

* `scripts/callgrind_ab.py` — 11-kernel ISA-matched A/B, **output-differential
  against GCC for every kernel** (the script refuses to report on a mismatch).
* `tests/regression/check_rmw_sib_folds.sh` — PASS (oracle parity at
  -O1/-O2/-O3 × {default, skip `mem_rmw`, skip `shl_sib`, skip both} + fired
  shapes + mutation arms + index-definition intact).
* `tests/regression/check_select_from_compare.sh` — PASS.
* Regression subsets: `run_regression_suite.sh peephole` 14/14 PASS,
  `... local` 16/16 PASS.
* `scripts/check_ci_gate_parity.py` — PASS (122 commands; every hosted CI step
  mirrored in `ci_local.sh`).
* `scripts/ci_local.sh --fast` — see §7.
* `python3 scripts/test_ci_gate_parity.py` — 21 tests OK.

## 7. Run log

`ci_local.sh --fast` was re-run on the final tree of this round; the result is
recorded in the session snapshot ledger.  Earlier in the round, an unstamped
partial run reported `130 passed, 2 failed, 5 skipped` with the two failures
being `doc-link-integrity` (broken links to the wrap fixtures) and
`ci-gate-parity` (the affine gate missing from the workflow) — both fixed and
both PASS on this tree.
