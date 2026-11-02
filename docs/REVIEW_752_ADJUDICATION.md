# Review-AI adjudication: PR #752 (AArch64 encoder correctness)

The Review AI scored PR #752 at 8.5/10 with a recommendation to merge, and
raised eight findings: two P1, three P2, three P3. This document records, for
each one, whether I agree, what I did about it, and — where I disagree — the
measurement that settles it.

The reviewer has no execution environment, so its findings are derived by
reading code. That makes it very good at spotting *where* a decision is made
without a check, and unreliable about *what the correct answer is*. Every
verdict below was settled by differential test against GNU Binutils 2.44
(`aarch64-linux-gnu-as`), which is the ground truth this project targets.

**Summary of verdicts**

| # | Finding | Verdict | Notes |
|---|---|---|---|
| F1 | Immediate offsets masked, not range-checked | **AGREE** | 13 sites; fixed |
| F2 | No AArch64 encoder regression tests | **AGREE** | 14 tests added |
| F3 | `fmov` FP↔FP classifies only one operand | **AGREE** | and it was worse than described |
| F4 | `fmov` GP↔FP ignores `h` | **AGREE** | but `h` is *legal*, not an error |
| F5 | `ldp`/`stp` duplicates the pair classifier | **AGREE** | fixed by sharing |
| F6 | Oracle not executable; not in CI | **AGREE** | both fixed |
| F7 | Doc says "Eleven" over a list of twelve | **AGREE** | fixed |
| F8 | `Co-authored-by` trailer in commit message | **AGREE** | sandbox artifact; stripped |
| — | Reviewer's exhibit `str q0,[x1,#8]` | **DISAGREE** | GAS *accepts* it |

---

## 1. F1 — offsets were masked, not range-checked: **AGREE**

The reviewer named 13 sites. All 13 existed exactly as described:

- nine `let imm9 = (*offset as i32) & 0x1FF;`
- four `let imm7 = ((*offset >> shift) as i32) & 0x7F;`

plus three `imm12 < 4096` guards with no `else` branch, so an offset too large
for the scaled form simply fell through to the unscaled form, where masking
turned it into something plausible.

`ldr x0,[x1,#32768]` is the sharpest example. 32768 needs a scaled `imm12` of
4096, one past the limit; the unscaled path then computed `32768 & 0x1FF == 0`
and emitted `ldur x0,[x1]`. The program reads the wrong address and nothing
reports a problem. `stp x0,x1,[x2,#8192]` had the same shape: `(8192 >> 3) &
0x7F == 0`, storing to `[x2]`.

This is the same defect class as the RISC-V branch truncation fixed earlier in
this tree: an immediate that does not fit is mangled into a plausible-looking
instruction instead of being refused.

**Fix.** Two functions, `checked_imm9` and `checked_imm7`, are now the only
place an offset becomes a bitfield. Both reject rather than truncate, and both
say what the legal range is and what to do instead:

```
ccc: error: ldr/str: offset 32768 is outside the signed 9-bit immediate of
     this addressing mode (allowed -256..=255); it would silently wrap.
     Use a register offset, or materialise the address into a register.

ccc: error: stp: offset 4 is not a multiple of 8; this pair accesses 8-byte
     elements and the immediate is scaled
```

Misalignment is the same bug wearing a different hat — a scaled immediate
truncates a non-multiple just as silently — so `checked_imm7` checks that too.

Verified against GAS on both sides of the boundary: `ldr x0,[x1,#255]` and
`ldr x0,[x1,#-256]` still encode, `ldr x0,[x1,#256]` and `ldr x0,[x1,#-257]`
are rejected by both assemblers, and likewise `stp x0,x1,[x2,#±504/-512]`.

## 2. Where I disagree: the reviewer's `str q0,[x1,#8]` exhibit

The reviewer listed `str q0,[x1,#8]` in its acceptance set as an instruction
that "should error". **It should not, and the reviewer could not have known
that without running GAS.**

The reasoning is tempting: 8 is not a multiple of the 16-byte `q` access
width, so the scaled form correctly declines; the reviewer assumed there is no
unscaled form for 128-bit accesses, and that the instruction must therefore be
rejected.

GAS disagrees:

```
$ printf '.text\nstr q0,[x1,#8]\n' > q.s && aarch64-linux-gnu-as q.s -o q.o
$ aarch64-linux-gnu-objdump -d q.o
   0:	3c808020 	stur	q0, [x1, #8]
```

`size == 00` with `opc == 10` selects the Q register; that combination is a
legal LDUR/STUR, not an UNDEFINED encoding. The unscaled form exists precisely
to serve offsets the scaled form cannot reach.

I implemented the reviewer's suggestion, and my own differential test caught
it as a regression within minutes — lccc refused an instruction GAS assembles.
That is the whole argument for the oracle the reviewer could not run: the fix
is reverted, the behaviour is now pinned by a named test
(`unscaled_form_exists_for_128bit_accesses`) with a comment explaining why it
is a tempting thing to "fix", and lccc matches GAS on `str q0,[x1,#8]`,
`ldr q0,[x1,#8]` and `str q0,[x1,#-256]`.

