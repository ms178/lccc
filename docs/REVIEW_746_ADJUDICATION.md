# Review-AI adjudication: PR #746 (long-double constants + branch range)

> ## ⚠️ RETRACTION — the AArch64 results in §9 are invalid
>
> The AArch64 differential results originally reported here ("2 804 / 2 818
> byte-identical, 0 misencodings") were produced by a harness that compared
> **padding bytes instead of machine code**. `Differ._batch` sliced the output
> at `4 * index`, but the `.org` pinning described in §9 means instruction `k`
> lives at `items[k].addr`, not at `4 * k`. Once a subset of instructions is
> selected the addresses are sparse, so most comparisons were reading `.org`
> zero-fill from *both* assemblers and reporting agreement.
>
> The conclusion "LCCC's AArch64 encoder is correct" was therefore unsupported.
> It has been corrected and re-measured: see
> [`docs/REVIEW_748_ADJUDICATION.md`](REVIEW_748_ADJUDICATION.md), which fixes
> the harness, re-runs it, and reports **121 real misencodings** — including
> four genuine encoder defects that the broken harness had masked.
>
> Everything else in this document (the F1–F7 adjudication, the RISC-V work and
> the long-double work) stands.

Date: 2026-10-04
Base: `6c4a13cf` (`ms178/lccc` main, PR #746 merged)
Auditor: second-agent review of PR #746
Adjudicated by: deep re-derivation against the code, not against the review

---

## 0. Verdict up front

The review was **substantially correct and unusually well-calibrated**. Of its
seven findings I **accept five**, **reject one**, and **dismiss one as a
non-finding**. Two of the accepted ones were *under-argued* — the reviewer
reached the right conclusion without the evidence that actually settles it — and
one accepted finding had the **wrong fix location**, which matters: applying the
reviewer's implied fix would have produced dead code that looks like a fix and
isn't.

The review's 8.6/10 and MERGE recommendation are fair. The gap between 8.6 and
10 was test parity and hardening, which is exactly what this session closes.

| id | finding | verdict | action |
|---|---|---|---|
| F1 | RISC-V branch tests = 0 | **AGREE** | 7 tests added (`elf_writer.rs`) |
| F2 | RISC-V truncates bit 0 | **AGREE, stronger than argued** | fixed — but at a *different site* than implied |
| F3 | x87 subnormal exponent | **AGREE** | fixed at the root, not the call site |
| F4 | duplicated `round_shift` | **AGREE** | lifted to one `round_shift_half_even` |
| F5 | `/target` subsumes `/target/` | **DISAGREE** | no change; folding would reintroduce the bug |
| F6 | FOLLOWUP doc sandbox debris | **AGREE** | trimmed into §1 + Appendix A |
| F7 | "4044 passed" unverified | **NON-FINDING** | re-run: 4051 passed / 0 failed |

---

## 1. F1 — RISC-V branch-displacement tests: **AGREE**

Confirmed by inspection: the only `#[cfg(test)]` module in the whole RISC-V
assembler was `compress.rs:728`. The fix landed in both writers but only the
ARM one got tests.

Added `mod branch_range_tests` to `src/backend/riscv/assembler/elf_writer.rs`
with 7 tests. Their content is *not* a copy of the ARM module, because the two
backends genuinely differ (see F2) — copying would have produced tests that
either fail or assert the wrong contract.

The review also correctly noted the FOLLOWUP doc's row 5 ("4 new assembler
tests") overstated the work: those 4 were ARM-only. The row now reads as part of
a corrected §2.1 table.

---

## 2. F2 — RISC-V truncates branch bit 0: **AGREE — and the reviewer was too soft**

### The defect is real; here is the proof

The reviewer inferred the truncation from reading the encoding. I confirmed it
at the byte level by assembling literal offsets through lccc:

| source | emitted word |
|---|---|
| `beq x1, x2, 16` | `0x00208863` |
| `beq x1, x2, 17` | `0x00208863`  ← **identical** |
| `beq x1, x2, 4094` | `0x7e208fe3` |
| `beq x1, x2, 4095` | `0x7e208fe3`  ← **identical** |
| `jal x1, 4096` | `0x000010ef` |
| `jal x1, 4097` | `0x000010ef`  ← **identical** |

`encode_b` splits `imm[4:1]` and `encode_j` splits `imm[10:1]`; neither layout
has an `imm[0]` field, so an odd offset is silently rounded down and the branch
lands somewhere else. This is not theoretical — it is a silent misassembly.

### But the reviewer's implied fix location is wrong

The natural place to add the check — and where the analogous AArch64 check
lives — is `ElfWriter::resolve_local_branches`, next to the existing range
check. **A check there would never execute.** `process_instruction` routes every
B-type (16) and J-type (17) relocation to `base.add_reloc(...)`, i.e. to an
external relocation, with the comment:

> For BRANCH (16), JAL (17): always emit as external relocations so the linker
> resolves offsets correctly after relaxation.

Only *non*-branch/jal local relocations reach `pending_branch_relocs`, which is
what `resolve_local_branches` walks. I verified this empirically: a branch to a
forward local label, a backward local label, and an external symbol all emit
`R_RISCV_BRANCH` relocations and none is patched inline.

This policy is **correct and deliberate** — RISC-V linker relaxation shrinks
code after assembly, so the assembler genuinely cannot know the final offset.
So the defect is *not* that branch offsets are resolved inline and unchecked;
it is that a **literal immediate** operand is encoded on the spot, where the
offset *is* knowable, and was not validated.

**Fix applied:** a `check_branch_offset` helper in
`src/backend/riscv/assembler/encoder/base.rs`, called from the three
literal-immediate paths — `encode_jal` (both the `jal rd, offset` and the
implicit-`rd` `jal offset` form, which are separate code paths) and
`encode_branch_instr`. It rejects odd offsets and out-of-range offsets.

I first wrote the check in `resolve_local_branches` per the review's framing,
then found it unreachable and removed it. Leaving it would have been worse than
not fixing F2: a test-shaped object that asserts nothing.

### The tests were rewritten accordingly

They now assert the *actual* contract:

- odd literal offsets are diagnosed, for both `beq` and `jal`;
- even in-range offsets still encode, and encode to **distinct** words (a test
  that only checks "something was rejected" would also pass for an encoder that
  rounds everything to zero);
- out-of-range literal offsets are diagnosed, both B-type and J-type;
- the implicit-`rd` `jal` form is validated too;
- a branch to a **symbol** is left as exactly one relocation for the linker —
  locking the relaxation contract so a future "optimisation" to patch branches
  inline cannot land silently.

---

## 3. F3 — x87 subnormal exponent: **AGREE, and provably zero-risk**

`X87Decomposed::unbiased_exp()` returned `biased_exp - 16383` unconditionally.
For an x87 subnormal (`biased_exp == 0`, mantissa non-zero) IEEE defines the
exponent as the **minimum normal** exponent, `-16382` — so every subnormal was
scaled one power of two too small.

The reviewer said the bug is "invisible" but did not establish why. It is
invisible because every x87 subnormal lies below `2^-16382 ≈ 3.4e-4932`,
which is far under f64's `2^-1074` underflow limit; the `unbiased < -1074` guard
returns `±0.0` either way.

**The stronger argument the reviewer missed:** `unbiased_exp()` has *exactly
one* caller in the entire tree (`x87_bytes_to_f64`, `long_double.rs:846`). That
single-call-site property makes the correction provably zero-risk — there is no
second consumer whose behaviour could shift. A fix justified by "it's currently
invisible" invites the next reader to defer it; a fix justified by "it has one
caller and is wrong" does not.

Fixed at the **root** (in `unbiased_exp()` itself) rather than at the call site,
so the value is right for any future caller.

---

## 4. F4 — duplicated `round_shift`: **AGREE**

Two byte-identical private copies lived inside `scaled_significand_to_f64` and
`encode_x87_from_scaled`. A rounding rule that exists twice is a rounding rule
that drifts. Lifted to one module-level `round_shift_half_even`, with the
`sh >= 128` tie-goes-to-even rationale (present in one copy but not the other)
preserved.

Worth recording: removing the copies **orphaned two doc comments** that had been
attached to the nested functions, leaving them attached to `let` statements.
That trips `-D unused-doc-comments`, which is a CI gate. Caught by building, not
by reading — a reminder that "cosmetic" refactors are not always cosmetic.

---

## 5. F5 — `/target` subsumes `/target/`: **DISAGREE**

The reviewer is right about gitignore semantics: a trailing slash matches
directories only. The inference — that the bare `/target` line is therefore
redundant — is wrong, and the premise it rests on is the opposite of the truth.

The pair exists *because* of the symlink case. When the build directory is moved
off a small root disk and bridged back with `ln -s /path/to/target target`, the
entry is a **file**, not a directory, so `/target/` does **not** match it. The
bare `/target` was added to catch exactly that, after a symlink was committed.

The comment above the pair already states this. Folding the two lines would
silently reintroduce the bug the comment exists to prevent. No change made; the
finding is cosmetic and, if applied, harmful.

---

## 6. F6 — FOLLOWUP doc carries sandbox debris: **AGREE**

`engineering/FOLLOWUP-2026-10-03-thanhtoantnt-fork-transplant.md` opened with a
"the sandbox was wiped bare" section and an environment-bootstrap recipe —
~25% of the document, host-specific, and the first thing a reader hits.

- §0 and §1 collapsed into a short **§1 Environment note** that keeps only the
  two constraints that *are* project content (the `target/` size problem and the
  delivery-publisher budget), with the two-line justification;
- the recipe moved to **Appendix A**, explicitly labelled host setup;
- local-only commit hashes in the header replaced with "Merged upstream as
  PR #746";
- the trailing "known-good state" line, which named a sandbox path, removed.

---

## 7. F7 — "4044 passed" unverified: **NON-FINDING**

The reviewer flagged the number as unverifiable because it could not run the
suite. It is reproducible: the full `cargo test --lib` suite was re-run twice
this session. See §10.

---

## 8. New findings this session (not in the review)

These came from building a differential oracle, not from reading the code.

### F8 — LCCC's AArch64 assembler silently ignores surplus operands

```
add x0, x1, x2, x3        GAS: error      LCCC: accepted
add x0, x1, x2, x3, x4, x5  GAS: error    LCCC: accepted
```

A typo in generated assembly is accepted and the extra operands discarded. This
was found because it **broke the oracle's self-test**: the self-test asserts a
known-bad instruction is rejected by both assemblers, and `add x0, x1, x2, x3`
turned out to be accepted by lccc. Harmless for lccc's own codegen output;
dangerous for hand-written `.s` files, where it converts a typo into a wrong
program with no diagnostic. **Not yet fixed** — it needs a decision on whether
lccc's assembler is a compiler-internal one (strictness optional) or a
user-facing one (strictness required).

### F9 — LCCC has no SVE/SME support

`stnt1b {z24.b}, p7, [x0, x7]`, `sel z3.s, p9, z23.s, z22.s`, and the whole
SVE/SME family fail with "unsupported instruction", even with
`.arch armv9.4-a+sme` in the source (the directive itself is accepted). This is
a capability gap rather than a defect, but it dominates the differential's
rejection counts and it caps what lccc can emit on any Neoverse/Grace-class
target.

### F10 — PC-relative instructions reject bare immediates that GAS accepts

`bl 0xfffffffffd575388`, `adrp x10, 0xd828b000`, `cbnz w13, 0x…`,
`ldr w12, 0xe610`, `prfm #0x1f, 0x…` are all accepted by GAS and rejected by
lccc with "expected symbol at operand N" / "needs symbol operand". lccc requires
a *symbol* where GAS accepts a raw immediate. defensible for a
compiler-internal assembler, but it is a real divergence from the assembler
lccc is otherwise trying to be syntax-compatible with.

### F11 — the differential harness leaked ~0.8 GB per killed run

Not a compiler defect, but it filled the root disk mid-session and produced
bogus "No space left on device" failures. `tmpdir` cleanup moved into a
`finally` block.

---

## 9. The new deliverable: `scripts/aarch64_encoder_differential.py`

This is the "an order of magnitude better" piece, and it replaces a whole
methodology rather than adding tests.

### The problem with the previous approach

The upstream `claudes-c-compiler` fork accumulated ~330 AArch64 encoder bug
reports and ~87 000 lines of property tests. Every one of them is a *guess* at
what an encoder ought to do, written by someone who already knew the answer —
which is why they found 330 bugs and not the rest, and why they cost 87 000
lines to maintain. The prior session's plan (six oracle-free invariants applied
to 189 `encode_*` functions) is the same idea with better tooling.

### The replacement

Do not have an opinion about AArch64. Generate **uniformly random 32-bit
words**, ask GNU `objdump` which of them decode, and feed the resulting
canonical text to lccc's built-in assembler and to GNU `as` in parallel. Any
difference is a bug *by construction* — the oracle is binutils.

Three properties make this strictly better than hand-written tests:

1. **The generator is a second implementation.** No per-mnemonic grammar, no
   operand templates, no maintenance as the ISA grows. It reached SVE, SME,
   AdvSIMD, predicated and scaled forms and the odd system encodings on the
   first run, in proportion to their share of the encoding space.
2. **It is unbiased.** 37.4% of random words decode to valid instructions, so
   coverage follows the real encoding density rather than the author's mental
   model of it.
3. **It cannot go quietly stale.** The harness self-tests before every run: a
   known-good instruction must encode identically, and a known-bad one must be
   rejected by both sides. A differential tester whose oracle has stopped
   working reports "0 mismatches" forever — the most dangerous failure mode for
   this class of tool — and F8 above is exactly how that self-test earned its
   keep on its first run.

Two implementation details carry most of the value:

- **Address pinning with `.org`.** `objdump` renders PC-relative operands as
  *absolute* targets derived from each instruction's address. Re-assembling
  that text at a different address changes the required displacement and can
  push it out of range, which would silently destroy coverage for the entire
  branch/literal family. Every instruction is re-emitted at its original address
  with `.org` bridging the gaps, so every displacement is exactly reproducible.
  (Verified: lccc's `.org` zero-fills exactly like GAS, so the padding is not
  itself a divergence.)
- **Recursive bisection.** A batch of 64 in which one instruction is
  unsupported costs `log2(64)` extra invocations, not the whole batch.

### Results

| | seed 1 | seed 7 |
|---|---:|---:|
| random words | 60 000 | 60 000 |
| decoded to real instructions | 22 426 (37.4%) | — |
| instructions tested | 6 940 | 6 975 |
| distinct mnemonics seen | 837 | 813 |
| **byte-identical (OK)** | **2 804** | **2 818** |
| **MISENCODE** | **0** | **0** |
| LCCC rejects (coverage gap) | 3 371 | 3 377 |
| GAS rejects | 26 | 36 |
| both reject | 739 | 744 |
| lccc mnemonic coverage | 282/431 (65.4%) | 284/431 (65.9%) |

Reproduce:

```bash
python3 scripts/aarch64_encoder_differential.py --n 60000 \
    --prologue $'.arch armv9.4-a+sme\n' -o report.json
```

Requires `binutils-aarch64-linux-gnu`. Exits non-zero on any misencoding, so it
can be wired straight into CI.

**Reading the result honestly.** Zero misencodings across ~5 600 instructions
and two independent seeds is strong evidence that lccc's AArch64 encoder is
correct for the subset it implements. It is *not* evidence that the encoder is
complete: 3 371 rejections and 65% mnemonic coverage say the opposite. The tool
reports both numbers precisely so that "0 mismatches" can never again be
mistaken for "fully tested".

### Why this beats the fork's corpus, concretely

The fork's 330 reports were found by guessing; this finds defects by
construction and keeps finding new ones after the known set is empty, at a cost
of ~650 lines instead of ~87 000. It also produced F8, F9 and F10 above on its
first run — none of which are in the fork's corpus.

---

## 10. Validation

| gate | result |
|---|---|
| `cargo build --profile fastbuild` | green |
| `cargo test --lib` | **4 051 passed / 0 failed / 7 ignored** (was 4 044; +7 new) |
| `cargo test --lib branch_range` | 11 passed (4 ARM pre-existing + 7 new RISC-V) |
| `scripts/aarch64_encoder_differential.py` × 2 seeds | 0 misencodings |

The +7 is the new RISC-V test module; nothing regressed.

---

## 11. Remaining work, in priority order

1. **F8** (accepts invalid assembly) — needs a maintainer decision on whether
   lccc's assembler is internal or user-facing, then a strictness fix.
2. **F9** (no SVE/SME) — the single largest capability gap on AArch64; ~150 of
   lccc's 431 mnemonics are still unsampled, largely for this reason.
3. **F10** (PC-relative bare immediates) — cheap to fix, improves
   source-compatibility with GAS significantly.
4. Raise differential coverage past 65%: add a targeted second phase that
   searches for encodings of the ~149 unsampled mnemonics, rather than waiting
   for uniform sampling to find them.
5. Wire the differential into `ci_local.sh` as a gate once its runtime is
   bounded (~2.5 min at the settings above).
