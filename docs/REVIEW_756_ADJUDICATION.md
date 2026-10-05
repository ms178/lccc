# AArch64 operand legality: register 31, operand roles, and the two oracles

This document is the durable rationale for the operand-legality work in the
AArch64 encoder: what the bug class is, why a register-*class* check cannot fix
it, which rules the architecture actually imposes, and how the result is kept
true. It records measured results, not intentions.

## 1. The defect class

`parse_reg_num` answers "which register number is this?" and resolves `sp`,
`wsp`, `xzr`, `wzr`, `lr`, `d0` and `x31` alike to a number in 0..=31. That is
the right question for a parser and the wrong one for an encoder, because
AArch64 gives **encoding 31 two different meanings**, and which one applies
depends on the instruction *form* and on the *operand slot*:

| form | Rd (31 =) | Rn (31 =) | Rm (31 =) |
|---|---|---|---|
| ADD/SUB (shifted register) | XZR | XZR | XZR |
| ADD/SUB (extended register) | **SP** | **SP** | XZR |
| ADD/SUB (immediate) | **SP** | **SP** | — |
| ADDS/SUBS (any) | **XZR** | SP (extended form) | XZR |
| AND/ORR/EOR/BIC (immediate) | **SP** | XZR | — |
| ANDS/BICS/TST (immediate) | **XZR** | XZR | — |
| logical (shifted register) | XZR | XZR | XZR |
| CLZ/CLS/RBIT/REV/REV16/REV32 | XZR | XZR | — |
| FMOV (general) | XZR | XZR | — |

A check that asks only "is this a general-purpose register?" answers *yes* for
`sp` in every row, so it cannot reject `clz x0, sp`, and it cannot tell
`mov sp, x0` (an ADD, where 31 is SP) from `mov xzr, x0` (an ORR, where 31 is
XZR). Reducing a spelling to a number before the form is known turns an invalid
operand into a *different valid instruction*, which is a wrong-code bug rather
than a strictness bug.

The fix keeps the three identities apart until the form has been checked:
`GpReg { num, is_64, is_sp, is_zr }` in `encoder/mod.rs`, read per slot through
`reg_operand(operands, idx, role, mn)` with the role that slot actually has
(`RegOrZr`, `RegOrSp`, `RegSpOrZr`, `Reg`). Widths are then compared per form,
because the extended ADD form is the one place where a 32-bit source with a
64-bit destination is legal.

Diagnostics name the fix rather than the rule, because the two spellings of 31
are exactly the case where a user needs to be told which one they meant:

```text
clz: operand 1 `sp` is the stack pointer, but this slot reads encoding 31 as the
zero register; write `xzr` if the zero register is what you meant
```

## 2. Two oracles, and why one cannot substitute for the other

`scripts/aarch64_encoder_differential.py` generates random 32-bit words, has
`objdump` decode them, and re-assembles the resulting text. Everything it feeds
the assembler is therefore a **valid encoding by construction**. It measures
"does LCCC encode a legal instruction the way GNU as does", and it is
structurally blind to "does LCCC reject what it should reject": `mov x0, d1` is
not an encoding, so `objdump` can never emit it and the oracle can never test
it.

`scripts/aarch64_operand_legality_matrix.py` is the other instrument: a curated
table of instruction texts whose every expectation is GNU as's own verdict,
stored in `tests/aarch64/operand-legality.tsv` (423 rows: 251 accepted with
their exact encoding, 172 rejected). It runs in two directions:

* `--check` re-derives every row from the cross assembler, so the table itself
  cannot rot when binutils changes;
* `--check-lccc` asserts the same rows against **our** encoder, so a correct
  table cannot hide a wrong encoder.

Both halves are wired into `ci_local.sh --fast` (skipped, never silently
passed, when the cross-binutils or the fastbuild binary are absent) and into
hosted CI. The table is additionally compiled into the unit-test suite
(`operand_legality_matrix_matches_the_encoder` in `elf_writer.rs`), so the same
guarantee holds on a machine with no aarch64 tooling at all, with ratchets on
the row counts so a regeneration that quietly drops coverage fails.

## 3. What was measured

Environment: rustc 1.99.0, fastbuild profile. The table was generated against
Debian GNU as 2.44 and then re-verified unchanged against the pinned
**GNU as 2.47.20260726** oracle (`scripts/ensure_gas_247.sh`): all 423 rows
agree with both, so the expectations are not an artifact of one binutils
version. The 97-probe battery is likewise 0/97 against 2.47.

| instrument | before | after |
|---|---|---|
| 97-probe audit battery vs GNU as | 38 mismatches | **0** |
| 423-row matrix `--check` vs GNU as | agreed | agreed |
| 423-row matrix `--check-lccc` vs LCCC | 135 disagreements | **0** |
| positive controls (24 probes) | 0 mismatches | 0 |

The probe battery is grouped by the finding it tests (register-31 roles,
operand widths, unguarded GP-only paths, FMOV spellings) plus a positive-control
group that must never regress.

Wrong-code counterexamples, each assembled by the pre-fix encoder and rejected
or corrected now (GAS word first):

