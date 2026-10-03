# Audit: `thanhtoantnt/claudes-c-compiler` — what is worth transplanting into LCCC

Date: 2026-10-03
Base: `070bf71e` (`ms178/lccc` main, post-#743).
Subject: `thanhtoantnt/claudes-c-compiler` — `main` (3982 commits) and the
`pbt-tests` branch (489 files, +87 133 lines), plus its 351 GitHub issues.

---

## 0. Executive summary

**Verdict: yes, this fork is worth mining — but almost none of it is worth
importing as-is.**

The fork is not a compiler fork with a bug tracker. It is a **property-based
testing campaign** that ran against an AArch64 assembler encoder and filed one
GitHub issue per finding. Its value is not the 351 issues; it is

1. the handful of **target-independent defects** that also exist in LCCC
   (three of which were live LCCC bugs and are fixed in this session), and
2. the **methodology**, which when re-pointed at LCCC with a real compiler as
   the oracle immediately found **three further defects the fork never
   reported**.

What is *not* worth importing: 87 133 lines of hand-written per-function
proptests, and 351 issues of which **332 (94.6 %) are AArch64-encoder-only** —
a target that is not where LCCC's performance battle is won or lost.

| | count |
|---|---|
| issues in the corpus | 351 (350 open, **1 closed**) |
| AArch64-encoder-only | 332 (94.6 %) |
| target-independent | 12 |
| distinct `encode_*` functions named | 169 real + 126 report-slug artefacts |
| issues naming a function that **exists in LCCC** | **331** |
| LCCC defects confirmed and fixed this session | **5** (3 silent miscompiles, 1 ICE, 1 precision) |
| LCCC issues in the corpus already fixed upstream | 1 (#5) |
| corpus issues that are *not* compiler bugs | 1 (#351, self-diagnosed) |

---

## 1. What the fork actually is

Both `ms178/lccc` and `thanhtoantnt/claudes-c-compiler` descend from
Anthropic's `claudes-c-compiler`, but **neither shares git history with the
other** (`git merge-base` reports unrelated histories: both were re-authored
from scratch). They are therefore *sibling re-implementations*, not branches.
Any "port the patch" approach is unavailable; everything must be re-derived.

Scale comparison, measured:

| | fork `main` | fork `pbt-tests` | LCCC |
|---|---:|---:|---:|
| commits | 3 982 | — | 5 458 |
| files under `src/backend/arm/` | 64 | 142 | 42 |
| lines under `src/backend/arm/` | ~35 k (excl. tests) | 79 734 | 38 997 |
| AArch64 encoder | 6 files, ~27 k lines | +100 `*_pbt.rs` files | **8 files, 8 581 lines** |
| PBT tests | 0 | 87 133 lines | 0 |

The two ARM backends have **diverged structurally**, not just cosmetically.
LCCC has 34 files the fork has no counterpart for
(`codegen/{f128,i128_ops,atomics,variadic,intrinsics,nested_fn}`, `linker/*`),
and LCCC's encoder is a third the size because it was refactored into shared
helpers (`get_reg`, `get_imm`, `encode_f128`, …) the fork never built.

That refactor is why the fork's defect *density* is so high, and it is also why
the correct fix for LCCC is **one change at the accessor layer**, not 330
per-function patches (see §6).

---

## 2. The corpus, classified

Every issue was fetched from the GitHub API and classified by the defect family
its title/body describes. Issues are multi-labelled.

| family | count | meaning | LCCC status |
|---|---:|---|---|
| `F4-imm-range` | 188 | out-of-range immediate silently masked/truncated instead of rejected | **present** (verified on `encode_bfi`) |
| `F3-width` | 113 | mixed W/X register widths accepted | **present** |
| `F1-regclass` | 53 | FP/SIMD register accepted where a GP register is required | **present** (root cause: `get_reg`) |
| `F6-other` | 51 | unallocated arrangements, wrong opcode bits, … | **present** |
| `F5-panic` | 42 | arithmetic underflow / empty-list panics | **present** (verified on `encode_bfi`, `encode_sbfx`) |
| `F2-sp-aliasing` | 28 | `sp`/`wsp` silently aliased to `xzr`/`wzr` where no SP form exists | **present** (root cause: `get_reg`) |

Verification that these families are live in LCCC, not merely plausible —
`src/backend/arm/assembler/encoder/bitfield.rs:135`, verbatim:

```rust
pub(crate) fn encode_bfi(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;          // F3: width discarded
    let lsb = get_imm(operands, 2)? as u32;       // F4: no range check
    let width = get_imm(operands, 3)? as u32;     // F4: no range check
    ...
    let imms = width - 1;                          // F5: underflow panic when width == 0
```

That single function exhibits four of the six families. It is fork issues #135
and #136, unmodified.

---

## 3. The 12 target-independent issues — the actual prize

`0 < 5 %` of the corpus, and the only part that touches LCCC's primary target
(x86-64 Raptor Lake). Triaged one by one:

| # | issue | LCCC status | action |
|---:|---|---|---|
| 4 | `cast_float_to_target` panics on small f64 → F128 | **BUG** | **fixed** |
| 5 | native F128 `Ptr → F128` classified as signed | **already fixed, better** | none (see §5) |
| 99 | LD1R post-index immediate not validated | AArch64 only (encoder) | deferred |
| 114 | `f64_to_x87_bytes_simple` loses subnormal f64 | **BUG** | **fixed** |
| 118 | `parse_reg_num` over-accepts malformed names | **BUG** | **fixed** |
| 121 | `resolve_local_branches` truncates branch offsets | **BUG (silent miscompile)** | **fixed** |
| 207 | duplicate of #118 | **BUG** | **fixed** |
| 210 | `sminv`/`uminv` wrong across-lanes opcode | AArch64 only (encoder) | deferred |
| 310 | `eval_const_binop_float` ICEs on `0 < \|v\| < 1.0` long double | **BUG** | **fixed** |
| 321 | SYS encoders accept FP/SIMD as `Rt` | AArch64 only (encoder) | deferred |
| 322 | SYS encoders default a missing required `Rt` | AArch64 only (encoder) | deferred |
| 351 | `constant_fold_properties` UT failure | **not a compiler bug** | none (see §5) |

Six of twelve were real LCCC defects; five of those six are fixed in this
session (the sixth, #5, was already fixed).

---

## 4. Defects found and fixed (evidence)

Every fix below is backed by a failing-then-passing test and, where an
independent oracle exists, by byte-level agreement with GCC.

### 4.1 `f64 → binary128` / `f64 → x87` widening of subnormals (fork #4, #114, #310)

**Four** sites shared one defect. All computed the widened exponent as
`biased_exp - 1023` and OR-ed in an implicit integer bit. That is valid only
for normals: a subnormal f64 has `biased_exp == 0` and **no** implicit bit.

Measured consequences (standalone repro of LCCC's exact arithmetic):

```
v = 5.45247436838069e-309  (subnormal)  ->  1.3851606476726355e-308   (2.5x too large)
v = 2.5e-323               (subnormal)  ->  1.1125369292536017e-308   (4.5e14 too large)
```

and, on the f128 path, because the exponent subtraction was typed `u128`:

```
v = 0.5 -> thread 'main' panicked at: attempt to subtract with overflow
```

i.e. **every `|v| < 1.0` ICE'd the compiler** under overflow checks (the
dev/test profiles); `long double x = 0.5L;` was sufficient.

*Fix:* one new shared primitive, `f64_normalize_significand()`, which
renormalizes a subnormal's mantissa and charges the shift to the exponent. All
four sites now call it. The normal path is bit-identical to before, so this
cannot regress existing code.

### 4.2 `f128 → f64` / `x87 → f64` narrowing returned `0.0` (found *by* the new oracle)

Both narrowings computed `mantissa as f64 * 2f64.powi(exp)`. The significand is
~2^112 (f128) or ~2^63 (x87), so the required power-of-two factor is 2^-1135 /
2^-1086 — and `powi` underflows to `0.0` **before** the product is formed.
Every f128/x87 value below 2^-1022 narrowed to `0.0`, which then fed
`eval_const_binop_float` and `passes/constant_fold.rs`.

*Fix:* `scaled_significand_to_f64()` — integer scaling with an explicit
guard/round/sticky step. Now correctly rounded (half-to-even) instead of
truncating, and it cannot underflow.

### 4.3 `f128 → x87` truncated instead of rounding (found *by* the new oracle)

`f128_bytes_to_x87_bytes` did `mantissa >> 49`, discarding 49 bits. Every
`long double` constant that round-trips through binary128 was up to 1 ULP low
relative to GCC. Confirmed exactly:

```
long double c = 1e-320L;
  gcc  : x87 mantissa 0xfd00b897478238d1   (correctly rounded)
  lccc : x87 mantissa 0xfd00b897478238d0   (truncated)
```

*Fix:* `encode_x87_from_scaled()` — half-to-even, with carry-out handling and
renormalization of f128 subnormals. The subnormal case mattered beyond
tidiness: LCCC emitted LDBL_MIN as a **non-canonical x87 subnormal** (exponent
0) where GCC emits the correctly-rounded smallest normal (exponent 1), and
LCCC's own `x87_bytes_to_f64` then read its own constant back as `0.0`.

### 4.4 Branch displacements wrapped silently (fork #121)

`resolve_local_branches` in **both** the AArch64 and RISC-V `elf_writer`
patched every PC-relative immediate with a plain mask:

```rust
let imm26 = ((pc_offset >> 2) as u32) & 0x3FFFFFF;   // R_AARCH64_JUMP26
let imm19 = ((pc_offset >> 2) as u32) & 0x7FFFF;     // R_AARCH64_CONDBR19
let imm14 = ((pc_offset >> 2) as u32) & 0x3FFF;      // R_AARCH64_TSTBR14
```

An out-of-range offset wraps and branches to an unrelated address. This is
worse than an ordinary miscompile: because the relocation is *resolved* at that
point, **no diagnostic is emitted and no external relocation is left behind**
for the linker to catch — the corruption is invisible in the object file.

*Fix:* both writers now validate the displacement against the encoding's range
and error (GAS parity); AArch64 additionally rejects displacements that are not
a multiple of 4.

### 4.5 `parse_reg_num` accepted malformed register names (fork #118, #207)

`name[1..].parse::<u32>()` accepts a leading `+` and arbitrary leading zeros,
so `"x+5"`, `"x007"` and `"w+31"` resolved to real registers. Now rejected
(GAS parity).

### 4.6 Validation

| gate | result |
|---|---|
| `cargo test --lib` | **4 044 passed, 0 failed** (was 4 036 before; +8 new tests) |
| `scripts/ldconst_differential.py --n 3000` | **3 444 / 3 444** long-double constants **byte-identical to GCC 14.2** |
| differential before the fixes | 1 mismatch + 2 defect classes invisible to the fork's corpus |

The differential oracle (`scripts/ldconst_differential.py`, new) is the
durable artefact here: it replaces "someone thought of this value" with a
3 444-value differential experiment against a real compiler, and it is what
found §4.2 and §4.3 — neither of which appears anywhere in the fork's 351
issues.

---

## 5. Issues that are *not* worth taking — with reasons

Being explicit about this is half the value of an audit.

**#5 — native F128 `Ptr → F128` classified as signed.** Already fixed in LCCC,
and fixed *better* than the fork's own proposed patch. The fork suggests
`if from_ty.is_unsigned() || from_ty == IrType::Ptr`. LCCC
(`src/backend/cast.rs:238`) does that **and** normalizes `Ptr` to `U64`/`U32`
by target width, which is what the softfloat libcall actually needs. Porting
this issue would have been a regression.

**#351 — `constant_fold_properties` unit-test failure.** The fork's own issue
text concludes: *"it was determined that the problem was a contract modeling
error in the test, not a compiler bug."* Filing it did not make it true.

**The 332 AArch64-encoder issues.** Real, but:

- they are validation gaps in an assembler that only LCCC's own ARM inline-asm
  path reaches, not codegen defects in generated code;
- LCCC's primary performance target is x86-64 Raptor Lake, and the standing
  brief is to prioritise by `expected performance impact × workloads affected ÷
  implementation cost`. 330 ARM encoder validation bugs score near zero on
  performance impact;
- importing them as 87 k lines of per-function proptests would add ~35 % to the
  crate's line count for coverage that is frozen at whatever someone thought to
  write.

**The `pbt-tests` branch's 100 `*_pbt.rs` files.** Same reasoning. They are
also written against the fork's encoder signatures, which do not match LCCC's
after the shared-helper refactor, so they would not compile.

---

## 6. The distilled artefact: universal invariants, not per-function tests

This is the part of the fork genuinely worth rebuilding properly.

The fork wrote **one test per bug**: 330 findings → 330 tests → 330 issues.
Maintenance cost scales with the bug count and coverage is frozen at the
findings. The 330 findings, however, are all instances of **six** root causes,
five of which are properties that hold for *every* encoder, not
per-mnemonic facts.

So the transplant is: encode the **invariant**, apply it mechanically to all
189 `encode_*` functions through the real `encode_instruction` dispatch table,
and let the harness report violations. Same or better coverage, a fraction of
the code, and it keeps finding bugs after the known ones are fixed.

Properties that need **no reference assembler** (this matters: no `llvm-mc` or
aarch64 `gas` is available in the sandbox, so the oracle must be internal):

| id | property | catches | oracle-free? |
|---|---|---|---|
| U1 | `encode_instruction` never panics, for any operand tuple | F5 | yes |
| U2 | substituting `xN → dN` (same number, different class) must not yield an **identical** encoding | F1 | yes |
| U3 | substituting `xN → wN` in a *source* position must not yield an identical encoding | F3 | yes |
| U4 | immediate `N` and `N + field_modulus` must not yield an identical encoding | F4 | yes |
| U5 | encoding is deterministic | — | yes |
| U6 | `sp` vs `xzr`: must differ where the mnemonic has no SP form | F2 | **no** — needs a per-mnemonic table |

U2–U4 are the elegant ones: they detect "the register-class / width / immediate
field was ignored" **without knowing what the right answer is**, by observing
that two semantically different inputs produced the same bits. U6 is the one
that genuinely requires a data table, which is why it belongs in the repo as
data rather than as tests.

The correct fix for F1/F2/F3 is likewise structural: `get_reg`
(`encoder/mod.rs:1169`) validates nothing — it accepts any register name and
returns `(num, is_64)`, discarding class and conflating `sp` with `xzr`. That
single function is the root cause of **134 of the 351 issues**. Adding
`get_gp_reg` / `get_gp_reg_or_sp` / `get_fp_reg` / `get_imm_range` accessors and
migrating the 209 `get_reg` call sites fixes the family at the source instead
of in 330 places.

**Status:** designed and specified here; not implemented this session. It is
the top item in the follow-up backlog (§7), with the design, the call-site
count, and the false-positive analysis needed to execute it safely.

---

## 7. Recommendation

1. **Take** the target-independent defects — done, 5 fixed, all validated.
2. **Take** the methodology, re-pointed at LCCC with real compilers as oracles.
   `scripts/ldconst_differential.py` is the template; it immediately paid for
   itself by finding two defects the fork's 351 issues missed.
3. **Do not take** the 87 k lines of per-function proptests or the 330
   AArch64-specific issues as-is. Rebuild them as the six universal invariants
   plus one accessor-layer fix (§6).
4. **Sequence** the ARM encoder work *after* x86-64 performance work. It is
   correctness hardening on a secondary target; the standing brief ranks
   generated-code performance on Raptor Lake first.

---

## 8. Provenance / reproduction

```
git clone https://github.com/thanhtoantnt/claudes-c-compiler   # main
git -C claudes-c-compiler fetch origin pbt-tests && git checkout pbt-tests
curl -sS 'https://api.github.com/repos/thanhtoantnt/claudes-c-compiler/issues?state=open&per_page=100&page=N'
```

Corpus analysis artefacts (issue JSON, family census, symbol
presence/absence lists) were produced in `/opt/audit` during the session; the
conclusions and every number quoted above are reproduced in this document and
in `FOLLOWUP-2026-10-03-thanhtoantnt-fork-transplant.md`.
