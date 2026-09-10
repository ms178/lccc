# Follow-up: i686 addressing encoder audit — GAS-parity rewrite + segment-override sweep

Date: 2026-09-10 · Base: ms178/lccc @ 919572c (PR #470, rebased from 6461190 per session policy)

## 1. Scope

Full red-team audit of the i686 assembler addressing encoder
(`src/backend/i686/assembler/encoder/core.rs`) plus every related file, per
the directive that superseded the original in-file-only constraint.
Deliverables: a production rewrite of `core.rs` (validation-before-mutation,
allocation-free address planning, GAS-verified semantics), cross-file fixes
in six encoder files, a differential regression corpus, a new CI gate, and
this document.

## 2. Astra's red-team report — adjudication

Agree / disagree, point by point.

| # | Astra finding | Verdict | Evidence |
|---|---|---|---|
| 1 | Original helper misassembles `(%bx)` in .code32 as `(%ebx)` without 0x67 | **Agree — worse than reported**: `(%xmm0)`→`(%eax)`, `(%ah)`→`(%esp)`-base SIB, scale-3→scale-1, `(%esp,%esp,2)` index silently dropped. All confirmed live via `lccc-i686` + objdump before the rewrite. | pre-fix bytes: `8b 03`, `8b 00`, `8b 14 08`, `8b 04 64` vs GAS/errors |
| 2 | Octal reinterpretation of leading-zero literals is an unjustified compatibility change | **Agree**. Restored the decimal contract (`"010"` = 10). GAS treats `010` as octal 8 — a documented, deliberate divergence (historical lccc contract; kernel sources do not rely on GAS octal). | in-file test `integer_literals_preserve_decimal_contract` |
| 3 | Tests did not exercise real relocation emission | **Agree**. New in-file tests build `InstructionEncoder` instances and assert relocation type/offset/addend/diff_symbol/zero-field through `add_relocation`, `encode_modrm_mem`, `encode_abs_addr_*`. | `actual_emission_records_displacement_offsets_and_zero_fields` et al. |
| 4 | Repeated `Vec::with_capacity` per call is waste | **Agree (minor)**. Rewrite plans into a fixed `[u8; 6]` buffer — zero allocation per operand. | `I686AddressPlan` |
| 5 | "Transactional" guarantee needed qualification | **Agree**. Rewrite validates *everything* (register names, scale, ESP-index, mode, mixed widths, relocation width/diff/modifier) before the first `self.bytes.push`. | `encoder_modrm_mem_validates_before_mutating` |
| 6 | Contracts left outside the file | **Agree**. The rewrite carries the full contract set in-file: REL addend materialization (field bytes stay zero; addend travels in `Relocation::addend`), relocation offset = displacement-field offset, `@PLT` stripping in `add_relocation` only, `tls_reloc_type` mapping, parser guarantees (six segment names, symbol spelling). |
| 7 | Invalid segment not reportable through the `()` interface | **Agree, but the parser makes it unreachable**: `MemoryOperand.segment` is only ever one of es/cs/ss/ds/fs/gs (parser gate), so the `()` interface is kept (cross-file signature stability) with a fail-fast panic on impossible input, documented in-file. |
| 8 | 16-bit symbolic displacements rejected | **Fixed**: `.code16` `sym(%bx)` now assembles to `8b 87 <disp16>` + R_386_16, GAS-identical including REL addend patch width 2. |
| 9 | GAS rejects unknown `@MOD` modifiers; old code silently degraded them to R_386_32 | **Agree, fixed in-file**: modifier mirror table with a drift-catch test against `tls_reloc_type`. |
| 10 | Release gates were unverified | All now run: repo compile (fastbuild), 2256 unit tests, asm-diff differential vs `as --32` (bytes **and relocations**), ELF REL addend checks, 16-bit resolver paths, `ci_local.sh --fast` fully green. Hardware measurement (boot) not available in this harness (no kernel tree) — flagged below. |

### Points where this session went further than Astra

* **The GAS displacement rule is modular, not strict-reject**: Astra's v2
  rejected literals outside `[iN::MIN, uN::MAX]`. GNU as accepts them: value
  in the union `[iN::MIN, uN::MAX]` is sign-normalized and shortened
  (`0xffff(%eax)` → disp8 `ff`); values outside wrap mod 2^N with a
  forced full-width field (`0x10000(%bx)` → disp16 `00 00`). Byte-exact GAS
  parity implemented and oracle-tested (`normalize()` + golden tables).
* **`.code32` 16-bit addressing is now fully supported** (GAS emits 0x67):
  planner selects the 16-bit table in both modes; a new
  `fixup_code32_addr16_prefix` splices 0x67 after the group-1/segment prefix
  run, relocating offsets. No more silent rejection divergence.
* **Segment overrides were silently dropped in ~100 memory-operand encode
  sites** across gp_integer/sse/system/vex/x87 (only mov and a few paths
  emitted them). All sites now route through `emit_segment_prefix`, which
  implements GAS redundancy semantics (override emitted iff it differs from
  the addressing form's default segment) and correct ordering (segment
  before 66/67, after FWAIT).
* **Accumulator moffs shortforms** (`mov abs,%eax` = `a1 <disp>`) for pure
  absolute operands — 1 byte shorter than the ModR/M form, GAS-identical,
  including `%es:`-overridden forms (`26 a1 ...`).
* **The scale-1 index fold beats all oracles**: `mov -1(,%ecx,1),%eax` →
  `8b 41 ff` (3 B) vs GAS/GCC/clang/ICX `8b 04 0d ff ff ff ff` (8 B).
  Upstream PR #470 meanwhile ported the same fold but folds `%ebp` too;
  kept excluded here: on i686, `%ebp` as index defaults to DS while `%ebp`
  as base defaults to SS — observable whenever DS≠SS. Folding eax/ecx/edx/
  ebx/esi/edi is semantics-preserving (DS both ways).
* **Prefix-order defects**: `.code16` rebuild order (segment → 67 → 66,
  GAS-verified) and FWAIT placement (`9b` ahead of the whole prefix run).

## 3. Files changed

| File | Change |
|---|---|
| `src/backend/i686/assembler/encoder/core.rs` | Full rewrite: `I686AddressValue` (modular normalization), allocation-free `I686AddressPlan`, strict validation before mutation, GAS parity for 32/16-bit tables, moffs helper, shared resolve-and-emit path, ~30 unit tests incl. an independent decoder and a 65536-value 16-bit sweep |
| `.../encoder/mod.rs` | `.code32` 16-bit addressing fixup; FWAIT-aware prefix rebuild in both fixups |
| `.../encoder/gp_integer.rs` | Segment overrides in all ALU/shift/bit/mov paths; accumulator moffs shortforms for absolute movs |
| `.../encoder/sse.rs`, `system.rs`, `vex.rs` | Segment override emission for every memory-operand arm |
| `.../encoder/x87.rs` | Override before FWAIT-carrying encoders; fnstsw/fstsw/x87-mem restructured (raw bodies) to keep `9b 26 ...` order |
| `tests/asm-diff/i686/addressing.casefile` | New: 12 case groups covering bases, displacements, 16-bit addressing in both modes, absolute forms, segments, `.code16`, relocations, rejects, and the fold (`betterok`) |
| `scripts/ci_local.sh` | New fast gate `i686-asm-diff` (`asmdiff.py --32 --lccc target/fastbuild/lccc-i686`) |

## 4. Empirical results

* 17/17 i686 asm-diff cases pass (bytes + normalized relocations) vs `as --32`.
* 2256/2256 lib tests pass; `ci_local.sh --fast` all green.
* Pre-fix silent miscompiles, now errors or GAS bytes:
  `mov (%bx),%eax` `8b 03`→`67 8b 07`; `mov (%xmm0),%eax` `8b 00`→error;
  `mov (%eax,%ecx,3),%edx` `8b 14 08`→error; `mov (%esp,%esp,2),%eax`
  `8b 04 64`→error; `mov 0xffff(%eax),%ebx` 6 B→GAS 3 B (`8b 58 ff`).
* Segment battery (50+ forms across mov/alu/lea/push/shift/imul/sse/avx/x87/
  system, both modes): byte-identical to GAS.
* Oracle: GNU as (binutils) **2.44** (Debian trixie); repo pins 2.47 via
  `scripts/ensure_gas_247.sh` — no 2.44/2.47 divergence is expected for
  these legacy encodings, but re-verify with 2.47 if available.

## 5. Remaining follow-ups (recorded, not hidden)

1. **Hardware/realmode end-to-end**: no kernel tree in this harness
   (`scripts/realmode_corpus.sh` needs KERNEL_DIR). Run
   `prepare_kernel_tree.sh` + realmode corpus + a boot test before trusting
   the `.code16` paths beyond byte-level parity.
2. **`asmdiff.py` betterok limitation**: semantically_equal objdump-decodes
   `.code16` text as 32-bit, so fold-wins inside `.code16` cannot be
   expressed as casefile cases (kept as in-file unit tests instead).
3. **Modifier mirror drift**: `i686_known_tls_modifier` mirrors the table in
   `x87.rs`; a unit test fails if a new modifier maps to the R_386_32
   fallback, but the two lists should be kept in sync on future TLS work.
4. **`emit_segment_prefix` panic path**: unreachable via the parser today;
   if the parser ever admits arbitrary segment tokens, switch the interface
   to the `Result` form the x86-64 backend uses (callers: ~120 sites).
5. **Upstream fold divergence**: upstream's `fold_scale1_index` folds `%ebp`
   (DS→SS default-segment change on i686). Consider upstreaming the
   ebp-exclusion argument; harmless in flat model, observable otherwise.
6. **`mov %es:sym, %eax` style moffs with memory-parsed operands**: the
   moffs shortform currently triggers for the Label path and for
   base/index-less Memory operands; both covered. Symbolic-diff/moffs
   interactions remain untested at link time (no link-level i686 oracle).
