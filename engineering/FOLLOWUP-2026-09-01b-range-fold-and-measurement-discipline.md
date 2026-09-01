# Follow-up: performance tuning against the Godbolt oracle, and two
# measurement/delivery defects found along the way

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `0cdd077`
Snapshots: `S11`, `S12` (see `artifacts/SNAPSHOT_LEDGER.md`)

Prior session's work (IR structural violations 396 → 0, the verifier gate, the
`movslq` win, Clang 23.1) is merged upstream as `25d7cac`.

---

## 1. Accomplished

### 1.1 `range_fold` never fired on the idiom it was written for

`src/passes/range_check.rs` exists specifically to turn
`x >= lo && x <= hi` into `(unsigned)(x - lo) <= hi - lo`, and its own header
names Expat's `xml_name_continue` and SQLite's varint classifier as the
targets. It was enabled, it was reached, and it did nothing.

`resolve_bool_cmp` looks through the boolean-widening cast the frontend emits
after every comparison. Its allow-list was:

```rust
(IrType::I8 | IrType::U8, IrType::I16 | IrType::I32 | IrType::U32)
```

but the IR for `char c; return c >= 'a' && c <= 'z';` is:

```
Cmp    v6  = Sge v1, I8(97)
Cmp    v10 = Sle v1, I8(122)
Cast   v11 = v10 (U8 -> I64)      <-- not in the allow-list
Select v14 = v6 ? v11 : Const(0)  (ty I64)
```

`I64` was missing, so the match failed and the pass bailed — on the canonical
single range, the simplest case it has. And because `set_membership` consumes
`range_fold`'s `Range` members, the *entire* classifier optimisation chain was
starved behind this one gap.

Fixed by listing every wider integer width. The soundness argument is recorded
in the code: the source is verified to be a `Cmp` result, so it is 0 or 1, and
both zero- and sign-extension of 0/1 yield 0/1 at any width. Narrowing stays
excluded and is pinned by a negative test.

**Effect** (`-O3`, x86-64):

| source | before | after |
|---|---|---|
| `c>='a' && c<='z'` | 8 insns, no fold | `subq $97; cmpb $25; setbe` — GCC's form |
| three-range ladder | 35 insns, 2 branches | **16 insns, 0 branches** |
| full boolean classifier | 45 insns | **22 insns (−51%)** |

The last one also shows `andq $-33, %rax; leal -65(%rax), %esi; cmpl $25` —
that is `set_membership`'s `[a-z]`+`[A-Z]` case-fold merge, which had never
been reachable before.

**Runtime**, new `tests/bench/k_classify.c`: **27.886 → 22.677 ms, −18.7%**,
0.53x → 0.65x versus GCC. Well above this host's noise floor (§1.3), and the
harness confirms it as a genuine change rather than layout drift.

The Godbolt oracle is what located this. Comparing the Expat classifier across
`gcc16.2` / `clang 23.1` / `icx` showed lccc emitting an **11-deep
compare-and-branch chain** where GCC vectorises (372 insns of SIMD) and ICX
uses 32 — a binary search over the ranges plus adjacent-constant folding
(`ch=='-'||ch=='.'` → `addb $-45; cmpb $1`). That gap was the signal to go
looking; the IR dump then localised it to one missing enum variant.

### 1.2 The benchmark harness was never delivered

`.gitignore` carries `bench_*` (intended for compiled benchmark binaries). It
silently swallowed `scripts/bench_kernels.py`. The result: `tests/bench/`
shipped upstream last session with six kernels and a timing driver, and **no
harness to run them**. The snapshot script builds the patch from git, so an
ignored file simply never appears — no warning, no error.

Fixed with an explicit negation and a comment explaining why it is there.
Worth remembering as a class: *a delivery pipeline that filters by
`.gitignore` will silently drop source that matches an artifact pattern.*

### 1.3 The harness could manufacture regressions — and wins

While evaluating a candidate optimisation the harness reported
`REGRESSED adler32 +3.7%`. It was false. The change had touched only
`bench_setup`, which the driver calls **outside** the timed region.
Disassembling both binaries showed the `bench_run` bodies were byte-for-byte
identical; the function had merely moved from `0x1270` to `0x1260`.

A 16-byte shift is worth ~3.5% on this core. Any harness that does not control
for it is reporting instruction-fetch alignment, not codegen.

`bench_kernels.py` now fingerprints the **timed function's disassembly**
(address column and RIP displacements stripped) rather than the whole object,
and classifies an out-of-tolerance delta as `NOISE` when that fingerprint is
unchanged:

```
NOISE  adler32: 7.219 -> 7.461 ms (+3.4%) - kernel codegen is IDENTICAL
       (5a1f9cf7948258aa); this is layout/measurement noise
```

This establishes a **~3.4% layout-noise floor** for this host. Deltas below it
are unmeasurable here, which retroactively validates the previous session's
−35.3% `matchlen` win (far above the floor) and disqualifies any future
sub-3.5% claim made without a codegen change to back it.

The filter is symmetric and that is the point: it suppresses false *wins* as
readily as false regressions.

### 1.4 A candidate optimisation, measured and then cut

A `narrow_copy_fold` peephole was written to remove the redundant
`movl %eax, %esi` between a byte load and its compare (the byte-scan shape in
`memchr`/`strlen`/`longest_match`). It worked, with 10 tests including 7
negative controls, and two of those negative controls caught real bugs in it —
a `%si`/`%sil` prefix collision that corrupted register names, and a rename of
the architecturally-pinned `%cl` shift count that produced `shrq %r9b, %rsi`.

