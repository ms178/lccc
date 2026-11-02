# struct_copy — aggregate by-value ABI & copy lowering (largest measured gap)

Status: `screening` — measured VM regression; not a hardware claim.

Source / workload revision: `tests/benchmark/programs/struct_copy.c` (returns
`Particle` and `ParticleGroup` by value, ~200-byte aggregates, 2M iterations).

LCCC revision and build mode: `ms178/lccc` main `77b4a18c`, `cargo build
--release` (Rust opt-level 1, two jobs).

Reference compiler(s), versions, flags: GCC 14.2, `-O2`.

Reproducer command:
```bash
python3 tests/benchmark/run_benchmarks.py --only struct_copy \
  --compilers lccc,gcc -O2 --reps 9 --work-dir /tmp/bench
```

Correctness evidence: output `struct_copy total: 5600000.00` identical for LCCC
and GCC.

Measurement environment: KVM-exposed Intel Xeon 2.60 GHz vCPU, pinned, 9 paired
rounds. Screening evidence only (no PMU).

Raw result location: `/tmp/bench_lccc.json` (LCCC 585.5 ms, GCC 27.9 ms,
ratio 21.06×).

Assembly observations:
- LCCC emits **137 `mov`-class ops** for the whole function vs GCC's **29**.
- LCCC lowers each by-value struct copy field-by-field through the accumulator
  register: `movq %xmm0,%rax; movq %rax,%xmmN` register churn for every `double`
  field, plus a separate `movslq`/store per scalar field.
- GCC emits wide memory moves / block copies and passes aggregates in SSE
  registers, so the per-iteration struct churn is a handful of instructions.

PMU observations: none available (explicitly not a hardware claim).

Hypothesis (falsifiable): the by-value ABI and aggregate-copy path is the
dominant cost; a wide/vectorized copy (a few 128/256-bit moves) plus in-register
aggregate passing should remove the per-field accumulator round-trips and close
most of the 21×.

Candidate optimization:
1. **Aggregate copy → block/vector copy**: recognize a multi-field store/load
   sequence for one aggregate and lower it to packed `movups`/`vmovdqu` + a
   couple of scalar tails, instead of per-field `movq %xmm,%rax; movq %rax,%xmm`
   chains.
2. **By-value struct ABI**: pass/return aggregates that fit in SSE/GPR registers
   in registers (SysV: up to 16 bytes in two registers; return in rax/rdx or
   xmm0/xmm1) rather than on the stack.
3. **Sret return**: return large aggregates via a hidden result pointer and copy
   in the caller with a single wide copy.

Microbenchmark / real-workload validation plan: A/B `struct_copy` and add struct
by-value kernels; confirm no regression on the broad suite; then a real package
with struct-heavy code (e.g. a VM/emulator or vector-of-structs workload).

Risk and invalidation conditions: must preserve the exact C ABI for external
calls and struct layout observability; a wrong wide-copy could misalign or
overwrite padding. Reject if a >1% regression appears on the suite or if code
size grows unreasonably.

Result / next decision: screening. This is the single largest measured
LCCC-vs-GCC gap; prioritize after the PGO/docs patch.
