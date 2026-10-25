# SSE1-only packed F32 maps

## Change and boundary

SSE1 permission is independent of the existing SSE2-and-newer lowering ceiling.
The main and early constant-trip vectorizer entries admit only F32 map trees
made of loads, invariants, add/subtract/multiply/divide and square root. They
reuse the established alias guards, scalar tails, FP-environment checks and
strict operation ordering. Integer/F64 maps, reductions, comparisons, rounding,
FMA and other vector families are not newly admitted. Explicit `-mno-sse2`
remains effective in either ordering with `-msse`.

Raw copies use MOVAPS/MOVUPS when SSE2 is absent. Scalar conversion zero idioms
use XORPS; stack-homed F32 arguments use MOVSS plus a high-word zero store,
preserving the existing eight-byte zero-extended home. Store forwarding must
not turn legal memory transfers into SSE2 MOVD/MOVQ cross-domain transfers.
The ordinary SSE2 paths retain their prior spellings.

The address-space review found a related existing defect: the early tiny-trip
entry lacked the main entry's named-space veto. An eight-element GS-relative
map became a default-space load and faulted (`named-before.txt`). Both entries
now refuse that transformation. Native GS-relative execution checks both the
SSE1-only and default SSE2 profiles.

This advances MB-01's observable feature contract and UP-7's evidence work;
it does not close either broad item or the remaining register-class audit.

## Paired runtime screening

Original strict-FP cache-resident multiply/add map: 512 floats, 4,194,304 passes,
input mutation each pass, output byte hash **5612d63b3564ff24** in every arm.
All compilers receive `-O2 -mno-sse2 -msse -ffp-contract=off`. GCC 16.2 and
Clang 23.1.3 use the modern GCC installation and binutils 2.47.20260726.
The compiler and linker were built warning-clean with fastbuild, two jobs,
Rust 1.97 and enabled swap.

| Comparison | Candidate/other paired median | 95% bootstrap CI |
|---|---:|---:|
| Legal scalar LCCC baseline | 0.255732 | [0.251539, 0.261082] |
| GCC 16.2 | 1.045411 | [1.023129, 1.056257] |
| Clang 23.1.3 | 1.021285 | [0.991558, 1.088467] |

This is **3.91x** the scalar baseline's throughput, not a claim of universal
optimality: GCC is still about 4.5% faster in paired ratios, and the Clang
interval crosses parity. Medians: candidate 263.58 ms, scalar 1,037.28 ms,
GCC 254.06 ms, Clang 260.00 ms. All timed runs exceed 249 ms. Fifteen randomized
paired rounds, two excluded warm-ups, CPU 0 pinning, no discarded samples.
MAD-flagged observations remain in the report. Virtualized Xeon, PMU not probed:
these are screening data, not bare-metal microarchitectural claims.

Unmodified main cannot supply a runtime baseline: its SSE2 PXOR zero idiom
triggers the final ISA fence under `-mno-sse2`; a conversion-only repair then
exposed MOVAPD register copies. The frozen scalar arm repairs these legality
prerequisites without enabling vectorization. `scalar-legalization.json` (embedded unified diff)
records those prerequisite changes. The intervening main updates were confined
to i686 and CI tooling, not the x86-64 optimizer/emitter under comparison.
Binary hashes in `identity.json` and `performance.json` identify the actual arms;
the runner's revision field records the checkpoint before the named-space
follow-up, not a claim that its binary was built from a clean checkpoint.
No local ICX timing is claimed. The authenticated hosted ICX integration lane
compiles this SSE1-only workload in strict FP mode and compares its output
with LCCC; the sharded timed benchmark lane uses GCC.

The retained disassemblies show two four-lane MULPS/ADDPS copies per hot loop,
unaligned-safe loads/stores, no FMA, and no SSE2 SIMD instructions in the
SSE1 kernel. No benchmark-name dispatch or benchmark-specific optimizer policy
was added.

## Focused checks

- New native gate: **40 arms x 606 = 24,240** exact-bit cases against GCC,
  including aliases, negative/zero/odd lengths, page ends, signed zero,
  subnormals, overflow/cancellation, a live scalar F32 parameter, flag ordering,
  O0/O1/O2/O3/Os, scalar ablation and disabled register allocation.
- Every kernel assembly is checked against an SSE1 SIMD allowlist; the integer
  control must contain no XMM use. Six forbidden-instruction mutations are
  rejected; denied F64/SSE1 features still diagnose.
- Forced-scalar compiler wrapper is rejected by the normal packed-code arm
  (`scalar-mutation.txt`); scalar ablation is not a vacuous passing test.
- Two GS-relative native controls preserve named addressing; the pre-fix
  kernel faults with SIGSEGV at the incorrect default-space address.
- Seven focused map policy units and one pure ISA-copy policy unit pass.
  The latter deliberately avoids racing the process-global ISA in unit tests.
- Inherited strict F32/F64 gate passes all 40 arms, including contraction
  mutants; vectorizer ISA and default-feature gates pass.
- Twelve deterministic differential fuzz cases pass against GCC/Clang at O2.
- Debug/size parity now reads benchmark build profiles instead of compiling
  every source with a fixed ISA. This preserves the SSE1-only assertion and
  also covers both registered F32/F64 builds; the focused three-profile check
  passes (`debug-parity.txt`). Raw diagnostic TUs retain default coverage.
- The benchmark's ISA assertion is scoped to its registered
  `SSE1_MAP_REQUIRE_ONLY` profile so generic SSA/cross-target corpus consumers
  can build the ordinary portable C workload too. It still rejects SSE2 when
  that profile is requested (`benchmark-assertion.txt`), and the focused SSA
  corpus case passes. `timed-source.c` retains the measured source; all four
  recompiled binaries are byte-identical after this guard-only change
  (`source-guard-identity.json`). The native kernel keeps its unconditional
  SSE1-only assertion.

Commands and raw samples are retained in the JSON and text files. Absolute
artifact/tool paths record this sandbox and are not claimed to exist elsewhere.
Broad regression, all-target and reference-compiler coverage belongs to hosted
CI rather than an expansive local CI run. Full manual auditing remains open.