Then it was measured:

* **10 instructions removed across 220 corpus files (0.03%)**
* **0 instructions removed on 5 of the 6 real-workload kernels**
* no runtime effect above the noise floor

Sibling passes (`copy_propagation`, `dead_writes`, `relay_and_lea`) already
reach nearly every instance by another route. ~350 lines and a pipeline slot
for 0.03% is not a trade worth making, so **the pass was deleted rather than
shipped**. Recorded here so nobody rebuilds it.

### 1.5 §3.3 peephole audit: clean

Last session's cross-block fact leak in `redundant_ext.rs` (the directive skip
running before the label check) prompted an audit of every other peephole that
carries per-register state. Result: the arm / i686 / riscv passes classify
lines through a `LineKind` enum that distinguishes `Label` from `Directive`
correctly (riscv tests `ends_with(':')` *inside* the dot branch), and no other
x86 pass matches the shape. **`redundant_ext.rs` was the only instance**, and
it is already fixed. Bounded and closed.

---

## 2. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1395 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=559 FAIL=0 SKIP=15**, AB-diff 0 |
| `./scripts/ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3354 configs |
| `bench_kernels.py --baseline` | `classify` −18.7%; no regressions |
| `godbolt.py audit` | all pinned oracles current |

Current scoreboard, best-of-9, `-O3`, versus GCC (>1 = lccc faster):

| kernel | vs gcc | note |
|---|---|---|
| `memchr` | 0.47x | branch structure (§3.1) |
| `namechars` | 0.48x | counting form of the classifier (§3.2) |
| `varint` | 0.62x | branch layout on a serial chain |
| `classify` | **0.65x** | was 0.53x |
| `matchlen` | 0.66x | load-compare fusion + branch structure |
| `adler32` | 0.78x | |
| `hashmix` | 0.81x | |

geomean **0.626x**.

---

## 3. To do next, in priority order

### 3.1 Block layout: three branches per iteration where GCC/Clang have two

This is now the single largest remaining lever, and it is the *same* defect in
the four worst kernels. lccc emits:

```
.LBB1:  cmp ; jcc exit          <- top test
.LBB2:  body ; jcc exit2
.LBB4:  incr ; jmp .LBB1        <- unconditional jump back
```

Clang emits one conditional branch at the bottom and falls through. Per
iteration lccc executes 3 branches (2 taken) against Clang's 2 (1 taken).

Important: this is **not** the IR-level `loop_rotate` pass. That was measured
last session at 1.00x on five kernels and 0.87x on one — the gap is in backend
block ordering and branch inversion, downstream of the IR. Start with the
classic `jcc L1; jmp L2; L1:` → `jncc L2; L1:` inversion and with placing the
latch block on the fall-through path.

### 3.2 The counting form of the classifier

`if (pred) n++;` does not produce `Select`s — the hit edges converge on a
shared increment block before the join — so `range_fold` and `set_membership`
never see it and `k_namechars` stays at 0.48x while `k_classify` improved to
0.65x. The two kernels deliberately bracket this: same predicate, one shape
optimised and one not. Teaching `if_convert` (or `set_membership`'s matcher,
whose docstring already anticipates a `Phi [(Const(1), T_1), ...]` join) to see
through the shared hit block would close it.

### 3.3 Load-compare fusion

`movzbl (mem), %reg; cmpl %other, %reg` → `cmpb %other_b, (mem)` when `%reg`
dies at the compare. `memory_fold.rs` does this only for `-N(%rbp)` stack
slots, not SIB memory. `FileLiveness::live_after` supplies the liveness proof.
Worth an instruction and a register per iteration in every byte loop.

### 3.4 Def-dominates-use in the IR verifier

Unchanged from last session and still the top verifier item: the six existing
checks do not include dominance, which is how an SSA violation shipped past
every gate (see the previous follow-up, §1.9). `verify.rs` already computes
reachability; Cooper-Harvey-Kennedy dominators over RPO is the natural next
step. Size the backlog with `ir_verify_sweep.py` before committing to fixes.

### 3.5 Standing items still not advanced

- Linker oracle: honour lld 23.1 / mold 2.42 (X86+i686-only preset) / bfd 2.47.
- `lccc-bootstrap.sh` still lacks the rustup 1.98.0 reinstall step.
- Broader benchmark corpus: SQLite b-tree compare, kernel `memcpy`/checksum,
  zlib-ng CRC32; and a local Clang so the `vs clang` column stops being empty.

---

## 4. Notes for whoever picks this up

- **Measure before you ship, and be willing to delete.** Two candidate
  optimisations were built and validated this session; one was cut for paying
  0.03%. Instruction count is not runtime and neither is "it looks better".
- **Know your noise floor.** ~3.4% on this host, from code alignment alone. The
  harness now proves it per-kernel via the timed-function fingerprint; do not
  argue about a 2% delta.
- **Negative controls earn their keep.** Six of the ten tests on the cut pass
  asserted that a rewrite does *not* happen, and two of them found real bugs
  the positive test could not.
- **Use the oracle to find gaps, the IR dump to localise them.** The classifier
  gap was invisible in lccc's own output until `gcc16.2`/`clang 23.1`/`icx`
  were put beside it; it then took one IR dump to find the missing enum
  variant.
- A pass that is enabled and reached can still be doing nothing. `range_fold`
  had tests, was in the pipeline, and had never once fired on its headline
  input.
