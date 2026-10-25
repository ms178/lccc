# 2026-10-25 — the last reference gap in the as-2.47 cross-reference

Base: `4fb7bc6e` (= `main` at the start of this pass, merge of #961).

`scripts/gas_encoding_crossref.py` checks every declarative encoder row of both
backends against a vendored distillation of GNU as 2.47's own instruction table
(`opcodes/i386-opc.tbl` @ `binutils-2_47-branch`).  It reported nine mnemonics
as **UNCOVERED** — `andnl/andnq`, `blsil/blsiq`, `blsmskl/blsmskq`,
`blsrl/blsrq`, `setcc` — and the gate's own comment recorded that as an accepted
"known reference gap: the distiller cannot expand those forms".  It was not a
template problem at all.

## Root cause

`i386-opc.tbl` packs `[mandatory prefix] opcode` into **one** field, and a
three-byte VEX/EVEX row leaves the opcode *alone* in it:

```
andn,   0xf2,    APX_F(BMI), Modrm|...|Vex128|EVex128|Space0F38|...
blsi,   0xf3/3,  APX_F(BMI), Modrm|...|Vex128|EVex128|Space0F38|...
shrx,   0xf2f7,  APX_F(BMI2),Modrm|...|Vex128|EVex128|Space0F38|...
```

`andn 0xf2` is `VEX.NDS.LZ.0F38.W0 F2` — prefix *none*, opcode `F2` — while
`shrx 0xf2f7` is `VEX.F2.0F38.W0 F7` — prefix `F2`, opcode `F7`.  The distiller
stripped any leading byte in `{66,f2,f3}` unconditionally, so the lone byte
became a prefix, no opcode byte was left, and the `len(arr) != 1` guard dropped
the row.  The encodings were always correct; nothing ever checked them.

## Fix

* The opcode is the **last** byte of the field; a leading byte in the prefix set
  is stripped only when another byte follows it.
* The APX `EVexMap4..7` promoted-legacy rows (`wrssd, 0x66, ...|EVexMap4`) keep
  their previous verdict: there the field really is only a prefix, the opcode
  lives in the legacy row of the same name, and maps 4..7 are modelled exactly
  by the SDM/XED gate.
* `SNAPSHOT_ALIASES = {"setcc": "setc"}`: the encoder's generic spelling
  resolves to the row it emits (`0F 92`), so the comparison stays exact instead
  of being skipped.

Effect: +26 mnemonics (2431 → 2457) and +52 encodings (5999 → 6051) in the
snapshot, because the same shape also hid the EVEX-only `vfpclass*` and
`vpblendm*` rows.  `--verify --strict --backend all`: **1637 rows checked, 0
findings, 0 unmodelled, 0 uncovered** (was 1628 / 0 / 0 / 9).

## Verification

| Check | Result |
|---|---|
| Rebuild of `scripts/isa/gas-2.47.json` from the upstream table | byte-identical |
| Upstream table | `opcodes/i386-opc.tbl` @ `binutils-2_47-branch`, 288187 B, sha256 `a48171c05930cbe1…ba81179` = `gas-2.47.manifest.json` `upstream.sha256` |
| New rows vs GNU as 2.47 (`as`/`objdump` 2.47.20260726, built from the tag-cache `binutils-2.47.tar.xz`, sha256 `154ab23b…11e4cff`) | `andn C4 E2 E8 F2 C1`; `blsi C4 E2 F8 F3 D9` (/3), `blsr …C9` (/1), `blsmsk …D1` (/2); `setc 0F 92`; `vfpclassps 62 F3 7D 08 66 C9`; `vfpclasspd` W1; `vfpclassph` pp *none* (`62 F3 7C 08`); `vfpclassbf16` pp *f2* (`62 F3 7F 08`); `vpblendmb 62 F2 6D 08 66 D9`; `vpblendmw` W1 |
| `LCCC_REQUIRE_PINNED_ORACLE=1 … unittest scripts.test_asmdiff scripts.test_encdiff` | 101 tests OK |
| `unittest discover -s tests/corpus` | 868 tests OK, 13 skipped (no PyYAML on this box; the yaml contract modules skip at import instead of erroring — CI installs `python3-yaml`, where they run) |

## Remaining reference gap (next, not this pass)

Three-byte-opcode rows are still dropped because their field mixes the escape,
the opcode and a ModR/M byte: `wrmsrns 0x0f01c6`, `sgdt 0x0f010/0`,
`lgdt 0x0f012/2`.  Distil them as *map* `0f`, *opcode* `01` and *reg* = the
ModR/M `/digit`, and extend the model by the same amount, which also retires
the `wrmsrns` entry in `SNAPSHOT_LIMITATIONS`.  Their SDM/XED verification is already
in place, so the exposure today is a missing cross-check, not a wrong encoding.

## Runtime side (unchanged, and why)

This sandbox has no Rust toolchain — not in the image, and not in the
`harness-cache-1` tag cache, which carries binutils 2.47, GCC 16.2, the Debian
deb bundle and mold 2.42.1 and *no* Rust — so `lccc` cannot be built, and no
runtime claim is possible here.  The corpus worklist from the checked-in
evidence (`scripts/bench_rank.py`) is unchanged: `lz4_match_extend` 1.937×
(byte-compare/`memcmp` shape, PF-LZ4-1), `nbody` 1.414×, `expat_xml_scan`
1.246×, `hash_table` 1.176×, `linux_find_bit_scaled` 1.162×.
