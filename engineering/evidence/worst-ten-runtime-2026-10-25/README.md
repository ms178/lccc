# Worst-ten runtime work: scalar SQRT and masked-loop placement

## Measured outcome

Freshly ranked all **56** registered workloads against GCC 16.2 / Clang 23.1.3,
then froze the ten largest paired gaps for investigation. The ranking screen
has three rounds; it selects work, not statistically established regressions.
The final comparison uses **31 randomized paired rounds, three excluded
warm-ups, CPU 0, strict output checks, no removed outliers**, and the actual
default compiler (no layout override). All ten pass.

| Selected workload | After/before paired median [95% bootstrap CI] | After/fastest vendor |
| --- | --- | --- |
| nbody | **0.7015 [0.6854, 0.7170]** | 1.0484, Clang |
| hot_web_loop | **0.7471 [0.7368, 0.7660]** | 1.1013, Clang |
| gzip_longest_match | 1.0005 [0.9839, 1.0185] | 1.3225, Clang |
| byte_load_reassembly | 1.0024 [0.9788, 1.0111] | 1.2588, Clang |
| sqlite_varint | 1.0045 [0.9634, 1.0362] | 1.1167, GCC |
| expat_xml_scan | 0.9763 [0.9557, 0.9907] | 1.2569, Clang |
| chacha20_block | 1.0010 [0.9919, 1.0162] | 1.1947, Clang |
| switch_dispatch | 1.0055 [0.9910, 1.0179] | 1.1664, GCC |
| llvm_scalar_facts | 0.9977 [0.9881, 1.0069] | 1.0946, GCC |
| binary_trees | 0.9970 [0.9792, 1.0151] | 1.0806, GCC |

Lower is better. **nbody uses 29.85% less elapsed time**, 377.51 -> 264.49 ms;
**hot_web_loop uses 25.29% less**, 85.23 -> 63.86 ms. These reductions are one
minus the paired ratio, not speedup multipliers. nbody/GCC is
0.9946 [0.9885, 1.0015]: near parity, **not a demonstrated GCC win**.
hot_web_loop/GCC is 0.9627 [0.9513, 0.9833]. Neither beats Clang here.

**The other eight whole executables are byte-identical before/after.** Their
sample fluctuations are not optimizer improvements, even XML's nominally
non-overlapping interval. A separate 31-round control repeats XML at
0.9947 [0.9863, 1.0096], and identical glibc_memcmp at
0.9952 [0.9893, 1.0034]. This illustrates why intervals alone do not establish
code-generation causation. No claim that all ten were improved is intended.

The full 56-workload before/after/vendor screen passes all outputs. **53/56
entire ELF files are byte-identical**. Only nbody, hot_web_loop and spectral_norm
change. Spectral norm gains one copy at its final scalar square root; its loop
is unchanged. A separate 31-round check gives spectral after/before
0.9993 [0.9840, 1.0101], with no measured regression or new win. Its previously
merged loop improvement is not counted as progress here.

## Generic implementation and rejected alternatives

1. Legacy scalar SQRT merges old destination upper bits. Seed distinct XMM
   destinations using the existing full-register MOVAPS/MOVAPD helper, then
   perform the square root in place. This removes the loop-carried destination
   dependency without changing arithmetic, argument evaluation, rounding,
   exception or errno policy. Aliased registers and AVX's explicit-source form
   retain their existing spellings. No reassociation, reciprocal approximation,
   fast-math flag, benchmark-name test or input change is involved.
2. Recognize a narrow low-bit-mask zero/nonzero loop branch as a **frequency
   guess**, not a range/uniformity fact. A contiguous mask of 4..16 low bits
   supplies a rare-arm hint. Propagate cold placement only through a same-loop
   single-entry arm, stopping at joins and rejecting dominance backedges,
   including a function-entry self-loop. Existing chain construction and all
   semantic control flow are retained. No instruction is speculated or deleted.
3. Keep the existing total-stack veto except for this proven CFG shape, a
   **strictly lower modeled branch score**, and no increase in estimated
   hot-loop stack operands. An unmapped emitted label or no hot label fails
   closed; ties and other
   shapes retain the old rule. This static metric is not a formal proof of
   dynamic spill cost. All-zero inputs can reverse the frequency guess.

The actual default hot-loop assembly is byte-identical to the independently
measured forced-layout candidate. The modeled branch score falls 84 -> 79;
static total stack operands rise 9 -> 12, but estimated hot-loop operands stay
0 -> 0. A blanket stack-veto relaxation is deliberately not used. Return-copy
and widened-induction hot-loop experiments had intervals spanning parity and
were rejected. Broader LICM was not justified by must-execute/zero-trip/alias
proofs. gzip's helper was already inlined; byte-load induction already widened.
The remaining vendor gaps are open work, not claimed fixes.

## Diagnostics and correctness boundaries

