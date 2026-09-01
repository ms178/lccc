# Follow-up: loop-aware block layout and machine-level loop inversion

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `a73f69c0`
Snapshots: `S14`–`S16` (see `artifacts/SNAPSHOT_LEDGER.md`)

---

## 1. Accomplished

### 1.0 Environment recovered after a harness wipe

The workspace was wiped mid-session. `ms178-1.patch` and `artifacts/` survived
(the standing auto-save duty), so nothing was lost, but the restore exposed two
non-obvious failures worth recording — both now handled by
`scripts/lccc-bootstrap.sh`:

* `~/.cargo/bin/rustup` comes back **without its execute bit**
  (`Permission denied`);
* the **proxy binaries** (`cargo`, `rustc`, …) are gone, and
  `rustup toolchain install` does *not* recreate them — only `rustup-init`
  does. So the toolchain installs "successfully" and `cargo` is still not
  found.

`setup_rust` now repairs both (rustup dispatches on `argv[0]`, so symlinking
the proxies back is the supported recovery) and then *verifies* `cargo`
actually runs rather than assuming it. A silent failure here costs a whole
session, because every later step blames the compiler instead of the PATH.

### 1.1 Block layout is loop-aware — `memchr` −40%

RPO is a *topological* order with no notion of which edge is hot. In a search
loop it laid the cold `return` between the body and the latch, so the body's
branch to the latch was a **taken forward jump over it**:

```text
.LBB2:  cmpl %r8d, %eax ; jne .LBB4   # TAKEN every iteration
.LBB3:  <return the match>            # cold, runs at most once
.LBB4:  leaq 1(%rbx), %rbx ; jmp .LBB1
```

Three branches per iteration, **two taken**, against Clang's two with one.

`relayout_blocks_loop_aware` starts from RPO and makes the **minimum**
deviation that restores loop-body contiguity — a stable partition of the span
between a loop's first and last block. The backend needed no changes: it
already inverts a conditional whose true-target is the next block
(`comparison.rs:403`), so the taken `jne` became a not-taken `je` for free.
Same instructions, one fewer taken branch.

**The minimality is load-bearing.** The first version was a greedy
hot-successor chain. It won `memchr` identically, but where every candidate
sat at the same loop depth its tie-break still walked the CFG chain-first
instead of topologically, reshuffling straight-line code for no reason:
**varint regressed 8.2%**. Reordering blocks is never free. A test now pins
that an already-contiguous loop comes back byte-identical.

### 1.2 Machine-level loop inversion — `memchr` to parity with GCC

With layout fixed, the remaining `memchr` gap was still 2x. Rather than guess,
I hand-edited the assembly into each candidate form and timed all of them:

| variant | time |
|---|---|
| as emitted, 6 insn/iter | 17.459 ms |
| **bottom-tested, guard kept, 5 insn/iter** | **8.769 ms** |
| bottom-tested **+ load folded into the compare**, 4 insn/iter | 8.771 ms |
| GCC 14.2 reference | ~8.80 ms |

Two conclusions, and the second one saved a day of work:

1. Bottom-testing is worth **−49.8%** and lands exactly on GCC.
2. Folding the load into the compare on top of it is worth **nothing**
   (8.771 vs 8.769 ms).

The cost was never the instruction count — it was the unconditional `jmp`. The
rotated loop has one taken branch per iteration instead of a taken conditional
*plus* a taken jump. So the load-compare fusion that had been sitting at the
top of the to-do list as "the obvious next win" was **dropped on evidence**,
and `memory_fold`'s SIB-operand limitation can stay as it is.

`src/passes/loop_invert.rs` implements the rotation. The key design decision is
*where*: it runs **after `eliminate_phis`**, so there are no phis left.
Inversion then degenerates to copying a handful of pure instructions and
retargeting one edge. Compare with the SSA-level
[`loop_rotate`](../src/passes/loop_rotate.rs), which must rewrite header phis —
the part that produced a string of miscompiles, and the reason it is still
opt-in and restricted to single-block bodies (it declines `k_memchr` outright:
`not single-block body (body_len=3)`). Phi elimination has already placed the
induction variable's copy at the end of the latch, so the duplicated test
naturally reads the *updated* value, which is exactly what a bottom test must
do.

Guards, each with a negative test: a single latch; the header must decide the
exit; every header instruction pure and duplicable (loads refused outright
rather than reasoned about); no header value may escape into the body; an
entry from outside the loop must exist; and a size cap on duplication.
`CCC_NO_LOOP_INVERT=1` disables it for bisection.