**Net effect of F1:** 13 sites, 12 of which now reject correctly — and the
thirteenth is the one where the reviewer was wrong.

## 3. F2 — no AArch64 encoder regression tests: **AGREE, and this was the real gap**

The tree had `branch_range_tests` for the RISC-V displacement work and nothing
equivalent for AArch64. That is why four encoder defects shipped: each fix was
validated by an external script that is not part of the build, so nothing in
the repository would fail if the defect returned.

Added `aarch64_encoder_tests` in `elf_writer.rs` — 14 tests covering all of
the below, the two earlier defects, and the two families found while
adjudicating.

The rule the module is written to is stated in its header comment, and it is
the part that matters most:

> Every expected word in this module was produced by GNU Binutils 2.44 and
> pasted here, rather than derived from the encoder under test. A unit test
> that re-derives its expectations from the code it guards shares every wrong
> assumption that code has, and passes forever.

Every constant in the module is a value GAS printed. Where a test asserts that
two forms differ, it asserts on the *distinctness* of GAS's outputs, not on
lccc's.

This paid for itself immediately: three of my own test expectations were wrong
on first run (one invented constant, one wrong byte index, one error string
containing a stray backslash). Those were caught by the tests failing, which
is the only reason they did not become part of the deliverable.

## 4. F3 — `fmov` FP↔FP classifies only one operand: **AGREE, worse than described**

The reviewer said the code "prefers the destination". It does, and the
fallback makes it stranger: `fmov s0,d1` and `fmov d0,s1` both assembled, and
they produced *different* encodings depending on which end you wrote first —
because the width came from whichever operand the chained `if` happened to
reach.

FMOV between FP registers carries a single `type` field, so the operands must
agree. GAS rejects all five mixed-width forms (`s`/`d`, `h`/`s`, `h`/`d` in
both directions). lccc now classifies both operands and requires agreement,
naming the offending pair:

```
ccc: error: fmov: `s0` and `d1` have different floating-point widths; fmov
     between FP registers moves bits within one width, so both operands must
     be the same (h, s or d)
```

## 5. F4 — `fmov` GP↔FP ignores `h`: **AGREE, but `h` is legal, not an error**

The reviewer proposed rejecting `h` in the GP↔FP direction. Measurement says
otherwise: H has a FMOV (general) form, and lccc was encoding it as S.

`fmov h0,w1` — GAS `1ee70020`, lccc `1e270020`, which is `fmov s0,w1`.

The full legal matrix, established by probing GAS:

| FP width | GP width | `sf` | `type` |
|---|---|---|---|
| S | W | 0 | 00 |
| D | X | 1 | 01 |
| H | W | 0 | 11 |
| H | X | 1 | 11 |

`type` follows the FP operand; `sf` follows the GP operand. S and D therefore
constrain the GP width (`fmov s0,x1` and `fmov d0,w1` are invalid), while H —
the case the reviewer wanted to reject — is the one width that pairs with
either. Q and B have no FMOV (general) form at all.

Rejecting `h` would have been a second regression in the same finding. All
ten legal combinations now match GAS, including `wzr`/`xzr` spellings.

## 6. F5 — `ldp`/`stp` duplicates the pair classifier: **AGREE**

`ldnp`/`stnp` had gained a strict `pair_reg_fields()` classifier as part of
PR #752, and `ldp`/`stp` — which differ from them only in the no-allocate bit
— still carried its own lenient copy. The copy mapped `b` and `h` onto the S
pair, and never checked that the two registers shared a class, so
`stp b0,b1,[x0]` silently assembled as `stp s0,s1,[x0]`.

Both now call one function with the mnemonic passed in, so the diagnostic
names the instruction the user actually wrote. GAS rejects the same inputs.

## 7. F6 — oracle not executable, not in CI: **AGREE**

`scripts/aarch64_encoder_differential.py` was mode 644 while every other
script in `scripts/` is 755, so `./scripts/...` failed. Now 755.

Wiring the full sweep into CI is not viable — one run is ~25,000 assembler
invocations and tens of minutes. But the oracle has a `--self-test-only` mode
that exercises its own invariants (GAS/lccc agreement on known words, junk
rejection, sparse-address extraction, the objdump canary) in a couple of
seconds, and that is exactly the failure mode that matters: a harness that has
drifted out of agreement with GAS reports "OK" for everything and proves
nothing. Added as a `--fast` gate, `aarch64-oracle-selftest`, skipped with an
explicit message when the cross-binutils or the fastbuild binary are absent.

## 8. F7/F8 — documentation and commit hygiene: **AGREE**

"Eleven previously-divergent H mnemonics" introduced a list of twelve. Fixed
to "Twelve". While there, the same document's "still open" list claimed
"AdvSIMD `mls` by element is missing a bit 29" — see §9, that understated it,
and the entry is now corrected in place.

