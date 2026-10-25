# Strict SSE2 computed-reciprocal reductions

## What changed

The existing exact computed-reciprocal matcher was restricted to AVX2 plus
SSE4.1. Plain x86-64 therefore evaluated one integer denominator, division and
product at a time even though SSE2 can evaluate independent pairs. The new
lowering reuses that matcher's complete legality proof, transactional integer
DAG planner and unchanged scalar tail, but evaluates four products with two
SSE2 packets. It is not a reciprocal approximation, reassociation, FMA or
benchmark-specific rewrite. No benchmark source, size or optimization flag
has been changed.

- Four signed I32 denominator lanes are converted as two pairs. SSE2 integer
  multiplication uses the existing PMULUDQ legalization, not forbidden PMULLD.
- DIVPD and MULPD preserve each scalar expression's operand order; ADDSD folds
  lanes 0, 1, 2, 3 in order. Products are staged in private scratch so the
  four scalar additions can consume independent low-lane memory operands.
- Both pairs are read before the result can overwrite the denominator
  register. Homed denominators bypass scratch; slot-only and scalar-DAG
  inputs also work. No allocator-owned XMM register is borrowed as scratch.
- That private scratch now needs only 16-byte alignment. MOVUPD/VMOVUPD do not
  require 32-byte alignment; removing needless over-alignment also avoids a
  dynamically reconstructed scratch pointer.
- User input may be only eight-byte aligned. It is explicitly loaded with
  MOVUPD: legacy MULPD m128 is **not** used on a possibly unaligned address.
- Resolved SSE2 **and x86-64** permission is mandatory. SSE1-only and i686 do
  not acquire these intrinsics. The existing AVX2/SSE4.1 spelling and its
  feature-denial fallback remain intact. FENV-sensitive functions retain the
  existing no-reordering refusal.

## Results and reproducibility

The accompanying raw native samples, compilation commands, binary hashes and
Callgrind profiles describe this exact implementation. Native runs use the
canonical runner, default workload sizes and `-O2`, with GCC 16.2.0 and Clang
23.1.3, CPU 0 pinning, 31 randomized paired rounds, three excluded warmups and
no discarded outliers. An 8 GiB swap file is active. There are no concurrent
heavy local jobs. This is a virtualized Xeon without a verified PMU, not
bare-metal cycle evidence.

The `before` arm is the preserved pre-runtime LCCC compiler, not a vendor
oracle. Main's intervening changes affect only the GAS reference, its tests and docs, so
its Rust compiler sources equal this baseline. Git metadata records the
pre-commit source stage honestly; the executable hashes identify measured arms.

| Workload | After/before paired runtime (95% CI) | Interpretation |
|---|---:|---|
| `fp32_product_map` | 1.0065 [0.9945, 1.0142] | unchanged machine code; no gain claimed |
| `fp64_product_map` | 1.0002 [0.9926, 1.0062] | unchanged machine code; no gain claimed |
| `i32_product_map` | 1.0010 [0.9840, 1.0193] | unchanged machine code; no gain claimed |
| `nbody` | 0.9979 [0.9853, 1.0177] | unchanged machine code; no gain claimed |
| `spectral_norm` | 0.7974 [0.7881, 0.8100] | runtime win |

Spectral norm: **20.26% lower paired runtime**, or **1.254x throughput**. Wall medians are 200.22 ms before and 158.23 ms after. The after/GCC ratio is 1.0087 [1.0032,1.0237]: still slightly slower, not a claimed GCC win. After/Clang is 0.8080 [0.8011,0.8227].

Bounded Callgrind: spectral norm **8,019,472 / 10,681,772 Ir = 0.750762** (24.92% fewer whole-program simulated instructions); nbody **816,667 / 816,667 = 1.0**. Both output checks pass; no profiling failures. All four native control `.text` sections are byte-identical to their before arms (`control-text.json`).

The selected rebuilt compiler matches the measured executable byte-for-byte: SHA256 `310c9aef0fdd13082425884824678aa2e8cacf8faae768dffdc09bd81f49b68d`. The rebase did not change that result. The separate `lowering-choice.json` retains the slower streaming experiment and its directly paired selected `spill` arm; rejected variants are not relabelled as the final compiler.


