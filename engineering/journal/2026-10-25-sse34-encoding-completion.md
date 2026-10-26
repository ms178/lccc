# Legacy SSE3 / SSSE3 / SSE4 encoding campaign

## Boundary and reference discipline

This tranche covers legacy instruction assembly in x86-64 and i686, including
AMD SSE4a as a distinct extension. SSE4a is not SSE4.1 or SSE4.2. POPCNT is
included as a separately gated companion; its CPUID bit is not SSE4.2's bit.
AES, AVX, SSE5/XOP and AVX-512 are not silently counted as legacy SSE4.

The primary-reference editions recorded by the preceding manual acquisition
are Intel SDM 325383-093US (September 2026), Vol.2 instruction entries, and AMD
APM Vol.4, publication 26568 rev3.27 (July 2026). Official reference locations:

* https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html
* https://docs.amd.com/v/u/en-US/26568_3.27_APM_Vol4_PUB

The sandbox's network allowlist prevents re-downloading those primary PDFs or
independently checking today's vendor indexes. Do not interpret these citations
as a new complete primary-manual read or a guarantee of today's latest edition.
An independent current-PDF comparison remains open; a supplied PDF would remove
that access limitation. Older GitHub PDF/text mirrors were not substituted and
called current.

The current official Intel XED ISA descriptions **were** re-fetched from
`intelxed/xed`, pinned at `0bcb6237345c5066726dcc08b3d87928df3b5b26`.
The archive SHA256 is
`75a432bb687eef39c1a71e0b500c60adee1ad4da465e8cc97e4dbb8483e724e9`.
The legacy `xed-isa.txt` digest matches the prior vendored reference exactly.
Add the upstream `datafiles/amd/xed-amd-sse4a.txt` description rather than
inventing AMD-only opcode rows in the reference catalogue. The manifest records
source paths, sizes, licenses and individual digests. Regenerate from the pinned
archive using `scripts/sdm_isa_crossref.py --build-catalogue`.

The XED parser now recognizes explicit MOD/REG/RM patterns as having a ModR/M
byte even without the MODRM() nonterminal. Preserve register-versus-memory form
identity with `modrm_mode`, rather than falsely declaring register-only forms
ModR/M-free or collapsing them into memory forms. This explains the generated
catalogue metadata diff; it is not thousands of newly implemented instructions.
Shared i686 dispatch is followed to the actual x86 encoding model. Unknown
routes remain unmodelled failures, not exclusions.

## Implemented defects and coverage

* Both modes: PHADDSW and PHSUBSW, MMX and XMM; PCMPISTRI/PCMPISTRM and
  PCMPESTRI/PCMPESTRM, including all imm8 values.
* i686: PCMPEQQ, LDDQU, MOVNTDQA; shared CRC32 dispatch and POPCNT's memory/word
  forms instead of the old register-only path.
* Both modes: AMD EXTRQ and INSERTQ (register and two-immediate forms), MOVNTSD
  and MOVNTSS. AT&T length/index order differs from architectural byte order;
  the two control bytes and REX field ownership are tested explicitly.
* Shared role validation prevents generic SIMD helpers from accepting MMX in
  XMM-only operations, mixed MMX/XMM classes, stores through load-only rows,
  non-XMM0 explicit legacy blend masks, and incorrect CRC source/destination
  widths or unencodable high-byte/REX combinations.

`tests/asm-diff/gen_sse34.py` generates both checked-in matrices. They cover
all register pairs for the binary families, representative register placements
for control-byte sweeps, MMX variants, disp8 edges,
SIB/high-register/RIP-relative addresses, 16-bit addresses under i686, all 256
control-byte values, insert/extract roles, and individual malformed operands.
The existing whole-object GAS gates discover these files automatically. No
case is classified as `betterok`; allocated bytes and relocations must agree.
A corpus test requires the committed matrices to reproduce exactly.

The independent Clang gate compiles every positive group in both modes using
Clang's integrated assembler and compares exact allocated bytes/relocations.
It requires identified **23.1.4**, fails on a missing/older oracle or any byte
mismatch, and is mirrored in hosted CI and the local gate definitions. Negative
syntax-policy checks remain against GAS; LLVM need not emulate GAS diagnostics.
Mock tests of this gate are not real LLVM measurements. Local LLVM 23.1.4 is
unavailable; actual execution of this gate belongs to hosted acceptance.

End-to-end C inline-assembly execution checks PHADDSW/PHSUBSW against scalar
saturation, and all four packed string comparisons against known answers,
including implicit ECX, EAX/EDX lengths, XMM0 masks and negative explicit lengths.
The source runs with GCC16 and LCCC under the same SSE4.2 permission.
System, non-temporal and AMD-only matrix operations are assembled, not blindly
executed on an incompatible host.

## Evidence and remaining limits

Warning-denied two-job fastbuild of lccc, lccc-ld and lccc-i686 passes with the
8 GiB swap file active. 194 targeted encoder tests pass. Both new whole-object
matrices pass against the identified GNU as/objdump 2.47 pair. Direct/assembly
parity remains green on four witnesses; the unchanged vector-extension quality
ceilings and eleven runtime levels pass. The complete assembler corpora pass: 6,639 x86-64 groups and 6,105 i686 groups.
The new matrices contribute 356/347 groups respectively (37,242 positive
instruction probes, 508 individual rejections). Hosted current-toolchain
results are tracked separately; never call pending runs green.
No local CI sweep was run.

This is not a claim that every instruction is automatically selected from
ordinary C, that all intrinsic headers are complete, or that every invalid
operand combination has been exhaustively enumerated. For example bundled
`nmmintrin.h` still lacks the string-comparison intrinsic API, and SSE4a's
positive driver flag remains deliberately unavailable pending first-class
feature/intrinsic support. These are explicit follow-up integration boundaries,
not silently advertised features. Inline assembly and standalone assembly can
emit the implemented forms; complete intrinsic-API coverage and independently
rechecked latest primary manuals are not claimed finished.
