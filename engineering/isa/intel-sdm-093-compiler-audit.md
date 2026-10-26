# Intel SDM edition 093: compiler-wide SSE/AVX cross-reference

**Primary source:** Intel combined manual Volumes 1–4, edition 093, requested
[via document ID 671200](https://cdrdv2-public.intel.com/v1/dl/getContent/671200),
which resolves to the combined PDF. Source SHA-256:
`a4a62e6a7ba11a76c7753a195b825087306812aac39b930973f9168ee599f321`.
The hosted acquisition record has the complete 5,364-page outline and page-text
SHA-256 receipts. `engineering/isa/intel-sdm-093-section-ledger.json` lists
105 candidate manual sections with the pages actually extracted (selected
instruction descriptions up to 16 pages; architecture excerpts up to 4 pages),
plus explicit cap markers. This is a **section locator and bounded excerpt
review, not a claim to have read all 105 sections or signed off all SSE/AVX
semantics**. The full source PDF and extracted page contents are not committed.

## Reproducible audit map

The machine-readable locator was emitted by
`scripts/audit_intel_sdm.py` and hosted Check Run “Intel SDM source pages
(unreviewed)”. The requested URL's current redirect target and edition/hash
are recorded there. To reacquire from an allowed network environment:

```sh
python3 scripts/audit_intel_sdm.py --self-test
python3 scripts/audit_intel_sdm.py --output /tmp/intel-sdm \
  --sha256 a4a62e6a7ba11a76c7753a195b825087306812aac39b930973f9168ee599f321
```

The source pages were cross-referenced by compiler responsibility, rather
than treating opcode tables as the whole compiler:

| SDM scope | Compiler ownership / review point | Acceptance evidence and uncovered work |
|---|---|---|
| Vol. 1 §§4.2–4.9, 10.2–10.4, 11.2–11.6, 12.8, 14.10–14.12, 15.5–15.11: data representation, MXCSR, FP exceptions, alignment, transitions | C type/layout and cast lowering; optimizer FP policy; volatile/alias/memory semantics; function ISA and state cleanup | 16/32-byte bitcast regression; existing exceptional FP runtime oracle. FTZ/DAZ combinations, unmasked traps, memory-fault/guard-page behavior and all instruction exception details remain open. |
| Vol. 2A instruction entries and exception classes: arithmetic, loads/stores, conversions, comparisons, shuffles, scalar merge forms | Sema feature permissions; builtin types/arity; IR operations and lane semantics; alignment and memory-fold legality | Every listed locator is not yet individually audited against implementation/tests. Section extraction cap is marked in the JSON ledger. |
| Vol. 2B/2C/2D opcode maps and encoding tables: legacy, VEX, EVEX, tuple/broadcast/masking, CPUID requirements | Feature resolver; target attributes; instruction selector; AT&T emitter/integrated assembler; disassembler/object validation | AVX vs AVX2 vs individual AVX-512 subset conjunctions and complete FMA feature classification remain backlog work. Existing VEX-promotion unit matrix and GCC/GAS differential do not prove all encodings. |
| Vol. 2A addressing and instruction exception classification; Vol. 1 §§14–15 alignment tables | GEP/address formation; ModRM/SIB/displacement selection; relocations; stack alignment; vector memfold | Current unaligned MOVUPS/MOVUPD/MOVDQU forms are distinct from aligned MOVAPS/MOVAPD/MOVDQA. Guard-page tests for memory-source fault suppression and masked operations remain open. |
| Vol. 3A/3B/3C/3D state, OS-enabled vector state, exceptions and memory ordering | Driver `-march`/per-function target; ABI/XSAVE and call boundaries; linker, TLS and atomics | Architectural OS/XSAVE gates, every call ABI direction and all linker relocation/runtime interactions are not established by the SIMD helper fixture. |
| Vol. 4 MSR state relevant to vector/FPU enablement | Runtime/loader/OS contract | No compiler sign-off is claimed; hardware-state setup belongs to runtime/platform evidence. |

## Direct cross-reference observations

* **A vector cast is a register-image operation, not a numeric conversion.**
  Vol. 1 §4.6.2 (printed page 4-9; PDF page 99) defines the fundamental
  packed image and its multiple numeric interpretations. Intel's 128-bit cast
  intrinsics are zero-cost reinterpretations. The prior bundled cast helpers
  used `memcpy`, forcing an unaligned temporary and a memory-backed boundary.
  They now return same-width native vector casts. In the controlled 64-bit SSE2
  `cast128` function, the saved compiler emitted a 120-byte frame and six
  vector moves after its input load; the candidate emits one XMM load, one XMM
  store and `ret`. The direct store replaces the old final copy, for seven
  fewer total instructions and no frame. This is instruction/frame evidence
  for that fixture, not runtime-speed evidence. New 256-bit cast helpers do
  likewise. A 16-byte alloca promoted between equal-width `VecLoad`/`VecStore`
  families may now change *lane interpretation* while preserving every bit;
  `vec_ssa` had unnecessarily required the exact same FP/integer family. Its
  eligibility still rejects different widths, nonzero offsets, volatile
  objects, escapes, lane accesses and other unclassified uses. Unit coverage
  enumerates every supported integer/FP family pair at 16/32 bytes and keeps
  explicit negative controls.
* **Encoding width, register width, and feature permission are different
  facts.** Vol. 1 §§10.2, 14.2 and the Vol. 2 encoding tables distinguish
  XMM legacy/VEX.128, YMM VEX.256, and EVEX forms. Public types alone cannot
  grant an instruction. `X86Isa::normalized`, frontend diagnostics and
  emitter gates must agree for base ISA and function-local targets; AVX1's
  VEX.128 integer/FP instructions are not AVX2 YMM forms. The full per-helper
  audit is not complete.
* **Upper-lane effects matter across passes and calls.** Vol. 1 §14.12 and
  the VZEROUPPER entry (Vol. 2C, PDF page 2832; VEX.128.0F 77, AVX) say that
  the instruction preserves low 128 bits, clears upper state of registers
  0–15 in long mode (0–7 outside it), and does not alter ZMM16–31. The
  backend's per-function YMM dirtiness and late VEX promotion must not infer
  AVX legality from AVX2, nor erase upper state that an ABI-visible argument
  or later wide read can observe. These are separate from feature policy.
* **Alignment is an architectural fault contract, not merely a cost hint.**
  Vol. 1 §§14–15 explicitly distinguish aligned MOVDQA/MOVAPS/MOVAPD from
  unaligned MOVDQU/MOVUPS/MOVUPD. Natural vector alignment may improve
  performance, but it may not be invented for a user pointer. Vol. 1 §§4.9,
  10.2.3 and 11.5 describe sticky SIMD exception flags, masks, rounding,
  FTZ/DAZ and exception priority; tests that merely compare final values do
  not check them.
* **Floating-point helpers remain operand-order sensitive.** The edition-093
  MIN/MAX entries (Vol. 2B pages 4-5–4-18 in the individual volume) define
  source-2 selection on unordered/equal-zero cases, unlike a commutative
  algebraic minimum. Existing native exceptional-value fixtures check order;
  emulator output is not used for hardware exception sign-off.

## Implemented work and limits in this change

The new casts remove a real temporary/memcpy boundary from header helpers;
the vector-SSA equal-width bit-image rule allows native values to cross typed
view changes without scalar pack/unpack. The regression compares runtime bit
preservation to GCC 16.2 at i686 SSE2, x86-64 SSE2 and AVX2, and rejects helper
calls in optimized code. Vector SSA's focused mutation/regression tests, full
library tests, benchmark output checks, gzip stack-memory veto and hosted CI
remain required. This focused work does **not** close the remaining SSE/AVX
public API inventory, all conversion encodings/exception semantics, full manual
review, 256-bit ABI carrier frames, feature subsets, or every long-standing
benchmark deficit; see `backlog.md` SIMD-MANUAL-1, SIMD-PERM-1 and
SIMD-PERF-1.