## Adversarial checks

The existing native differential harness is strengthened rather than replaced:
all four rounding modes; signed-wrap I32 boundary biases; cancellation; signed
zero; infinities; raw quiet/signaling NaNs; smallest subnormals; lengths -3..65;
every scalar tail; eight-byte-unaligned inputs ending at an inaccessible page;
and a completely inaccessible pointer for zero/negative trips. Finite results
are compared bit-for-bit, NaNs by class, and masked exception flags exactly.
GCC and Clang independently supply reference objects. Normal, XMM-disabled,
all-register-disabled, VEX-promotion-disabled and scalar-denominator modes run
for SSE2 and AVX2; the existing i686 runtime arm also remains exercised.

Two focused emitter tests cover denominator/result home aliasing, high XMM
homes, exact scratch/add order and the scalar-DAG no-home case. Structural
gates require the new SSE2 conversion/division/multiply packets, disallow a
packed-add reassociation or post-SSE2 instructions, retain the AVX2 checks,
and verify the SSE1-only F64 diagnostic. The existing ISA gate also passes.

An independently GAS-assembled correct program passes the same native witness.
Reversing its first packet's addition order is rejected by the witness under
downward rounding; the failure is a value-bit mismatch, not a compile error.
Valgrind Memcheck reports no errors for the correct witness.

The explicit FENV_ACCESS unmasked-trap witness returns the required overflow
trap for LCCC and Clang. GCC returns the later divide-by-zero trap instead,
even with explicit trapping/rounding flags: it is not used as the oracle for
that pragma-sensitive test. This is distinct from the masked-flag differential
where GCC and Clang both agree with LCCC. No unmasked-trap-order guarantee is
claimed for functions compiled without the existing FENV protection.

## Experiments rejected, scope kept honest

Increasing the FP unroll budget made nbody slower (roughly 367 ms at 256 versus
419–430 ms at 512–2048 in the initial screen). Direct SSE staging-copy removal
also failed to establish a runtime win across nbody and six other workloads.
Neither experiment is shipped. A further register-streaming experiment
removed product stores but was 1.0659x slower [1.0473,1.0723] than the selected
scratch-based lowering in a separate 31-round paired comparison. It is also
rejected: fewer stores are not proof of better runtime. This change targets the demonstrated missing
SSE2 reciprocal path instead; it does not claim to solve nbody or every runtime
gap. Callgrind uses reduced sizes and `-fwrapv`, and its simulated instruction
counts are **not** native cycles or default-size runtime ratios.

## Commands

From the repository root, with modern tools on PATH and a preserved pre-change
compiler in `$BEFORE`:

```sh
CCC=target/fastbuild/lccc GCC=gcc-16 bash tests/regression/check_strict_packed_dag.sh
CCC=target/fastbuild/lccc bash tests/regression/check_strict_computed_recip_codegen.sh
CCC=target/fastbuild/lccc bash tests/regression/check_vectorize_isa_gate.sh
python3 tests/benchmark/run_benchmarks.py --only spectral_norm,nbody,fp32_product_map,fp64_product_map,i32_product_map --compilers lccc,gcc,clang --gcc gcc-16 --clang clang-23 --candidate before="$BEFORE" --reps 31 --warmup 3 --cpu 0 --no-pmu-probe --strict --json runtime.json
python3 scripts/callgrind_corpus.py --mine "$PWD/target/fastbuild/lccc" --ref "$BEFORE" --gcc-include-command gcc-16 --target-flags=-march=x86-64 --only spectral_norm,nbody --out callgrind
```

The native witness ran 18,216 comparisons per configuration, 11 configurations
per vendor reference (including i686), **400,752 comparisons total**. The
seeded SSE2 bit-output witness adds 1,260 identical output lines per arm across
GCC, Clang and four LCCC home/planner configurations. This is targeted local
work; full integration/Clippy is delegated to GitHub CI, not a local CI run.
