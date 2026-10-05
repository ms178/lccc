# Follow-up: PR #762 fail-closed completion (2026-10-05)

This session's base is `pr762` (`efe65d7e`) on `ms178/lccc` main
`c59c227f` (PR #758). The older `ms178-1.patch` in
`archpkgbuilds/toolchain-experimental/claudes-c-compiler/` **is PR #762**
(11 files, +2563/−339). It is not the PBT campaign.

## 1. Red-team audit of the older patch / PR #762

**Agree with #762's architecture. Do not replace it. Complete it.**

`GpReg` / `GpRole` / `reg_operand` is the right single source of truth for
register 31. A class check ("is this a GP register?") cannot distinguish
SP from ZR; collapsing a spelling to a number before the form is known
turns an invalid operand into a *different valid instruction*. The
dual-gated operand-legality matrix (table↔GAS, encoder↔table) is the
correct instrument for the reject-parity bug class the differential
word oracle is structurally blind to.

**Where #762 is incomplete (live, oracle-characterized on this tree
before the fixes below):**

| Family | Defect | Class |
|---|---|---|
| `ld2r` / `ld4r` | R bit placed in S (bit 12) instead of bit 21 | **undefined instruction words** 0x4d40d000 / 0x4d40f000 vs GAS 0x4d60c000 / 0x4d60e000 |
| MOVI/MVNI | parser did not recognise `msl`; `.4h/.8h` `lsl #8` stuck at cmode 1000; unknown shifts silently dropped | silent misencode |
| EXT | `index & 0xF` wrap; no arrangement check | `#16` assembled as `#0` |
| ADDV | `.2d` encoded an unallocated word | GAS rejects |
| TBL/TBX | empty `RegList` panics; `len = (n-1) & 3` wraps 5-reg tables | panic / misencode |
| PRFM register | opc shifted `<< 23` instead of `<< 22` | 0xf9206800 vs GAS 0xf8a06800 |
| STR/LDR Rt | `str sp,[x0]` stored XZR | wrong-code |
| LDRSW | W destination accepted | wrong-code |
| LDUR/LDTR | imm9 masked | `#256` → `#-256` |
| CAS/SWP/LDXR/… | nonzero offset dropped | `[x2,#8]` assembled as `[x2]` |
| SVC/BRK/MSR/HINT/SYS | fields masked | `#65536` → `#0` |
| MRS/MSR/DC/IC | W/FP Rt accepted via `parse_reg_num` | `mrs d0,fpcr` wrote X0 |
| MOVZ/MOVK/MOVN | still used `get_reg` (FP/SP) and `& 0xFFFF` | `movz d0,#1`, `movz w0,#0x10000` |
| `.p2align N,,M` | max-padding ignored | 12 NOP bytes where GAS pads 0 or 4, shifting later labels |
| hot path | `parse_reg_num` / `is_64bit_reg` / `encode_cond` still allocated | measured −1.954% Ir win left on the table |

#762's own data-processing/fp-scalar/bitfield work is correct and was
not re-litigated. The fail-open remainder was load_store / neon / system
plus the MOV-wide leaves.

**Rating of #762 as-is: ~6.5/10 correctness** (right architecture, live
undefined-word encoders). After this session's completion: the families
above are fail-closed and pinned.

## 2. Open PRs

| PR | Verdict |
|---|---|
| **#762** | **Base. Keep. Complete.** Slim path, no PBT campaign. |
| **#759** | NAK as a merge. 208 files, +39k. Encoding insights (ldnr R bit, movi cmode, ranges) salvaged and re-implemented in #762's architecture. The PBT campaign, 149 ignores, false witnesses, and `pub` widenings are the design mistake. |
| **#756** | Superceded by #762. Smaller operand-validation without GpRole. Close. |
| **#751** | **Salvaged.** Real lccc-ld ICF bug: folded TLS twins re-apply sequence-shaped GD/LD→LE rewrites onto the survivor and die. The `dead_sections` skip is in this tree. Test-harness i386 SKIP-ENV changes were **not** taken (environmental, not the bug). |
| **#750** | Superceded by #751 (same ICF idea, less refined tests). Close in favor of #751's encoder hunk, now landed here. |
| **#725** | Independent x86 flag-flow / volatile RMW lane. Not mixed into this assembler PR. Worth a dedicated review; do not piggy-back. |

## 3. Last 10 closed PRs

