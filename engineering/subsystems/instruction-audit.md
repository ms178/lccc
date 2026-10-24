# Instruction/manual audit: coverage is a set of proved contracts

This is the live audit boundary across the repository's four backend targets.
A compiler build, an instruction-name census, a PDF extraction, or green CI is
not a whole-architecture semantic audit. None of the rows below is certified
complete. Review also covers the shared frontend/typed IR/vectorizer, assembler,
ABI layout, relocations and linker rewrites, not just emitted opcode spelling.

| Target | This follow-up's reviewed contract | Still outside that contract |
| --- | --- | --- |
| x86-64 | FP promotion must preserve all eight MOVSD source bytes; a partial GP memory store cannot be replaced by a full GP-to-XMM move. Pinned store/load participants cannot change. Native SSE2 and pass regressions cover these cases. | Other passes, instruction families, flags/faults, atomic/volatile behavior, vector upper lanes/masks, feature legality, ABI and link relaxations remain separately reviewed obligations. |
| i686 | No new semantic certification. Its peephole is separate from the x86-64 pass, so the new guards must not be assumed to apply there. | x87/SSE carriers, stack alignment, calling conventions, register constraints, assembler and ELF32 linker contracts remain open. |
| AArch64 | No new semantic certification. | Instruction fields, partial writes, NZCV, SP/ZR roles, SIMD lanes, ABI, TLS and relocations need their own normative references and executable/cross-oracle checks. x86 results do not transfer. |
| RV64 | Immediate shifts/rotates must fit six bits; W forms five. Check the signed operand before narrowing. Keep register-count behavior distinct. Exact legal encoding bits and invalid/immediate-alias cases are pinned. | Remaining base/extensions, compressed/vector instructions, ABI, relocations, linker transformations and execution remain outside this tranche. Encoding agreement is not native-runtime evidence. |

## Sources and provenance

* Intel SDM 093: the prior acquisition record in
  [`../evidence/instruction-audit-2026-10-24/README.md`](../evidence/instruction-audit-2026-10-24/README.md)
  records the combined PDF hash, 5,364 extracted pages and 4,106 bookmarks.
  That is an acquisition record, not 4,106 completed reviews. The PDF and full
  extracted ledger are not present in this restored workspace; reacquire and
  hash-check them before asserting additional whole-section reviews. The new
  narrow MOV/MOVSD contract is supported by executed SSE2 counterexamples and
  production-pass regressions, not a fictitious re-reading of the whole PDF.
* RISC-V unprivileged specification at commit
  `7a90b34e1a04e57a65bfd9de9a3b4e8dc2c5ddc8`,
  [`src/unpriv/rv64.adoc`](https://github.com/riscv/riscv-isa-manual/blob/7a90b34e1a04e57a65bfd9de9a3b4e8dc2c5ddc8/src/unpriv/rv64.adoc),
  `rv64i-slli` and `rv64i-slliw`: RV64 shift immediates have six bits; W
  encodings with imm[5] nonzero are **reserved**. This does not say that all
  reserved forms must trap. The assembler rejects out-of-range source operands
  instead of silently turning them into another legal instruction.
* Zbb rotate immediate fields are checked against exact encoding models and
  Clang 23.1.3's assembler. This is not a full normative Zbb review.
* Current AMD APM and Arm architecture full-text review remains open. Intel
  semantics are not authority for AMD-specific instructions or Arm behavior.
* ELF/psABI rules and APX supplemental encoding specifications are separate
  authorities; do not infer a relocation rewrite from an ISA mnemonic alone.

## Procedure for every subsequent family

1. Pin the normative document/revision and the exact section and source bytes.
   Log missing or contradictory authority; do not resolve it by guessing.
2. Map the relevant frontend/IR operation, vectorizer transformation, backend
   emitter, assembler form and linker consumer. Include i686 explicitly when
   x86-64 assumptions differ. Track target feature and ABI preconditions.
3. State full effects: input widths, preserved/zeroed/sign-extended bits,
   implicit registers/flags, memory access width/order/faults, masks/upper lanes,
   immediates, reserved encodings and relocation PC bias.
4. Add independent integer/lane models, exhaustive finite-field checks and
   native or differential execution when available. Encoding-only or emulator
   evidence must remain labelled as such. Prove each regression can fail by
   reintroducing its defect; retain positive controls for legal optimization.
5. Measure runtime with output/exit agreement, pinned randomized paired samples
   and adequate per-arm work. Compiler-latency improvements are not fixes for
   the SSE2 generated-program runtime gap. PMU and target-cost claims need
   suitable hardware and data, not just the widest legal ISA.

Whole-manual, whole-architecture and universal performance completion remain
open until the corresponding contracts, not merely their names, are reviewed.
