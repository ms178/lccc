# Vector types: measured first tranche, 2026-10-24

This is a first implementation tranche, not an exhaustive vector-type rewrite
or evidence of superiority over all compilers. The baseline is main commit
`c18b2fccb5cd5b65d6a851444333607d961da864` (the merged sign-fold work).

## Reproducible baseline

Compile `tests/regression/vector_ext/matrix.c` with:

- LCCC fastbuild: `-O2 -mavx2 -mfma`;
- GCC 16.2.0: `-O2 -mavx2 -mfma`;
- Clang 23.1.3: `-O2 -mavx2 -mfma -ffp-contract=fast`.

Count non-label, non-directive, non-comment instruction lines between each
function's label and `.size`, including `ret`. There are 377 common functions;
these are not runtime rankings. Totals:

| compiler | instructions |
| --- | ---: |
| LCCC baseline | 2583 |
| LCCC with packed FMA prefix coalescing | 2582 |
| GCC 16.2.0 | 2441 |
| Clang 23.1.3 | 2133 |

Only `fma_chain` changes: **4 -> 3**, matching both reference compilers. The
whole-matrix reduction is one instruction, not a sweeping vector improvement.
The executable runtime matrix remains the correctness authority; matching
static totals does not establish equivalent optimizations or speed.

## Implementation and proof boundary

Baseline:

```asm
vmovaps      %ymm1, %ymm14
vfmadd132ps  %ymm0, %ymm2, %ymm14
vfmadd231ps  %ymm3, %ymm14, %ymm0
ret
```

New:

```asm
vfmadd132ps  %ymm0, %ymm2, %ymm1
vfmadd231ps  %ymm3, %ymm1, %ymm0
ret
```

The transformation renames a dying copied register in the producer and its
adjacent FMA consumer. It does not exchange inputs, alter instruction forms,
contract separate arithmetic, change precision, or speculate memory reads.
Consequently the proof does not need to normalize NaN payloads: each original
instruction receives the same bits in the same operand roles.

Accept only VEX full-width copies and register-only, unmasked packed ps/pd
FMAs, at equal 128/256-bit widths, in a straight-line window. Refuse labels,
transfers, calls, inline assembly, masks, memory operands, scalar operations,
EVEX-width operations and any consumer reference to the original source.

The existing CFG liveness oracle must prove the source dead after the producer
in low AND upper architectural regions, and all removed temporary writes dead
after the consumer. Unknown liveness refuses. Invalidate cached liveness after
each rewrite. Use a CFI index, rather than rescanning an entire function for
every copy candidate; the pass never inserts lines or mutates CFI delimiters.
The existing `vector_copy_fma` disable group owns the transformation.

## Tests and performance interpretation

- Structural matrix: 2 widths x 4 sign families x 3 forms x 2 element types =
  48 accepted shapes, checking exact instruction forms and operand order.
- Negative tests: live old source, live temporary, upper-only observations,
  branch-reachable reads, memory operands, masking, width mismatches, scalar
  forms, labels, calls and a consumer needing the old source.
- Full-pipeline disable test: the copy returns when `vector_copy_fma` is off.
- Native exact comparison: 4 vector types x 4 rounding modes x 4096 inputs =
  **65536** before/after comparisons. Check result bits AND exception flags;
  include signed zeros, subnormals, infinities, quiet/signaling NaN payloads
  and randomized raw bit patterns. Structural tests always run; native
  execution explicitly reports an ISA skip on hosts without AVX2/FMA.
- Existing whole-vector gate: 11 runtime/ISA levels passed, including its
  FMA contraction policy checks. Tighten `fma_chain`'s ceiling from 4 to 3.

`benchmark_packed_fma_prefix.py` measures a call-inclusive dependent f32x8
recurrence, pins one allowed CPU, alternates A/B order, warms up, verifies
outputs and retains every timing sample. The retained quiet run uses 21 pairs
of 100 million iterations: median before/after **0.99685**, ratio MAD
**0.00388**, range **0.98544..1.01411** on a virtualized Intel Xeon host.
This does **not** demonstrate a speedup or a meaningful slowdown. Earlier runs
while compilation competed for CPU were much noisier and are not used as
performance evidence. No PMU tool is installed in this host. Static code size
is improved; throughput/latency superiority is not claimed.

