# AArch64 encoder parity: the operand-legality matrix and the generated sysreg table

**Status:** merged upstream as part of PR #765 and extended by PR #766; `ci_local.sh --fast`
green (rustfmt + clippy included).

> **Revision note (2026-10-06).**  The counts here are the shipped ones: the
> table is **10411 rows in 55 groups** and the sysreg table **1619 names**.  Two
> corrections to what this document first recorded: the hosted matrix step is
> provisioned with the *pinned* pair again (it had been switched to the runner's
> distro `as`, which disagrees with 1077 rows and made the gate red), and the
> verification rows below are re-derived in
> `docs/VERIFICATION_AARCH64_MATRIX_2026-10-06.md`, which carries the commands,
> the oracle's provenance and the raw output.  Rows marked *author-measured*
> there are the ones this document recorded; everything else was re-run.
**Audience:** anyone touching the AArch64 assembler encoder, the differential tooling
around it, or the CI mirrors that provision its oracles.

---

## 1. What the two instruments are, and what they are for

The AArch64 encoder has two structural blind spots that no compiler test catches, because
both produce a *plausible* instruction stream:

1. **a legal instruction we reject** (`str b9,[x10]` is legal and we refused it), and
2. **an illegal instruction we accept** (`str v9,[x10]` is *not* an encoding and we emitted
   `str q9`'s word for it).

The differential oracle cannot see either one: it decodes what we emit, so everything it
compares is a valid encoding by construction.  The instruments that can:

| instrument | what it pins | invocation |
|---|---|---|
| `tests/aarch64/operand-legality.tsv` (**10411 rows; the exact per-group counts are the 55-line ratchet table in `elf_writer.rs`**) | every row's verdict is GNU as's own; `operand_legality_matrix_matches_the_encoder` (`include_str!`) pins the encoder both ways | `python3 scripts/aarch64_operand_legality_matrix.py --check-lccc target/fastbuild/lccc --objcopy <distro> --as <pinned>` |
| `src/backend/arm/assembler/encoder/sysreg_table.rs` (**1619 names**, generated from `opcodes/aarch64-sys-regs.def`) | `.def` ↔ pinned GNU as ↔ generated table ↔ encoder words, both directions | `python3 scripts/aarch64_sysreg_table.py --check --as <pinned> --objcopy <distro> --lccc target/fastbuild/lccc` |

The verdicts are asserted, not quoted: `--check` re-derives every row from the pinned
`as`, `--check-lccc` re-derives them from this crate's encoder, and both must agree with the
checked-in table exactly -- including that the table IS the generator's output, key for key.
`ci_local.sh --fast` runs them as `aarch64-operand-legality`,
`aarch64-operand-legality-encoder`, `aarch64-sysreg-table` and `aarch64-legality-probe`.

The verdict authority is the **pinned 2.47 `as`**
(`$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as`, `scripts/ensure_gas_247.sh
aarch64-linux-gnu`), never a distro `as`: Debian's 2.44 disagrees with 1077 rows that 2.47
accepts, so a distro assembler turns an agreement gate into a red one *and* would have
hidden the `tco`/`afgdtp0_el1`/`actlrmask_el1` classes of names entirely.

## 2. Defects found and fixed in this stretch

### 2.1 Byte/halfword load-store legality is keyed on the MNEMONIC, not the operand class

`str b9,[x10]` and `strb b9,[x10]` take the same operands and only the first is legal:

| spelling | GNU as 2.47 | note |
|---|---|---|
| `str b9,[x10]` | `0x3d000149` | legal — the FP byte access |
| `str h9,[x10]` | `0x7d000149` | legal |
| `str s9/d9/q9` | `0xbd000149` / `0xfd000149` / `0x3d800149` | legal |
| `str v9,[x10]` | **rejected** | bare `v` is not an operand; write `q0`-`q31` |
| `strb b9` / `strb h9` / `strb x9` | **rejected** | `strb`/`strh`/`ldrb`/`ldrh` take `w` only |

The encoder tested the *operand class* (`size 00/01 && fp ⇒ error`), so it rejected the two
legal FP byte/halfword stores and accepted `str v9` (emitting `str q9`'s word).  The rule is
now mnemonic-keyed: `encode_ldr_str(..., gpr_byte = true)` for `ldrb/strb/ldrh/strh`
(`w0`-`w30`/`wzr` only), `false` for the auto `ldr/str` path, where `b/h/s/d/q` is the FP
access and it must be 128-bit only for `q`.  61 rows were added as the `ldr/str byte forms`
group so an operand-class keyed regression cannot come back silently.

### 2.2 The system-register resolver was case-SENSITIVE one layer below its callers

GNU as resolves register names case-insensitively, in both directions and in both spellings
(measured, 2.47):

```
mrs x0,DBGBVR7_EL1   -> 0xd5300780      msr DBGBVR7_EL1,x0  -> 0xd5100780
mrs x0,DbgBvr7_El1   -> 0xd5300780      msr PAN,x0          -> 0xd5184260
mrs x0,S3_0_C1_C0_1  -> 0xd5381020      msr S3_0_C1_C0_1,x0 -> 0xd5181020
```

`encode_mrs`/`encode_msr` lower-cased the *operand*, but `sysreg_encoding_named` itself
compared bytes, so the guarantee lived in the callers: a third caller (or an upper-case raw
spelling reaching the resolver directly) disagreed with GNU as.  The resolver now folds case
itself — an allocation-free ASCII-folded comparator over the generated table, plus
case-insensitive `s`/`c` prefixes in `parse_generic_sysreg` — and the matrix gained the
rows above, **including upper-case out-of-range probes** (`S4_0_C1_C0_1`,
`S3_8_C1_C0_1`, `S3_0_C16_C0_1`, `S3_0_C1_C0_8`) that GNU as rejects and we still must: a
case fold must never be a way past a range check.

### 2.3 Every hand-derived expectation in the new unit tests was wrong

The three 16-bit field constants written by hand in `sysreg_table_tests` disagreed with the
assembler's own words and were caught only because the *encoder* was right:

| register | hand-written | GNU as word | field |
|---|---|---|---|
| `dbgwvr15_el1` | `0x80de` | `0xd5300fc0` / `0xd5100fc0` | **`0x807e`** |
| `dbgbvr7_el1` | `0x803d` | `0xd5300780` | **`0x803c`** |
| `s3_0_c15_c15_7` | `0xffff` | `0xd538ffe0` | **`0xc7ff`** |

All three are corrected, the GAS word is cited next to each constant so the arithmetic is
checkable where it is read, and the whole module was then cross-checked programmatically
against the matrix (`sysreg_encoding_named`, `enc_mrs`, `enc_msr` assertions: **every one
matches the GAS-derived row**).  The standing rule that follows from this: **a verdict is
never arithmetic performed in a comment** — it is read out of the assembler and pinned by a
matrix row.  (`0x80de` was even inconsistent with the mnemonic word quoted beside it.)

### 2.4 The local mirror preferred a pinned oracle nothing local provisioned

`ci_local.sh` resolved `$A64_AS` from `$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as` and
*fell back to the distro `as`* when it was absent — but no local gate installed it, so a
clean box silently graded itself against 2.44.  The mirror now has the missing provisioning
gate:

```
gate "a64-gas-provision" fast \
    bash scripts/ensure_gas_247.sh aarch64-linux-gnu
```

Hosted CI got the matching step (pinned 2.47 `as` for AArch64 differential) and the new
system-register-table gate, and `scripts/check_ci_gate_parity.py` registers the
`gas-provision-aarch64` invocation contract (`aarch64-linux-gnu`), so the two mirrors can no
longer drift apart in *either* direction.  Both parity suites are green
(`check_ci_gate_parity.py` PASS, `test_ci_gate_parity.py` 66 tests OK).

### 2.5 The scalar SIMD&FP conversions were a whole missing family

Every `fcvt*` mnemonic has **two** forms, and the operand spellings decide which one was
written: the general-purpose-destination form (`fcvtms w0,s1`, `scvtf s0,w1`) and the
scalar register-file form (`fcvtms s0,s1`, `scvtf s0,s1`).  The encoder only knew the first,
so twelve mnemonics' worth of legal spellings were refused — and, in the other direction,
`fcvt s0,s1` was *accepted*: `encode_fcvt_precision` read the two width letters
independently, so all three same-width spellings (`h0,h1`, `s0,s1`, `d0,d1`) assembled to a
word that `objdump` prints as `.inst`, i.e. an undefined encoding.  GNU as refuses all three
("operand mismatch"), because `fcvt` *converts* between widths and has no same-width form.

| spelling | GNU as 2.47 | we emitted (before) |
|---|---|---|
| `fcvtms s0,s1` | `0x5e21b820` | refused |
| `fcvtzu d0,d1` | `0x7ee1b820` | refused |
| `fcvtms h0,h1` | `0x5e79b820` | refused |
| `fcvtzs s0,s1,#3` | `0x5f3dfc20` | refused |
| `ucvtf h0,h1,#16` | `0x7f10e420` | refused |
| `fcvt s0,s1` | **rejected** | `0x1e224020` (`.inst`) |

The register-file form does not carry a `rmode` field: the rounding mode is baked into the
opcode, so each mnemonic has a base word of its own, and the `h` element has a second base
because its element-size field is not the `s`/`d` pair's `size` bit.  The bases are therefore
taken from binutils' own encoding table (`opcodes/aarch64-tbl.h`: `SIMD_INSN`/`SF16_INSN`
with class `asisdmisc`, qualifier `QL_S_2SAMESD`/`QL_S_2SAMEH`), whose masks —
`0xffbffc00` for the `s`/`d` base, `0xfffffc00` for the `h` one — say exactly what may vary:
bit 22 (`size`) and the two register fields.

The four integer-conversion mnemonics also have a **fixed-point** form,
`fcvtzs s0,s1,#fbits`, whose bit count ranges over the *element* width
(1..=16 / 1..=32 / 1..=64) and whose field is the complement: `64 - fbits` for `s` and `d`
(with the element size as that field's top bit, so `d` reads `0x40 | (64 - fbits)`) and
`32 - fbits` for `h`.  The other eight rounding mnemonics have no such form, and saying so
is what GNU as does (`fcvtas s0,s1,#3` is refused).

Verification is exhaustive rather than sampled: a 1018-spelling differential (twelve
mnemonics × three widths × four register pairs, every mixed-width pair, every `fbits` in
`0..=width+1` for the four fixed-point mnemonics, the eight "no fixed-point form" rejections,
the vector/`q`/`b`/`x` class rejections, and the general-purpose-destination forms that must
keep working) reports **0 disagreements**, and the matrix carries permanent rows for the same
space.

### 2.6 The exclusive/acquire-release width rule was keyed on a family name

`exclusive_rt` enforces "a trailing `b`/`h` fixes the transferred register at 32 bits"
(`ldxrb x0,[x1]` has no encoding) — but its callers passed `"ldxr"`, `"stxr"`, `"ldaxr"`,
`"stlxr"`, `"ldar/stlr"`, so `size_suffix_forces_w` never saw a suffix and the rule applied
only where a `forced_size` argument happened to exist.  `ldaxrh xzr,[x1]`,
`stxrb wzr,x1,[x2]`, `ldarh lr,[x1]` and their relatives were accepted and assembled as their
32-bit forms with a 64-bit register's number in a 32-bit field.  The fix is to thread the
*spelling* through (`mn: &str`) instead of a family name, which also makes every diagnostic
name the mnemonic the programmer wrote.  The same argument applies to the pair loads/stores:
`ldxp/ldaxp/stxp/stlxp` have one `sz` field for both halves, so `ldaxp w0,x1,[x2]` used to
assemble as the 64-bit pair; the two halves are now required to select the same width.

### 2.7 ADC/SBC and the conditional-select aliases read their slots permissively

`adc`/`sbc` (and `adcs`/`sbcs`) have one `sf` for `Rd, Rn, Rm` and read encoding 31 as the
zero register in every slot.  All four facts were fail-open through the permissive `get_reg`
reader: `adc x0,x1,sp` assembled as `adc x0,x1,xzr`, `adc sp,x1,x2` as `adc xzr,x1,x2`, and
`adc x0,x1,w0` (a 32-bit third operand) by ignoring every width but the destination's.  The
same reader sat behind `cset`/`csetm`/`cinc`/`cinv`/`cneg`, where the destination is an
`Rd|XZR` slot (`cset sp,eq` assembled as `cset xzr,eq`) and the two register operands share
one width (`cinc w0,x1,eq` silently encoded the 64-bit form).  All of them now go through
`gp_same_width`/`reg_operand`, so the class and the width are checked together and the error
names the operand.

### 2.8 CASP's pair halves: the successor rule and the zero-register spelling

The four register operands of CASP are two *pairs*, and only the first half of each is
encoded.  GNU as therefore requires the second half to be the successor of the first
("reg pair must be contiguous"), and accepts it written either as the register
(`casp x0,x1,...`) or — when the pair starts at 30 — as the **zero register**, whose number
is 31 (`casp x30,xzr,...`, `0x483e7c80`).  Reading all four slots as plain numbered registers
refused the latter, a spelling the kernel's `cmpxchg_double` paths produce.  Both halves are
now validated against the pair's first register (including a width check across all four),
and `casp` rows for the second half of each pair are in the matrix so the rule cannot rot.

## 3. Host prerequisites (measured, not folklore)

A `--fast` run on a box without the 32-bit oracle is **red, not green**: `reassoc-latency`
(`-m32 sha256_transform reshaped`), `regression-corpus-link` (6 cases `compile:fail`),
`redundant-test-elimination`, `i686-integer-isa-parity`, `map-i64-two-lane` and
`linker-suite` all fail rather than skip — by design, because a leg that cannot run must not
report coverage.  `scripts/arena_session_restore.sh` step 3 is the provisioning path
(`gcc-multilib g++-multilib libc6-dev-i386`, plus kernel tooling).  After it: all six gates
green, and the i686 legs actually execute — `regression-corpus-link` **887 passed / 0
skipped**, `linker-suite` **302 pass / 0 fail / 0 skip**, `reassoc-latency` reporting the
full residency verdict (`-m32 SHA-256/rot() left alone, -m32 bitops and x86-64 rot()
rewritten`).

## 4. Verification evidence for this stretch

| check | verdict |
|---|---|
| `cargo test --profile fastbuild --locked -j2 --lib` | **4131 passed, 0 failed, 7 ignored** |
| matrix vs pinned `as` (`--check`) | **10411 rows agree** |
| matrix vs encoder (`--check-lccc`) | **10411 rows agree** |
| scalar-conversion sweep vs pinned `as` (1018 spellings) | **0 disagreements** |
| CASP pair sweep vs pinned `as` (228 spellings) | **0 disagreements** |
| sysreg table (`.def`/`as`/table/encoder) | **1619/1619** |
| `check_ci_gate_parity.py` + `test_ci_gate_parity.py` | PASS; 66 tests OK |
| `ci_local.sh --fast` | see the run log behind `target/ci_local.pass` |
| `cargo fmt --all -- --check`, clippy | clean |

## 5. Follow-ups (program items, unchanged by this stretch)

1. **Godbolt-oracle evidence** for the AArch64 encoder work: per-test hard data against
   GCC 16.2 / Clang 23.1 / ICX, "beat the best of their solutions", no guesswork.
2. **Red-team audit** of `ms178-1.patch`, all open PRs and the last 10 closed PRs, with a
   `docs/REVIEW_<n>_ADJUDICATION.md` (salvage / agree / disagree, quantified, empirical).
3. **archpkgbuilds benchmark extraction** (gzip, zlib-ng, expat, SQLite, kernel, glibc) into
   the integrated benchmark section over the lccc 39-corpus.
4. **Linker/mold-i686 oracle** honouring user build/version prefs — now unblocked locally by
   the 32-bit toolchain (mold's `X86` build preset).
5. The `ldr/str byte forms` group is curated; the sign-extending trio (`ldrsb/ldrsh`,
   `ldrsw`) is gated by the same group, but a *generated* sweep of (size × addressing mode ×
   register) for the whole load/store space would pin the remaining combinations the way
   `_sweep_sysreg` pins the system registers.

## 6. Environment recovery (why the ratchet test earned its keep)

The Arena harness reset the root filesystem mid-stretch.  The worktree survived except for

* `.git` itself (restored from the persisted bundle: `git clone --no-checkout
  artifacts/lccc.bundle` → `cp -a .git` → `git remote set-url origin
  https://github.com/ms178/lccc.git` → `git symbolic-ref HEAD refs/heads/merged` →
  `git reset --mixed`, which leaves every file in place),
* `src/backend/x86/linker/emit_exec.rs`, where the reset deleted the ICF/TLS
  `dead_sections` guard (`git checkout --` restores it from the index), and
* the executable bit on 393 tracked scripts — the wipe materialises files 0644, so
  `git ls-files --stage | awk '$1=="100755"{print $4}' | xargs chmod +x` is the repair, and
  without it `scripts/ensure_gas_247.sh` exits 126 and the gates SKIP.

The oracles are not persisted (`.cache/`, `~/.cargo`, `~/.rustup`, `/swapfile` are all
outside the snapshot), so `scripts/ensure_gas_247.sh <triple>` and a rustup stable install
with `rustfmt`+`clippy` are part of the recovery, and `scripts/ensure_swap.sh` recreates the
swap file (`fallocate -l 8G /swapfile`; the harness VM has 2 GiB of RAM and the Rust
bootstrap link exceeds it).

One silent casualty was worth the whole exercise: the wipe had reverted the
`GROUP_RATCHETS` total in `elf_writer.rs` to the *previous* matrix size while the table kept
the newer one, so `operand_legality_matrix_matches_the_encoder` failed with
"the matrix has 9702 rows, the ratchet pins 8229" — a class of drift that no eye would have
caught in a diff, and exactly what the two-sided ratchet (table-side count and
generator-side count) exists to catch.
