# SSE/AVX public surface and native-helper audit — 2026-10-26

This is an incremental audit, **not exhaustive semantic or manual sign-off**.
The machine-readable inventory makes omissions visible instead of equating
existing wrappers with the complete public API.

## Public surface

Run `python3 scripts/audit_sse_avx_public_api.py --check` (or `--self-test`).
The checked-in reference requires GCC **16.2.0** when regenerated with
`--write-reference --gcc PATH`; each of its eight resource headers is SHA256
pinned. Conditional definitions, macros, object aliases, operand counts,
private helper/raw builtin dependencies and canonical overrides are retained.
The parser is deliberately a controlled-header scanner, not a general C parser.
Its controls cover multiline declarations/macros, nested parameter commas,
post-declarator attributes, aliases, comments and conditional alternatives.

| Accounting | Count |
|---|---:|
| Bundled `_mm_*` / `_mm256_*` names | 624 |
| GCC names in the eight selected SSE–AVX2 leaf headers | 906 |
| GCC reference-only names | 321 |
| Bundled-only names relative to those leaf headers | 39 |

Bundled scope also includes shared `lcccsimd.h`. Reference-only means absent
from the extracted definitions, not necessarily absent from canonical lowering.
Bundled-only does not mean nonstandard: FMA/AVX512VL reference headers are outside
this comparison. Uppercase control macros/constants, MMX aliases and further
extension headers require separate inventories (SIMD-API-1 in `backlog.md`).

## Concrete repairs and native migration

* Header-derived arities now include generic helper declarations/calls, and
  `(void)` means zero arguments. The generated frontend table grows from 558
  to 856 entries. The lowering independently checks aliases not present there.
* Permission checks use the resolved mnemonic, including an appended width;
  an unsuffixed SSE1 spelling no longer defaults to SSE2 permission.
* Nine AVX-512 immediate shifts have one real source, not two. The leading
  control supplies the immediate; it is appended to the IR operands once.
* Unknown helpers diagnose instead of falling through. Two historical
  vector-result declarations (`512_i_vpcmpeqb512`, `512_i_vpcmpeqw512`) remain
  explicitly unmapped and ratcheted by the metadata unit test; they are not
  claimed implemented.
* Packed divide/min/max/sqrt use register-valued Vec IR for F32x4, F32x8 and
  F64x4 on x86-64 when the complete operand plan is legal. Volatile access,
  foreign address spaces, mismatched types and unsupported shapes keep the
  established path. Argument order is unchanged.
* Eight 256-bit FP helper result prototypes/adapters now use native vectors,
  not `void *` reinterpretation. Other families are deliberately not converted
  by a width-wide textual replacement.
* F64x2 keeps the existing emitter: its inlined I128 carrier bridge made an
  attempted native route worse (77 to 92 provider instructions). That attempt
  was rejected. Wide native types and ABI carrier elimination are different
  obligations; remaining bridges are SIMD-PERF-1.

## Current primary-manual evidence

Intel's [manual index](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html)
identifies edition 093. Following its **individual Volume 2B** link resolves to
[253667-093](https://cdrdv2-public.intel.com/929354/253667-093-sdm-vol-2b.pdf).
Unlike the combined PDF, the first 30 pages of this volume expose actual
instruction sections. Retrieved instruction anchors:

| Instruction | Printed Volume 2B pages | Facts checked against native lowering |
|---|---|---|
| MAXPD / VMAXPD | 4-5–4-7 | SSE2 legacy, AVX VEX; source 2 wins signed-zero ties/unordered inputs; source-2 SNaN is forwarded unchanged; Invalid (including QNaN), Denormal |
| MAXPS / VMAXPS | 4-8–4-10 | SSE legacy, AVX VEX; same ordered source-2 rule; legacy destructive destination versus VEX three operands |
| MINPD / VMINPD | 4-16–4-18 | SSE2 legacy, AVX VEX; source-2 zero/NaN rule, noncommutative operand order, exception list |
| MINPS / VMINPS | 4-19 onward | SSE legacy, AVX VEX; same source-2 rule, lane counts and legacy/VEX upper-lane distinction |

The same tables expose `(AVX512VL AND AVX512F) OR AVX10.1` for narrow EVEX forms:
width or a leading `v` alone is not a sufficient permission model. The selected
change emits legacy/VEX, not EVEX/AVX10.1. Subset work remains SIMD-PERM-1.

Retrieval used the page extraction tool, limited to the first 30 PDF pages;
Volume 2B chunks 3–7 contain these sections. A full binary hash was **not**
obtained, so these are edition/URL/printed-page anchored observations, not a
hash-pinned full-manual campaign. Intel combined 325462-093 and AMD 26568
rev 3.27 / 24594 rev 3.38 edition checks alone do not establish instruction
coverage. Division/sqrt's new runtime tests do not replace their still-pending
current-manual row review. See SIMD-MANUAL-1 for the full outstanding scope.

## Focused evidence and limits

`check_simd_helper_contract.py` exercises missing/extra operands without header
prototypes on both x86 modes at O0/O2, canonical aliases and unknown helpers:
**2,868 compile legs**. The saved pre-change compiler fails this gate.
`legacy_permission_tests` cross-checks every generated helper prototype against
lookup metadata or the exact two-name unmapped ratchet.

`native_packed_fp_helpers.c` is checked against GCC 16.2 in all four separate
caller/provider directions, direct and indirect calls, O0/O2, i686/x86-64,
SSE1/SSE2/AVX/AVX2: **64 runtime legs**. Inputs include all 225 pairs from 15
raw values per format, four operations and four MXCSR rounding modes; outputs
retain NaN payloads, signaling encodings, infinities, signed zero and subnormals.
MXCSR exception flags are accumulated, **not checked individually per operation**.
FTZ/DAZ combinations, unmasked traps and guard-page memory forms remain open.
The original packed arithmetic ABI fixture remains a separate 64-leg control.

Provider instruction counts (`-m64 -O2 -DPROVIDER_ONLY`; branch/return instructions
included) and whole-process Callgrind screening (`-DREPEATS=32`):

| Shape | Saved pre-native compiler | Candidate |
|---|---:|---:|
| SSE2 F32 provider instructions / frame references | 70 / 46 | 63 / 32 |
| SSE2 F64 provider instructions / frame references | 77 / 53 | 77 / 53 |
| AVX2 F32 provider instructions / frame references | 235 / 67 | 166 / 67 |
| AVX2 F64 provider instructions / frame references | 235 / 67 | 166 / 67 |
| SSE2 whole-process Callgrind Ir | 38,994,190 | 38,245,394 |
| AVX2 whole-process Callgrind Ir | 77,210,489 | 69,261,689 |

Exact repeat-32 outputs match: SSE2 `6ff3772e59d63a25 3f`, AVX2
`3be387364d9e4325 3f`. Candidate SSE2 and AVX2 Memcheck: zero errors, no leaks.
The provider-density gate fails the saved compiler (70 exceeds 69). These are
mechanism/memory-safety receipts, **not a portable runtime speed claim**.

Environment: active 4 GiB swap; fastbuild, two jobs, no concurrent heavy build;
GCC 16.2.0, GAS 2.47.20260726, mold 2.42.1. Exact bootstrap downloads failed;
the authorized fallback is Rust 1.97.0 / Clang 23.1.3. Cached Valgrind 3.19.0
was used with matching private glibc debug symbols; no claim of latest Valgrind.
The pre-native binary was saved before native helper edits, and its old alias
permission failure was reproduced; it is not represented as a separately
reconstructed clean-source compiler build. Hosted CI is the full acceptance
lane; no local CI suite was run.