| instruction | GNU as | pre-fix LCCC | defect |
|---|---|---|---|
| `mov sp, #1` | `b24003ff` | `d280003f` | wrote **XZR**, not SP |
| `mov x0, #0x10001` | reject | `d2800020` | silently **truncated** the constant to `#1` |
| `fmov h0, wsp` | reject | `1ee703e0` | `FMOV H0, **WZR**` |
| `lsl x0, x1, #64` | reject | `fffffc20` | corrupt word from `width - 1 - amount` underflow |
| `add x0, sp, w1` | `8b2143e0` | `8b2163e0` | read `w1` as 64-bit (UXTX not UXTW) |
| `uxtb x0, w1` | `53001c20` | `d3401c20` | **encoding choice**, not wrong-code: both are UBFM of the low 8 bits (32-bit vs 64-bit form). GAS and llvm-mc prefer the 32-bit word; we match GAS. |
| `mov d0, #1` | reject | `52800020` | `movz w0, #1` |
| `add x0, x1, x2, lsl #64` | reject | `8b020020` | shift masked to `#0` |
| `clz x0, sp` | reject | `dac013e0` | SP encoded as the zero register |
| `mov sp, xzr` | reject | `910003ff` | mixed the two meanings of 31 |
| `movz x0, #1, lsl #15` | reject | `d2800020` | halfword selector not validated |
| `rev32 w0, w1` | reject | `dac00820` | scalar REV32 is 64-bit only |

### Performance

Validation is usually a tax, so it was measured rather than assumed. Callgrind
(valgrind 3.24.0) over a deterministic 20,000-instruction AArch64 workload
weighted towards the families the role gate touches, two interleaved runs per
side, instruction counts exact and identical between runs:

| | Ir | |
|---|---:|---|
| before (register-class checks) | 844,137,540 | |
| after (role-aware checks) | 812,155,331 | **−31,982,209 (−3.79%)** |

The role-aware path is *faster* because it replaced per-operand string
normalisation with allocation-free byte scanning and reads each operand once:

| function | before | after | delta |
|---|---:|---:|---:|
| `<str>::to_lowercase` | 15,457,732 | 10,316,166 | −5,141,566 |
| `malloc` | 49,315,500 | 45,577,136 | −3,738,364 |
| `free` | 77,425,857 | 70,244,263 | −7,181,594 |
| `deallocate` | 20,549,704 | 18,680,522 | −1,869,182 |

The same workload is also a differential test at scale: disassembling both
objects and comparing all 22,506 words against GNU as 2.47 gives **0**
differences for the role-aware encoder and **692** for the class-check version
(all of them the `uxtb`/`uxth` aliases emitted as a wider `ubfx`).

The negative direction is verified too: corrupting two matrix rows makes the
in-tree test fail with the offending rows and the encoder's own message, so the
gate is not vacuous.

## 4. Rules the encoders now enforce

* **Register 31** is resolved per slot per the table in §1; SP and ZR are never
  mixed in one instruction (`mov sp, xzr`, `add sp, xzr, x1`).
* **Widths** are checked per form: the shifted and logical register forms need
  all operands equal; the extended ADD form takes a 32-bit source with a 64-bit
  destination, but a 32-bit *destination* still rejects a 64-bit source
  spelling (`add w0, w1, x2, uxtw`).
* **Shifts** are validated instead of masked: `shift_field` refuses an unknown
  kind, refuses `ror` where the architecture has none (arithmetic forms), and
  bounds the amount by the operand width. `lsl x0, x1, #64` no longer
  underflows.
* **MOV wide immediates** validate the halfword selector (`lsl` by a multiple of
  16, at most 48/16) instead of computing `amount / 16` and ignoring the kind.
* **`mov #imm` is a single-instruction alias**, matching GAS's preference order
  (one MOVZ when the value has one nonzero 16-bit chunk, one MOVN when the
  inverted value does, then the bitmask ORR) and refusing to expand to
  `movz`+`movk`, which is what makes `mov x0, #0x10001` an error rather than a
  silent truncation.
* **The ADD/SUB immediate form** shifts by 0 or 12 only and requires the value
  to fit imm12 *after* the shift.
* **Extend aliases** take a 32-bit source spelling (`sxtb x0, wzr` yes;
  `sxtb x0, xzr`/`sp`/`lr`/`x1` no); `sxtw` is 64-bit only; `uxtb`/`uxth`/`uxtw`
  are 32-bit aliases, so the destination's `x` spelling does not widen them.
* **FMOV (general)** takes `Xn|XZR`/`Wn|WZR` and never SP; operand
  classification is by identity, not by first letter (the old test counted `sp`
  as single-precision, which is why `fmov d0, sp` was reported as a
  floating-point width mismatch).

## 5. Deliberately deferred

* The multiply family (`mul`, `madd`, `msub`, `smull`, `umull`, `smaddl`,
  `umaddl`, `umulh`, `smulh`) and `adc`/`sbc`/`ccmp`/`ccmn` still read operands
  through the permissive `get_reg`. They are GP-only and would benefit from the
  same treatment; they are not in the matrix yet, so nothing here claims they
  are correct. Adding them means adding matrix groups first.
* NEON single-structure and post-indexed forms (`ld1 {v4.b}[0], [x3], x12`,
  `ld1r`, `ld2r`, `pmull2 v7.8h, ...`) mis-encode or drop the register offset.
  This is independent of the operand-role work: it lives in `neon.rs`, predates
  it, and needs its own matrix group before it can be fixed against an oracle.