One of those negative tests caught a real bug: `find_natural_loops` returns one
loop **per back edge**, so a two-latch loop arrives as *two* single-latch loops
and the single-latch guard waved both through, duplicating the test twice. The
pass now merges by header first, as `loop_unroll` does.

### 1.3 Results

Best-of-11, `-O3`, versus GCC 14.2 (>1 = lccc faster):

| kernel | start of session | now | vs gcc |
|---|---|---|---|
| **memchr** | 29.170 ms | **8.748 ms (−70.0%)** | 0.31x → **1.01x** |
| adler32 | 5.794 ms | 5.279 ms (−8.9%) | 0.83x → 0.91x |
| hashmix | — | 30.499 ms | 0.97x |
| classify | — | 27.210 ms | 0.57x |
| matchlen | — | 447.692 ms | 0.54x |
| varint | — | 90.202 ms | 0.52x |
| namechars | — | 34.293 ms | 0.45x |

**geomean 0.564 → 0.677**, zero regressions. `memchr` now edges out GCC.

---

## 2. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1415 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=559 FAIL=0 SKIP=15**, AB-diff 0 |
| same, `CCC_LOOP_ROTATE=1` stacked | **PASS=559 FAIL=0**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3354 configs |
| `bench_kernels.py --baseline` | memchr −70.0%, adler32 −8.9%, no regressions |

---

## 3. To do next, in priority order

### 3.1 Rotate the loops that `loop_invert` still declines

`memchr` and `adler32` rotated; `matchlen` (0.54x), `varint` (0.52x) and
`namechars` (0.45x) did not. Run with `CCC_DEBUG_LOOP_INVERT=1` and read the
bail reason for each — the guards are deliberately tight and the first ones to
relax should be chosen by which kernels they unblock, not by which looks
easiest. Given §1.2, expect rotation to be worth far more than any
instruction-count work on the same loop.

### 3.2 `namechars`: the counting form of the classifier

Unchanged from the previous session and still the worst kernel. `if (pred) n++`
does not produce `Select`s — the hit edges converge on a shared increment block
before the join — so `range_fold` and `set_membership` never see it, while the
boolean-returning `k_classify` gets the full treatment. The two kernels
deliberately bracket this. `set_membership`'s matcher already anticipates a
`Phi [(Const(1), T_1), …]` join; teaching it (or `if_convert`) to see through
the shared hit block closes it.

### 3.3 Vectorisation is the remaining structural gap

GCC vectorises `memchr` (149 insns of SIMD) and the Expat classifier (372) and
still only ties lccc's scalar `memchr`. Where it wins big it is because it is
using vectors. That is the next order-of-magnitude item, not more scalar
peepholes.

### 3.4 Def-dominates-use in the IR verifier

Unchanged and still the top verifier item: the six existing checks do not
include dominance, which is how an SSA violation once shipped past every gate.
`verify.rs` already computes reachability; Cooper-Harvey-Kennedy dominators
over RPO is the natural next step.

### 3.5 Standing items not advanced

- Linker oracle: lld 23.1 / mold 2.42 (X86+i686-only preset) / bfd 2.47.
- Broader benchmark corpus: SQLite b-tree compare, kernel `memcpy`/checksum,
  zlib-ng CRC32; and a local Clang so the `vs clang` column stops being empty.

---

## 4. Notes for whoever picks this up

- **Hand-edit the assembly and time it before writing the pass.** Four variants
  of `k_memchr` took ten minutes and showed that the transformation at the top
  of the to-do list (load-compare fusion) was worth 0.01% while the one below
  it was worth 50%. Building the first would have been a week wasted at full
  validation cost.
- **Instruction count is not runtime.** Rotation removes one instruction and
  halves the time; the memory fold removes one instruction and changes nothing.
  Branches, not instructions, were the currency in every kernel here.
- **Pick the point in the pipeline where the problem is easy.** Loop rotation
  is hard in SSA because of phis and easy after phi elimination. The same
  transformation, thirty lines apart in the pipeline, moves from "string of
  miscompiles, still opt-in" to "pure duplication of a pure test".
- **Minimal deviation beats clever heuristics.** The greedy layout won the same
  memchr case and cost 8.2% elsewhere. If a region has no problem, leave its
  order alone.
- Negative controls keep earning their keep: 7 of the 10 `loop_invert` tests
  assert the pass does *not* fire, and one of them found the merge-by-header
  bug.