The `Co-authored-by: arena-agent` trailer is a sandbox artifact of how the
patch was produced, not attribution the project asked for. Stripped.

---

## 9. What the adjudication turned up that the review did not

Reading the code closely enough to rule on each finding surfaced two whole
defect families the reviewer did not raise. Both are fixed, both are pinned by
tests, and both were worth more than several of the findings they were found
while checking.

### 9.1 Advanced-SIMD by-element: two independent errors

`mla`/`mls` by element omitted **bit 29**, the U field — which is 1 for them
and 0 for `mul`/`sqdmulh`/`sqrdmulh`. So `mla v0.4s,v1.4s,v2.s[1]` assembled
as `0x4fa20020`, which is MUL by element: a multiply that discards the
accumulator. The program runs and computes the wrong answer.

Separately, the FP by-element forms never emitted **bit 23**, which belongs to
their fixed `011111` prefix, and `fmul` wrongly set bit 29 by copying the
integer group's habit — `fmul v0.4s,v1.4s,v2.s[1]` gave `0x6f229020` against
GAS's `0x4fa29020`.

An earlier note of mine blamed "bit 30" for both. That was wrong: it is bit 29
for the integer forms and bit 23 for the FP ones. It is recorded here because
a confidently wrong note is worse than no note, and because the correction is
the kind of thing only measurement settles.

The `encode_neon_float_elem` signature took a `u_bit` parameter that must
always be zero. Rather than documenting that, the parameter is **gone** — a
parameter that must always be zero is a bug waiting to be reintroduced, and
`fmul` is the proof.

All 20 probed forms in both families now match GAS.

### 9.2 The `#fbits` operand was parsed and discarded

`fcvtzs`, `fcvtzu`, `scvtf` and `ucvtf` accepted a third operand and then
ignored it. `fcvtzs x10,s30,#55` assembled as `fcvtzs x10,s30` — a conversion
whose result is wrong by a factor of 2^55.

Two fields are involved: bit 21 flips from 1 to 0 (it is the bit
distinguishing the integer from the fixed-point form) and bits 15..10 carry
`scale = 64 - fbits`. lccc set neither.

One further subtlety, again settled by probing rather than reading: **only
FCVTZS and FCVTZU have a fixed-point form.** The other eight rounding modes
(`fcvtas`, `fcvtau`, `fcvtns`, `fcvtnu`, `fcvtms`, `fcvtmu`, `fcvtps`,
`fcvtpu`) reject `#fbits` outright; accepting it would encode a rounding mode
the hardware does not have. lccc now matches, and `fbits` is range-checked
against the width of the integer operand (1..=32 for W, 1..=64 for X), which
is where `scvtf s1,w2,#64` is rejected but `scvtf d1,x2,#64` is accepted.

---

## 10. Measured effect

Differential sweep, seed 1, 60,000 random words, `--per-mnemonic 32
--max-insns 12000`, 6,940 instructions across 837 mnemonics, 25,286 assembler
calls:

| | PR #752 (merged) | + F1/F5 | + by-element, `fbits` | + vector `fbits` |
|---|---:|---:|---:|---:|
| OK (byte-identical to GAS) | 2,098 | 2,123 | 2,138 | **2,146** |
| MISENCODE | 121 | 96 | 81 | **73** |
| ROUNDTRIP_DRIFT (excluded) | 418 | 418 | 418 | 418 |
| LCCC_REJECT / GAS_REJECT / BOTH_REJECT | 3,538 / 13 / 752 | unchanged | unchanged | unchanged |

A 40% reduction in misencodings from a clean base, with the excluded and
rejected buckets unmoved — which is the evidence that nothing regressed.

Remaining, in priority order: `mov` immediate (18, should use `MOVN`/bitfield
immediate — a 3-4x code-size regression on some constants and therefore
directly contrary to the project's goal); NEON `st1`/`ld1`/`ld1r`/`ld2r`/
`ld3r`/`ld4r` post-index-by-register `Rm`/index nibble (36); `uaddw2`/`uaddw`/
`sqshrn` Q bit (6); two residual `scvtf`/`fcvtzs` cases and one `fcvtmu` whose
FP-register destination should route to the SIMD form rather than the
general-purpose one; one `and`; and the largest gap by far — **no SVE/SME
support at all**, which accounts for all 3,538 rejections.

## 11. Validation

- `cargo fmt --check` clean
- `cargo clippy --all-targets -- -D warnings` clean
- `cargo test --lib`: 4,079 passed, 0 failed (15 new)
- `ci_local.sh --fast`: all gates green
- `scripts/aarch64_encoder_differential.py --self-test-only`: harness verified

## 12. Where I disagree with the reviewer, in one line

The reviewer's instincts about *where* checks were missing were right in eight
out of eight cases, and its instinct about *what the right answer is* was
right in seven. The exception — `str q0,[x1,#8]` — is the argument for
keeping the differential oracle in the loop: an assembler that refuses
instructions GAS accepts is broken in a way that reading the code will never
reveal.
