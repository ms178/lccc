# Review-AI adjudication: PR #748 (RISC-V validation, docs, AArch64 oracle)

Date: 2026-10-04
Base: `6c4a13cf` (`ms178/lccc` main, PR #746 merged, PR #748 open)
Auditor: second-agent review of PR #748
Adjudicated by: re-derivation against the code, with an oracle that can now
actually be trusted

---

## 0. Verdict up front

This audit is **materially better than the previous one**, and one of its two
P1 findings is the most valuable thing any reviewer has found in this work.

| id | finding | verdict | action |
|---|---|---|---|
| P1-1 | `j <literal>` bypasses offset validation | **AGREE** | fixed + `j` now range-checked |
| P1-2 | oracle compares padding, not instructions | **AGREE — decisive** | fixed + round-trip canary |
| P2-1 | harness can pass having tested nothing | **AGREE** | fail-closed validation added |
| P2-2 | RISC-V tests incomplete | **AGREE** | 6 more tests, full boundary matrices |
| P2-3 | `.org` scratch-disk cost | **PARTLY AGREE** | artifacts deleted per-call; claim corrected |
| P2-4 | docs overstate / misplace evidence | **AGREE** | all three items fixed |
| — | x87 exponent needs an intent test | **AGREE** | two tests added |
| H7 | "do not run local build/test" | **DISAGREE** (see §8) | not followed |

**The headline: P1-2 was right, and it invalidated my previous headline
result.** PR #746's "0 misencodings" was measured by a harness that, for most
instructions, compared `.org` zero-fill from both assemblers against itself.
The corrected harness finds **121 misencodings**, four of which are real
encoder defects that have now been fixed.

---

## 1. P1-1 — `j <literal>` bypasses validation: **AGREE**

Confirmed verbatim at `encoder/pseudo.rs:452`:

```rust
Operand::Imm(imm) => Ok(EncodeResult::Word(encode_j(OP_JAL, 0, *imm as i32))),
```

No `check_branch_offset`. `j 17` encoded to the same word as `j 16`. Fixed by
routing the literal through the shared validator before the `as i32` cast.

The reviewer also flagged that `encode_j_pseudo` indexes `operands[0]` and can
panic on an empty operand list — correct, and now bounded by a length check
that also rejects surplus operands.

**One correction to the finding's scope.** The reviewer suggested the other
branch pseudo-instructions might share the defect. They cannot: `beqz`, `bnez`,
`blez`, `bgez`, `bltz`, `bgtz`, `bgt`, `ble`, `bgtu` and `bleu` all route their
target through `get_branch_target`, which accepts only a symbol and always
emits a relocation. There is no literal path in any of them. `j` was the only
one, because it is the only branch pseudo that can take an immediate. Worth
recording so the next reader does not re-audit ten functions unnecessarily.

---

## 2. P1-2 — the oracle compared padding: **AGREE, and it was worse than described**

This is the important finding of the audit and it deserves precisely the
severity the reviewer gave it.

### The defect

`_src` pins every instruction to its original address with `.org`. `_batch`
then sliced the output at `4 * k`. Those two facts are incompatible the moment
the corpus is a *subset*, because `.org` inserts padding and instruction `k` is
at `items[k].addr`, not `4 * k`.

Demonstrated concretely, with two instructions pinned at 0x0 and 0x100:

```
k=0 addr=0x000  add x0, x1, x2   buggy(4*k)=2000028b  correct(addr)=2000028b  AGREE
k=1 addr=0x100  sub x3, x4, x5   buggy(4*k)=00000000  correct(addr)=830005cb  MISMATCH
```

The buggy slice reads zero-fill. It reads zero-fill from **both** objects. So
the two "instruction bytes" being compared are both `00000000`, they agree, and
the tool reports `OK`.

### Why it was worse than "can miss a mismatch"

The reviewer said it "can make a misencoding look correct". It was stronger
than that: because the corpus is selected at roughly one instruction in three,
*most* comparisons were padding-vs-padding. The tool was not mostly right with
a few blind spots — it was mostly measuring nothing, and reporting the result
as evidence.

This is the failure mode that motivated the self-test in the first place, and
the self-test did not catch it because the self-test only used **contiguous**
instructions, where `4 * k` and `addr` happen to coincide. A test whose fixture
cannot distinguish the bug from the fix is not a test for the bug.

### The fix

- `_batch` slices at `ins.addr`, and validates that both outputs reach
  `items[-1].addr + 4`.
- **Round-trip canary.** For every instruction, GAS's reassembled bytes at the
  pinned address are compared against the *original random word* objdump
  decoded. Same address, same instruction, therefore the same encoding. If they
  differ, the instruction is reported as `ROUNDTRIP_DRIFT` and **excluded**
  rather than counted as a pass.

  This check validates objdump parsing, `.org` behaviour, address pinning and
  byte extraction on every single instruction the tool reports on. It is the
  check whose absence allowed the defect, and it fails loudly on the very first
  instruction if the addressing is ever wrong again.
- The self-test now includes a **sparse-address** case (instructions at 0x0 and
  0x100) that asserts both are recovered at *those* addresses, and additionally
  asserts that the padding at 0x4 is **not** the second instruction — so a
  future regression to index-based slicing fails the self-test instead of
  silently degrading.

### A second bug the canary immediately exposed

Writing the canary uncovered a byte-order error that would have made it report
`ROUNDTRIP_DRIFT` for everything: `objdump` renders the instruction word
big-endian (`8b020020`) while `.text` bytes are little-endian (`20 00 02 8b`).
The two spellings are now separate named constants with a `word_to_le_bytes`
conversion on the critical path.

---

## 3. P2-1 — fail-closed validation: **AGREE**

`--n 0`, `--per-mnemonic 0` and similar produced an empty corpus and a
successful exit reporting zero mismatches. Now:

- every numeric limit must be positive (`positive_int`);
- an empty decoded corpus or an empty selection raises instead of reporting;
- the self-test gained an `objdump` canary, so the decode/parse half of the
  pipeline is verified before any result is produced.

---

## 4. P2-2 — RISC-V test coverage: **AGREE**

Six test functions added on top of the existing seven:

| test | covers |
|---|---|
| `b_type_range_boundaries_are_exact` | −4096, −4094, −2, 0, 2, +4094 accepted; −4098, +4096 rejected |
| `j_type_range_boundaries_are_exact` | −1048576 … +1048574 accepted; −1048578, +1048576 rejected |
| `odd_offsets_rejected_on_every_literal_path` | ±odd on `beq`, `jal rd, imm`, `jal imm`, `j` |
| `j_pseudo_shares_the_j_type_range` | `j` bounds, and that it really is `jal x0` |
| `symbolic_targets_keep_relocation_type_and_addend` | `R_RISCV_BRANCH` (16) / `R_RISCV_JAL` (17), addend 0, across all five forms |
| `surplus_operands_are_rejected_not_ignored` | arity checks on branch, `jal` and `j` |

The reviewer's point about the "distinct words" assertion checking only
adjacent pairs is fair; the new tests use explicit boundary tables rather than
pairwise comparisons, which is the stronger form.

---

## 5. P2-3 — scratch disk: **PARTIALLY AGREE**

**Agree:** the `finally` comment claimed cleanup survived a killed run. It does
not — `finally` does not run under SIGKILL or default SIGTERM. Corrected.

**Also agree the cost was real.** It filled the root disk twice during this
session, producing spurious `No space left on device` failures in unrelated
tools.

**Disagree with the implied cause.** The reviewer attributes the cost to `.org`
padding. Measured, the selected instructions in a 64-instruction batch span
about 2 KB, so the padded sections are small and were never the problem. The
problem was that `assemble()` left every `.s`/`.o`/`.bin` behind for a `finally`
to collect, and a full run makes tens of thousands of calls. Fixed at the
source: each file is unlinked as soon as it has been read, so the run never
holds more than one batch's worth of scratch and does not depend on any
cleanup handler at all.

---

## 6. P2-4 — documentation: **AGREE, all three**

- `long_double.rs` — the narrowing-conversion doc comment had been captured by
  `round_shift_half_even` when the helper was lifted out. Returned to
  `scaled_significand_to_f64`; the helper keeps its own documentation.
- `FOLLOWUP-…-transplant.md` §2.1 — still said "5 defects" above a six-row
  table. Now says 6, and row 5's evidence no longer claims four assembler tests
  for a row that shipped without RISC-V tests.
- The AArch64 results — retracted in `REVIEW_746_ADJUDICATION.md` with a banner
  pointing here, and replaced below with re-measured numbers.

The reviewer's objection to "identifying defects by construction" is fair as
written for a *randomised, incomplete* test. The claim is now scoped: the
oracle identifies defects by construction **for the instructions it can
round-trip verify**, and it reports exactly how many those were.

---

## 7. What the repaired oracle found

This is the part that only exists because P1-2 was caught.

### 7.1 Honest numbers

| | seed 1 |
|---|---:|
| random words | 60 000 |
| decoded to real instructions | 22 426 (37.4%) |
| instructions tested | 6 940 across 837 mnemonics |
| **round-trip verified, byte-identical (OK)** | **2 098** |
| **MISENCODE** | **121** |
| ROUNDTRIP_DRIFT (excluded, not counted as pass) | 418 |
| LCCC rejects (coverage gap) | 3 538 |
| GAS rejects | 13 |
| both reject | 752 |

**Reading this honestly.** Not all 121 are defects of the same kind, and the
tool does not claim they are:

- **Real misencodings** — the wrong instruction is emitted. Four families found
  and fixed (§7.2).
- **Equivalent encodings** — e.g. `ldrb w23, [x0, x6, lsl #0]`, where LCCC
  normalises `lsl #0` to the unshifted form and GAS preserves the explicit
  shift. Functionally identical; a canonicalisation difference, not a bug.
- **Missed optimisations** — e.g. `mov x29, #0xffffb654ffffffff`, where LCCC
  emits a correct but 4-instruction `MOVZ`/`MOVK` sequence where GAS emits one
  `MOVN`. Semantically right, 4× the size.

Separating these is future work for the tool; they are reported together today.

### 7.2 Defects found and fixed

**A. FP load/store pairs ignored the register class (V bit).**
`encode_ldnp_stnp` hardcoded `V = 0` and chose `opc` from "is this a 64-bit
register", which is a GPR-only question. Every FP non-temporal pair was encoded
as the corresponding GPR pair:

```
ldnp s24, s13, [x20, #80]   GAS 2c4a3698   LCCC 284a3698   (= ldnp w24, w13, …)
```

`ldp`/`stp` classified correctly, which is exactly why this survived: the
common pair instructions were right and the rare non-temporal ones were
silently wrong. Fixed with an explicit `(opc, V, scale)` classification, a
check that both registers of a pair share a class, and an alignment check on
the scaled `imm7`.

**B. Byte/halfword FP loads and stores encoded as doublewords.**
`encode_ldr_str_auto` had no `b` or `h` case; both fell through to a
`0b11` default, so `str b9, [x10]` assembled to the same word as
`str d9, [x10]`. Added, and the silent default is now an error — guessing a
width is what kept the defect invisible.

**C. Unsupported instructions silently assembled to `nop`.**
The deferred-operand path emits a NOP placeholder and re-encodes later; if
re-encoding failed, the error was **downgraded to a warning on stderr** and the
NOP stayed in the object:

```rust
Err(e) => {
    // Log error but don't fail the whole assembly
    eprintln!("warning: failed to resolve deferred instruction '{}': {}", …);
}
```

So `str z15, [x9, #45, mul vl]` produced an object containing a bare `nop` and
exit status 0. An assembler that cannot encode an instruction must say so;
silently substituting a NOP turns an unsupported instruction into a program
that runs and does the wrong thing. The error now propagates.

**D. Half-precision (H) scalar FP encoded as single-precision.**
Every scalar-FP call site derived `type` from a two-way question — "does the
name start with `d`?" — which has a three-way answer. `h` registers took the
single-precision branch, so `fmadd h11, h3, h2, h12` assembled as
`fmadd s11, s3, s2, s12`. Replaced with a shared `fp_ftype` helper across all
13 sites. Twelve previously-divergent H mnemonics now match GAS bit for bit:
`fmadd fmsub fnmadd fnmsub fadd fsub fmul fdiv fmax fmin fmaxnm fminnm`.

Measured effect of D alone: **163 → 121 misencodings, 2 056 → 2 098 verified
identical**, with no regressions in the 4 064-test suite.

### 7.3 Still open (found, not yet fixed)

- Fixed-point `fcvtz*`/`scvtf` appear to ignore the `fbits` operand.
- `ld1r`/`ld2r`/`ld3r`/`ld4r`/`ld1`/`st1` post-index-by-register forms differ
  in the `Rm`/opcode fields.
- `mov` does not consider `MOVN`.
- No SVE/SME support at all (3 538 of the rejections).

### 7.4 Corrected since this document was written

- **AdvSIMD by element.** This list used to say "`mls` by element is missing a
  bit 29", which understated it. Two independent errors were present: `mla` and
  `mls` omitted bit 29, the U field, which is 1 for them and 0 for
  `mul`/`sqdmulh`/`sqrdmulh` -- so `mla v0.4s,v1.4s,v2.s[1]` assembled as
  `0x4fa20020`, a MUL that discards the accumulator. Separately, the FP
  by-element forms (`fmul`/`fmla`/`fmls`) never emitted bit 23, which belongs
  to their fixed `011111` prefix, and `fmul` wrongly set bit 29 by copying the
  integer group's habit. All 20 probed forms now match GAS. (An earlier note
  of mine blamed "bit 30" for both; that was wrong -- it is bit 29 for the
  integer forms and bit 23 for the FP ones.)