| PR | State | Takeaway |
|---|---|---|
| #761 | closed | Fail-closed + PBT. Encoder deltas overlapping #759; campaign still the problem. Insights taken, campaign not. |
| #760 | closed | #759 + session-3 hardening. 149 ignores, false MOVI/sqshrun/mov_dispatch contracts. **Do not import.** |
| #758 | **merged** (main) | record-tag identity, pointer-depth, DWARF, fptr-param. Already in the base. |
| #757 | closed | libcall / NOBITS / array bounds / RA-01. Different lane. |
| #755 | closed | earlier PBT hardening. Superceded. |
| #754 | **merged** | checked AArch64 offsets, pair classification, fmov/#fbits. Already in main; this session builds on it (`checked_imm9` now used by LDUR/LDTR). |
| #753 | closed | libcall gates / option contract. Different lane. |
| #752 | **merged** | fail-closed AArch64, RISC-V branch, x87 exp. Already in main. |
| #749 | closed | PBT encoder validation. Superceded. |
| #748 | closed | RISC-V / long-double / AArch64 differential oracle. Oracle infrastructure already on main. |

**Disagree with importing the PBT campaign (#759/#760/#761).** It is a
trustworthiness defect (false witnesses that assert unallocated encodings
are correct; 23 stale ignores; zero oracle pins for the new fixes). The
encoding *fixes* inside it are real and are now in #762's architecture
without the campaign.

## 4. What this session landed (on top of #762)

1. **NEON**
   - LD2R/LD4R R-bit (undefined words → GAS words 0x4d60c000 / 0x4d60e000)
   - Shared MOVI/MVNI cmode table including `msl` and 16-bit-lane shifts
   - Parser recognises `msl`
   - EXT per-arrangement index; ADDV allocated arrangements only
   - TBL/TBX empty-list + 1..=4 table size
2. **load/store**
   - SP is never Rt of LDR/STR (`str sp` rejected, not XZR)
   - `mem_xn_or_sp` / `atomic_xn` (W-base and nonzero atomic offsets rejected)
   - LDRSW requires Xt; LDUR/LDTR use `checked_imm9`
   - PRFM register opc `<< 22` (0xf8a06800)
3. **system**
   - SVC/HVC/SMC/BRK/HINT/MSR-imm/SYS range-checked, not masked
   - MRS/MSR/DC/IC/AT/TLBI Rt is Xt via `gp_xt`
4. **MOV-wide** uses `GpRole::RegOrZr` + 16-bit immediate range
5. **`.p2align N,,M`** max-padding honoured (`align_to_capped`)
6. **Allocation-free** `parse_reg_num` / width predicates / `encode_cond`
7. **lccc-ld ICF** skip of folded-section relocation replay (#751)
8. **Unit test** `fail_closed_families_match_gas_pins` — GAS-pinned words
   plus rejection contracts. Lib suite: **4122 passed, 0 failed, 7 ignored**.
   Clippy `-D warnings` clean. rustfmt clean on touched files.
9. **Pair / Rm / PRFM / CASP fail-closed remainder**
   - `pair_reg_fields` rejects `sp`/`wsp` (was `ldp sp,x1,[x0]` → XZR)
   - `mem_rm` on every register-offset Rm (STR/LDR/LDRSW/LDRS/PRFM)
   - PRFM base is `mem_xn_or_sp`; CASP base is `atomic_xn`
   - `expect_operands` helper; `ext` migrated
10. **ARM codegen assembler-parity** `scripts/check_arm_codegen_assembles.sh`
    wired into `ci_local.sh --fast`. This host: **assembled 14 / GAS-compared 0**
    (no `aarch64-linux-gnu-as`). Integrated assembler accepted every
    `tests/regression/arm_*.c` `-O2` output.
11. **Godbolt + Callgrind** (see §7). Distilled kernels:
    `tests/benchmark/programs/{crc32_nibble_hot,adler32_do8_hot}.c`.

## 5. Remaining work (priority order)

1. ~~Wire `atomic_xn` / `mem_xn_or_sp` through pair / register-offset.~~ **Done**
   (`mem_rm`, pair Rt, PRFM, CASP).
2. ~~**Regenerate the operand-legality TSV** against GAS 2.47.~~ **Done.**
   `ensure_gas_247.sh aarch64-linux-gnu` → GNU as **2.47.20260726**.
   Regenerated **468 rows (271 OK / 197 REJECT)**; `--check` 468/468 vs that
   pair. llvm-mc **23.1.2** (GitHub `llvmorg-23.1.2`) is the second oracle:
   it **agrees** on `ldr … lsl #1` reject, `mov w0,#0x100000000` (GAS reject;
   llvm-mc **truncates to #0** — we match GAS, we do not copy llvm-mc's
   truncate), `and w0,w1,#0x100000001` reject, `casp d0,…` reject.
   **Split:** `uxtw x0,w1` GAS=`mov` `0x2a0103e0`, llvm-mc=`ubfx` `0xd3407c20`.
   Encoding pin is GAS (GNU assembler contract). `--check` still trips if
   the table claims the llvm word.
3. **Dispatcher-coverage ratchet**: more `assemble()` goldens; this is where
   `fmls` hid. Pair/Rm/PRFM rejects are now in `fail_closed_families`.
4. ~~`expect_operands`.~~ **Added**; migrate remaining neon arity sites.
5. **Migrate `get_neon_reg` / `get_imm` / remaining `get_reg` sites** off
   allocating paths, file by file.
6. ~~ARM codegen assembler-parity gate.~~ **Landed**; byte-compare still
   SKIP until GAS 2.47 is on PATH.
7. **#725** as its own PR after a flag-flow audit. **Not mixed.**
8. **Do not** re-import the PBT campaign.
9. **Godbolt follow-through**: LCCC vectorizes Adler-32 (AVX2) with a 272-byte
   spill frame — insn-count loses to ICX; CRC32 scalar loop uses indexed
   `(%rsi,%r9)` instead of GCC's pointer IV (`addq $1,%rsi`). Next
   performance increment is LSR / pointer-IV, not more assembler work.
10. ~~**`ci_local.sh --fast`**.~~ **Green on this host after gcc-multilib /
    g++-multilib** (the first pass's 6 failures were `-m32` / `-lgcc` /
    `bits/c++config.h`, not this tree). cargo-test 540s, clippy 124s,
    rustfmt, 889-link corpus, linker-suite 302, arm-codegen-assembler-parity
    14 assembled. Slow gates still skipped as required.

## 7. Oracle measurements (2026-10-05)

Godbolt `scripts/godbolt.py audit`: gcc→16.2, clang→23.1.0, icc→2021.10.0,
icx→latest — all pinned aliases current.

`compare` `-O3 -march=x86-64-v3`, function insn counts:

| Kernel | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX |
|---|---:|---:|---:|---:|---:|
| `gzip_crc32_update` (nibble table) | **15** | 15 | 49 | 35 | 64 |
| `adler32_len` (DO8) | 132 (AVX2) | 99 (scalar) | 86 | 74 | **73** |

Callgrind (crc32 driver, 200×4 KiB, cache+branch sim, valgrind 3.24.0):

| | Ir | Dr | Dw | Bcm |
|---|---:|---:|---:|---:|
| GCC 16.2 | 6 675 696 | 1 665 143 | 13 022 | 2 925 |
| LCCC | 7 499 624 | 1 665 142 | 13 149 | 2 930 |

Same exit code (189). LCCC +12.3 % Ir: extra induction register + indexed
load vs GCC's pointer-end compare. **Not** a silent-miscompile.

Valgrind was not in the image; installed 3.24.0. `libc6-dbg` 404 from the
Debian mirror — callgrind still ran.

## 8. Audit of the "UXTW is a live encoder bug" claim (2026-10-05)

An external review treated TSV rows `uxtw x0,w1` / `uxtw w0,w1` → `OK e003012a`
as circular (encoder-derived) because ARM ARM / llvm-mc encode
`uxtw x0,w1` as `UBFM Xd,Xn,#0,#31` (`0xd3407c20`) and reject a W dest.

**Disagree, with a live GNU as measurement.** Debian `aarch64-linux-gnu-as`
2.44:

```
uxtw x0, w1              ->  0x2a0103e0  (== mov w0, w1)
uxtw w0, w1              ->  0x2a0103e0
ubfm x0, x1, #0, #31     ->  0xd3407c20  (the ARM/llvm-mc word)
uxtw x0, x1              ->  REJECT
```

The table matches GAS. The encoder matches GAS. llvm-mc is the wrong
authority for this project's reject-side oracle (already documented: LLVM
wraps `ext #16` etc.). Sibling-fork issue #312 is a PBT-vs-llvm-mc false
witness of the same class.

**Agree** that the two Godbolt kernels must not use the checksum as `main`'s
return value — that is what made GitHub CI red (`rc=40`/`rc=33` = low byte
of the digests). They now print the digest and `return 0` after a known-answer
check, matching `gzip_crc32.c`.

**Agree** `.p2align N, fill` must be honoured (GAS
`.byte 1; .p2align 3, 0xff; .byte 2` → `01ffffffffffffff02` in `.data` and
`.text`). Fill is now parsed and applied; default NOP padding is unchanged.

**Agree** the 97-probe battery is not an in-tree artifact; do not cite it as
reproducible. The reproducible command is
`python3 scripts/aarch64_operand_legality_matrix.py --check`.

## 6. Standing rules for the next agent

- Base: this tree, rebased on latest `ms178/lccc` main. If #762 has
  merged, rebase onto that merge.
- Fastbuild, `-j2`, swap on, no full/slow CI.
- Snapshot `ms178-1.patch` after every validated fix.
- NAK regressions: every byte change of a valid instruction needs a GAS
  2.47 / llvm-mc 23.1.2 pin.
- No new dependencies, no `#[allow]`, no weakened assertions.
- `ci_local.sh --fast` before claiming done on a machine that can run it.
