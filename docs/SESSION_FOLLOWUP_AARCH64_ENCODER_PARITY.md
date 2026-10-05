# AArch64 encoder parity: the operand-legality matrix and the generated sysreg table

**Status:** landed on `work` on top of PR #762; `ci_local.sh --fast` green (rustfmt + clippy included).
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
| `tests/aarch64/operand-legality.tsv` (**6817 rows: 5564 accepted / 1253 rejected**) | every row's verdict is GNU as's own; `operand_legality_matrix_matches_the_encoder` (`include_str!`) pins the encoder both ways | `python3 scripts/aarch64_operand_legality_matrix.py --check-lccc target/fastbuild/lccc --objcopy <distro> --as <pinned>` |
| `src/backend/arm/assembler/encoder/sysreg_table.rs` (**1619 names**, generated from `opcodes/aarch64-sys-regs.def`) | `.def` ↔ pinned GNU as ↔ generated table ↔ encoder words, both directions | `python3 scripts/aarch64_sysreg_table.py --check --as <pinned> --objcopy <distro> --lccc target/fastbuild/lccc` |

The recorded matrix runs covered the 6,347-row inventory then in use: `--check`
reported **6347/6347** agreement with pinned `as` (42 s), and `--check-lccc`
reported **6347/6347** agreement with the encoder (34 s in `ci_local.sh`). The
checked-in fixture now includes the load/store sweep and totals **6,817 rows
(5,564 accepted / 1,253 rejected)**; CI should recheck the expanded inventory.
The sysreg table check reported **1619/1619** (4 s). `ci_local.sh --fast` runs
the three AArch64 parity gates `aarch64-operand-legality`,
`aarch64-operand-legality-encoder` and `aarch64-sysreg-table`.

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
| `cargo test --profile fastbuild --locked -j2 --lib` | **4126 passed, 0 failed, 7 ignored** |
| matrix vs pinned `as` (`--check`) | **6347/6347 on the recorded 6,347-row inventory; current 6,817-row inventory awaits CI** |
| matrix vs encoder (`--check-lccc`) | **6347/6347 on the recorded 6,347-row inventory; current 6,817-row inventory awaits CI** |
| sysreg table (`.def`/`as`/table/encoder) | **1619/1619** |
| `check_ci_gate_parity.py` + `test_ci_gate_parity.py` | PASS; 66 tests OK |
| `ci_local.sh --fast` | see the run log behind `target/ci_local.pass` |
| `cargo fmt --all -- --check`, clippy | clean |

## 5. Follow-ups (program items, unchanged by this stretch)

1. **Godbolt-oracle evidence** for the AArch64 encoder work: per-test hard data against
   GCC 16.2 / Clang 23.1 / ICX, "beat the best of their solutions", no guesswork.
2. **Red-team audit** of the AArch64 encoder changes, all open PRs and the last 10 closed PRs, with a
   `docs/REVIEW_<n>_ADJUDICATION.md` (salvage / agree / disagree, quantified, empirical).
3. **archpkgbuilds benchmark extraction** (gzip, zlib-ng, expat, SQLite, kernel, glibc) into
   the integrated benchmark section over the lccc 39-corpus.
4. **Linker/mold-i686 oracle** honouring user build/version prefs — now unblocked locally by
   the 32-bit toolchain (mold's `X86` build preset).
5. The `ldr/str byte forms` group is curated; the sign-extending trio (`ldrsb/ldrsh`,
   `ldrsw`) is gated by the same group, but a *generated* sweep of (size × addressing mode ×
   register) for the whole load/store space would pin the remaining combinations the way
   `_sweep_sysreg` pins the system registers.