- Native scalar-SQRT fixture: **16,480 result/exception comparisons per
  executable**, GCC/Clang/LCCC x SSE2/AVX/AVX2 = nine passing executions. Four
  rounding modes, signed zero, subnormal, infinite, negative and random inputs,
  quiet/signaling NaNs. Finite results are bitwise compared; NaNs by class, not
  payload. Memcheck: zero errors.
- Masked-loop fixture: all-zero, all-one, sparse and random populations over
  1,026 trip counts, 4,104 output lines matching GCC/Clang/LCCC. Memcheck: zero
  errors. These are correctness distributions, not distribution-wide speed claims.
- Warning-denied `fastbuild -j2` compiler build, targeted emitter/selection tests,
  all 16 block-layout tests, and ISA-emission gate pass. The final rebuilt
  compiler is byte-identical to the measured compiler. No local CI sweep was run.
- Latest-main GAS repair: targeted snapshot/cross-reference tests plus binutils
  2.47 strict audit: 1,638 rows, 224 encodings/160 mnemonics, zero findings.
- Bounded Callgrind profiles cover all ten before/Clang and all ten after/before,
  with exact flags, source/compiler/binary hashes, output checks and counter
  traces. Both indexes report complete with no failures. Final nbody Ir is
  826,715 / 816,719 = **1.01224x**; hot-loop Ir **1.01852x**; other eight **1x**.
  Runtime improves despite increased simulated instruction counts. Callgrind
  does not model physical cycles, dependency-breaking throughput or real branch
  predictor behavior; it is not offered as evidence of fewer native cycles.

## Environment and reproduction

Virtualized x86-64 Xeon, two logical CPUs, ~3.8 GiB RAM, **8 GiB active swap**.
GCC 16.2.0, Clang 23.1.3, Rust 1.97.0, mold 2.42.1, binutils 2.47.20260726,
Valgrind 3.19.0. Serialized native timing, then profiling/builds; no concurrent
heavy local job. PMU was not probed. These are VM wall-time results, not
bare-metal cycle claims or a universal target-tuning result. ICX is not a local
reference in this measurement set.

Preserved pre-change compiler SHA-256:
`068dd42a1f90f289a7a63ac1380241fa49963e86f58f78a91ef28adbe58aca6d`.
Measured/rebuilt compiler SHA-256:
`3f636a4e9d78e9744140c6fd3a92de4c7683ad57ec39e34f74cd868667da9aca`.
The baseline is the clean starting compiler, before either change. Full raw
commands, compiler versions, source hashes, flags, machine data and individual
rounds are in the JSON receipts. Canonical sources/sizes are unmodified.

With a preserved pre-change compiler at `$BEFORE`, current compiler built using
`RUSTFLAGS='-D warnings' cargo build --locked --profile fastbuild -j2 --bin lccc --bin lccc-ld`,
and the named modern toolchains on PATH:

```sh
python3 tests/benchmark/run_benchmarks.py \
  --only nbody,hot_web_loop,gzip_longest_match,byte_load_reassembly,sqlite_varint,expat_xml_scan,chacha20_block,switch_dispatch,llvm_scalar_facts,binary_trees \
  --compilers lccc,gcc,clang --gcc gcc-16 --clang clang-23 \
  --candidate before="$BEFORE" --reps 31 --warmup 3 --cpu 0 \
  --no-pmu-probe --strict --artifact-dir "$AUDIT/artifacts" \
  --json "$AUDIT/final.json" --markdown "$AUDIT/final.md"
```

For the full screen omit `--only` and use `--reps 3`; for the control use
`--only spectral_norm,glibc_memcmp,expat_xml_scan`. Set `$AUDIT` to a scratch
directory. For Callgrind use `scripts/callgrind_corpus.py` and the exact bounded
per-workload commands/flags in the manifests; bounded sizes are intentionally
not timing sizes. See `tests/benchmark/callgrind_sizes.json` for those sizes.

## Retained evidence

- `screen.json`, `worst-ten.json`: fresh ranking and frozen selection.
- `all-after.json`, `all-binary.json`, `all-text.json`: full screen and whole
  ELF/section identity proofs.
- `final.json`, `final.md`: final selected-ten raw rounds and generated report.
- `control.json`, `control.md`: independent 31-round tail/noise checks.
- `sqrt-choice.json`, `hot-layout-choice.json`, `hot-force-choice.json`:
  hypothesis experiments, explicitly separate from final default-compiler data.
- `hot-choice.json`: rejected return-copy/induction experiments.
- `final-*.txt.gz`: gzip-compressed, unedited targeted build, unit, native, Memcheck and ISA receipts.
- `profiles-diagnostic/`: mine=before, ref=Clang. `profiles-final/`: mine=after,
  ref=before. Both preserve manifests, compile/native/instrumentation receipts,
  gzip-compressed stdout/stderr/logs and original `.cg` files. Decompress to verify
  each manifest's original counter-file hash. Historical absolute paths remain
  in receipts; replace their original directory prefix with the retained profile
  directory when locating files. Executables themselves are not checked in.