## Contemporary sources and limits

Source snapshots obtained through the GitHub API on this date:

- GCC master `590068c52cb2b1bb4a30ad01834f369e8e628671`, `gcc/BASE-VER`
  reports `17.0.0`. `gcc/config/i386/sse.md` describes constrained 132/213/231
  FMA alternatives. This is source inspection, not a GCC-17 binary comparison.
- LLVM main `2b6e769dd3add8bbd8d8be60e59f8d7e5f30ec54`,
  `llvm/lib/Target/X86/X86InstrFMA.td` documents form-changing commutation.
  LCCC deliberately does not use that extra freedom in this transformation.
  No binary built from this LLVM revision was measured.
- Intel SDM index, updated 2026-10-09:
  https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html
  lists SDM revision 093. The optimization-volume index and individually
  indexed October PDF revisions disagree (050 versus 051); do not silently
  combine them into an alleged single pinned edition.
- AMD APM volume 4, publication 26568, revision 3.27, 2026-07-21:
  https://docs.amd.com/v/u/en-US/26568_3.27_APM_Vol4_PUB . Its public viewer
  exposed publication metadata, not a complete searchable instruction body.
  This is not a claim to have audited all 2026 Intel/AMD instruction pages.

No new ISA feature, width-selection policy or CPU cost number is inferred from
publication dates or vendor marketing. A full instruction-reference and
microarchitecture audit remains separate work.

## Next tranches, in priority order

1. **Incoming MEMORY-class parameter homes.** Re-measure the wrapper, volatile,
   phi and call shapes identified in the earlier ext-vector follow-up. Model
   an incoming stack object explicitly; do not substitute its address for a
   realigned local when declared alignment exceeds the caller's guarantee.
   Preserve parameter-object identity, address escape, unwind information and
   stable addressing under dynamic stack realignment. Run cross-compiler
   by-value/varargs matrices and perturb incoming stack residues.
2. **Uniform dynamic vector shifts.** Distinguish a scalar count from per-lane
   counts in typed IR. Select packed scalar-count instructions rather than a
   broadcast plus variable-lane shift, including SSE2 word lanes. Keep shift
   bounds, scalar integer promotions, signed-right-shift semantics and target
   feature legality explicit. Do not reinterpret repeated low lanes as a
   scalar count register.
3. **First-class vector/mask SSA.** Separate semantic lane shape, storage
   layout, ABI carrier and machine legalization. Keep padded ext-vectors,
   packed bool masks and x87 lane store/alloc stride distinct. Do not replace
   all pointer-backed vectors at once: addressable/volatile/escaping objects
   need deliberate memory materialization boundaries.
4. **Target-aware costs and scheduling.** Compare 128/256/512-bit strategies
   with spill counts, dependency latency, load/store traffic, code size and
   measured cycles. Intel and AMD require distinct cost data; widest legal
   is not automatically fastest. Add Arm/RISC-V coverage before generalizing
   target-independent IR rules from x86 observations.

These items are an evidence-gated implementation plan, not completed features.

## Final local verification for this tranche

- `scripts/build_lccc_fast.sh`: clean, warning-denying build, Rust 1.97.0,
  two jobs, active 8 GiB swap.
- `cargo test --profile fastbuild --locked -- --test-threads=2`: 6164 library
  tests passed, 10 ignored; the linker binary's additional test passed;
  three existing doctests ignored, no failures.
- Strict all-target Clippy with fastbuild and `-D warnings`: passed.
- Python harness contracts: three passed; CI parity: 377 commands / 27
  invocation contracts; audit census held without baseline increases.
- Full vector-extension runtime/codegen/contraction gate: passed all eleven
  execution levels; the packed-FMA ceiling is now three instructions.

Retained evidence is under `engineering/evidence/vector-types-2026-10-24`.
The small `before.s`/`after.s` files contain the exact kernels; timing also
records hashes of the full input assembly snapshots and extracted kernels.
To rerun only the native timing experiment:

```sh
python3 scripts/benchmark_packed_fma_prefix.py \
  --before engineering/evidence/vector-types-2026-10-24/before.s \
  --after engineering/evidence/vector-types-2026-10-24/after.s \
  --cc gcc-16 --iterations 100000000 --samples 21 --out /tmp/fma-ab.json
```
