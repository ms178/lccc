# Expat XML scan — acyclic static-leaf inline-budget prototype

**Status:** `prototype` — implemented and validated in the repository working
tree, but not a bare-metal/Raptor Lake conclusion or a release-performance
claim yet.

## Reproducer

```bash
./scripts/build_lccc_o1_j2.sh

# Explain the inliner decision and retain generated-code evidence.
GCC_INC="$(gcc -print-file-name=include)"
CCC_INLINE_DEBUG=1 ./target/release/lccc -I"$GCC_INC" -O2 \
  -o /tmp/expat_lccc tests/benchmark/programs/expat_xml_scan.c
objdump -drwC /tmp/expat_lccc | less

# Controlled LCCC/GCC screening measurement.
python3 tests/benchmark/run_benchmarks.py \
  --only expat_xml_scan --compilers lccc,gcc --reps 15 --warmup 2 \
  --cpu auto --artifact-dir /path/to/artifacts \
  --json /path/to/results.json --markdown /path/to/report.md
```

- **Source:** `tests/benchmark/programs/expat_xml_scan.c`
- **Origin:** Expat 2.8.2 `lib/xmltok_impl.c`; see
  [`tests/benchmark/WORKLOAD_PROVENANCE.md`](../tests/benchmark/WORKLOAD_PROVENANCE.md).
- **Generated-code flags:** `-O2` for LCCC and GCC.
- **Compiler self-build policy:** Rust opt-level 1, exactly two Cargo jobs.

## Baseline evidence

The initial LCCC/GCC randomized, CPU-pinned VM screen used 15 timed paired
rounds and 2 excluded warm-ups (seed `20260810`) on a KVM-exposed Intel Xeon
2.60 GHz vCPU.  `perf` was unavailable, so this contains **no PMU claims**.

| Metric | Baseline LCCC | GCC |
|---|---:|---:|
| Median wall time | 160.48 ms | 40.16 ms |
| LCCC/GCC paired median | 4.006 | — |
| Paired bootstrap 95% interval | [3.993, 4.040] | — |

The deterministic output was identical:

```text
626766774715194881
```

## Root cause — confirmed at the compiler-decision level

The original hypothesis was that a hot static predicate was failing to inline.
The baseline `CCC_INLINE_DEBUG=1` trace identified `xml_name_start` as skipped;
a pre-inline IR dump then measured **13 blocks and 32 instructions**.  The
active `MAX_INLINE_BLOCKS_NO_LOOPS` constant was 12, while the static
instruction budget was 96.  The debug output has now been extended so future
skips expose the same diagnosis directly:

```text
[INLINE_DEBUG] <name> skipped: is_static=..., is_inline=...,
  blocks=<n>, inst_count=<n>, has_loops=<bool>, medium_block_limit=<n>, ...
```

Thus the source-level boolean expression is small but lowers to **13 acyclic
blocks**.  The old no-loop cap excluded it solely on CFG shape.  By contrast,
`xml_name_continue` (9 blocks, 22 instructions) was admitted.

The baseline generated code consequently contains a hot call in
`expat_scan_document`:

```asm
401492: mov  %rbp,%rdi
401497: call 4010f0 <xml_name_start>
...
4014b3: call 4011ca <expat_utf8_name_length>
```

GCC’s scan loop has no call to `xml_name_start`; it folds the classification
into compare/range/bit-test instructions.  GCC still keeps the larger UTF-8
name scanner outlined, so the finding is specifically **not** “inline
everything.”

## Prototype

`src/passes/inline.rs` now raises the acyclic no-loop block cap from 12 to 16:

```rust
const MAX_INLINE_BLOCKS_NO_LOOPS: usize = 16;
```

The bound is accompanied by:

- an inliner unit regression test for a 32-instruction, 13-block acyclic static
  leaf; and
- richer `CCC_INLINE_DEBUG` skip output including block count, instruction
  count, loop status, and active limit.

With the prototype, the inliner reports:

```text
[INLINE] Inlined 'xml_name_start' into 'expat_scan_document'
```

and the generated binary has no residual call to `xml_name_start`; it retains
only calls to the intentionally larger `expat_utf8_name_length` and
`expat_scan_document` helpers.

## Controlled A/B result

A direct 21-round randomized A/B/A comparison pinned to CPU 0 compared binaries
produced by the 12-block baseline and 16-block prototype.  Both output the
same checksum; GCC was included as a stable reference but is not used to
calculate the prototype gain.

| Metric | Inline cap 12 | Inline cap 16 | Prototype / baseline |
|---|---:|---:|---:|
| Median wall time | 161.763 ms | 131.412 ms | **0.8164** |
| Speedup | — | — | **1.225×** |
| `.text` bytes | 2,459 | 2,624 | +165 (+6.7%) |
| Pair-ratio CV | — | — | 3.07% |

This is a 22.5% VM-screening improvement for the target kernel.  It narrows
but does not erase the remaining LCCC/GCC gap, which is expected because the
larger scanner and enclosing scan function remain outlined.

## Breadth / regression guard

The source change passed the following gates:

- **Compiler unit tests:** 575 passed, 6 ignored (`cargo test --release` under
  the required opt-level-1/two-job compiler build policy).
- **Benchmark correctness:** all 24 canonical benchmarks produced matching
  LCCC/GCC output at `-O2`.
- **New-kernel sanitizer gate:** all six workload-derived kernels passed GCC
  ASan+UBSan; LCCC and GCC matched at `-O0`, `-O1`, and `-O2`.
- **Differential fuzzing:** 300 general cases (seeds 0–99 at `O2,O3,Os`) and
  200 CFG/phi cases (seeds 0–99 at `O2,O3`) all matched GCC.
- **Six workload-kernel A/Bs:** 15 randomized paired rounds each gave a
  geometric-mean prototype/baseline ratio of **0.9657**.  Only
  `expat_xml_scan` changed materially (0.8161); no other kernel regressed by
  more than 0.13% median.
- **Ten representative synthetic A/Bs:** 11 paired rounds each gave geometric
  mean 1.0021.  The largest apparent slowdown was `strlen_bench` at 1.0165,
  but its generated `.text` was byte-identical and the paired-ratio CV was
  3.05%; it is logged as a VM-noise investigation item, not hidden or declared
  harmless.

Raw JSON, compiler binaries, commands, disassemblies, and sample sequences
were retained outside the source tree under `bench-evidence/inline-prototype/`.

## Remaining acceptance conditions

1. Repeat on the i7-14700KF with controlled P-core affinity, governor/thermal
   records, and PMU evidence (`cycles`, `instructions`, IPC, branch misses,
   frontend/backend Top-Down metrics).
2. Build and run a real Expat parser workload plus at least one unrelated
   parser/string-heavy package to verify the cost model generalizes.
3. Measure compile-time and `.text` impact on larger package TUs.  Reject or
   refine the 16-block policy if it causes a reproducible >1% regression or
   unacceptable code growth.
4. Only then promote this from `prototype` to a fully integrated optimization
   decision in the hotspot lifecycle.