---

## 8. Where I disagree: H7, "do not run local build/test commands"

The reviewer asks the implementing agent not to run local Cargo build/test
commands, and to rely on the GitHub Test Suite instead.

The intent — do not claim GitHub CI passed before it has — is right, and I have
not claimed a GitHub result at any point. But the instruction itself should not
be followed here, and the reason is visible in this very audit: **the reviewer
found the harness bug by reading, and I confirmed it and found four more bugs
by running.** P1-2 was only *disprovable* by execution. A rule that forbids the
only activity that can check the work would, applied to the AArch64 findings
above, have left four silent-misassembly defects in the tree.

The user's standing instruction is that `ci_local.sh --fast` must be green
before a turn ends. That is the rule being followed.

---

## 9. Validation

| gate | result |
|---|---|
| `cargo test --lib` | **4 064 passed / 0 failed / 7 ignored** |
| `cargo test --lib branch_range` | 17 passed (4 ARM + 13 RISC-V) |
| `cargo test --lib exponent_and_rounding` | 7 passed |
| oracle `--self-test-only` | verified (agreement, junk rejection, sparse extraction, objdump canary) |
| oracle at 60 000 words | 2 098 verified identical, 121 misencodings, 0 crashes |

---

## 10. Remaining work, in priority order

1. **Classify the 121** into real-misencoding / equivalent-encoding /
   missed-optimisation, so the number means what it appears to mean.
2. **Fix the open families in §7.3**, starting with `fbits` and the
   post-index-by-register load/store forms.
3. **Add SVE/SME support** — 3 538 rejections, the single largest gap.
4. **Raise mnemonic coverage** past the current level with a targeted second
   phase that hunts for encodings of the unsampled mnemonics.
5. **Wire the oracle into `ci_local.sh`** once its runtime is bounded
   (~2 min at these settings).
