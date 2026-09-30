# Captured kernel assembly for the loop-density metric

`scripts/test_hot_loop_metric.py` runs offline known-answer cases, and this
directory adds one REAL cross-compiler pair to it: the same kernel compiled by
LCCC and by the pinned GCC 16.2 oracle, kept here so the regression cannot be
satisfied by synthetic shapes alone.

| file | provenance |
|---|---|
| `matmul.lccc.s` | `target/fastbuild/lccc -O2 -march=x86-64-v3 -S tests/benchmark/programs/matmul.c` |
| `matmul.gcc162.s` | Compiler Explorer `cg162` (x86-64 gcc 16.2), flags `-O2 -march=x86-64-v3` |

Why `matmul`: it is the case that exposed both defects the metric used to have.

* The old "branch to a textually earlier label" rule picked a composite cycle
  (the middle `k`-loop, whose body contains the packed inner loop) and charged
  its 24 static instructions against a 2048-byte trip, reporting 0.0117
  insn/byte for a kernel whose real steady-state cost is 0.1172 — ten times
  too good, and enough to invert the LCCC-vs-GCC verdict.
* The old step recovery only recognised a pointer advance on a register that
  appears in a memory operand. LCCC's packed reduction advances a separate
  index (`addl $128, %r8d` with `movslq %r8d, %r9` for the address), so the
  packed loop measured as "step unknown" and the tool silently reported the
  scalar remainder instead — making a vectorized kernel look scalar.

Correct numbers (steady state, innermost non-composite loop):

| compiler | insns | bytes/trip | insn/byte |
|---|---|---|---|
| LCCC | 15 | 128 | **0.1172** |
| GCC 16.2 | 6 | 32 | 0.1875 |

Refresh with:

```sh
python3 scripts/codegen_oracle.py --rank tests/benchmark/programs/matmul.c \\
    --flags '-O2 -march=x86-64-v3' --artifact-dir /tmp/matmul
```
