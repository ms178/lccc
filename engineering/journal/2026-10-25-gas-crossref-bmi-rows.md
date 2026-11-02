# 2026-10-25 — closing the as-2.47 cross-reference gaps

Base: `4fb7bc6e` (= `main` at the start of this pass, merge of #961).

`scripts/gas_encoding_crossref.py` checks every declarative encoder row of both
backends against a vendored distillation of GNU as 2.47's own instruction table
(`opcodes/i386-opc.tbl` @ `binutils-2_47-branch`).  It reported nine mnemonics
as **UNCOVERED** — `andnl/andnq`, `blsil/blsiq`, `blsmskl/blsmskq`,
`blsrl/blsrq`, `setcc` — and the gate's own comment recorded that as an accepted
"known reference gap: the distiller cannot expand those forms".  It was not a
template problem at all, and the same misreading was quietly costing the whole
`0F 01 <ModRM>` group.

## Root cause 1 — a lone opcode byte read as a prefix

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

## Root cause 2 — three-byte-opcode rows dropped whole

The same guard discarded `0F 01 <ModRM>` rows, whose field mixes the escape, the
opcode and the ModRM byte: `wrmsrns 0x0f01c6`, `sgdt 0x0f010/0`,
`swapgs 0x0f01f8`.  `wrmsrns` was papered over with a `SNAPSHOT_LIMITATIONS`
entry ("the snapshot recorded the ModR/M byte as the opcode"); the rest were
simply absent.

## Fix

* The opcode is the **last** byte of the field; a leading byte in the prefix set
  is stripped only when another byte follows it.
* `0F 01 <ModRM>` is distilled as *map* `0f`, *opcode* `01`, *reg* = the ModRM
  `/reg` field (or the `/digit` the row spells itself).
* `SNAPSHOT_ALIASES = {"setcc": "setc"}`: the encoder's generic spelling
  resolves to the row it emits (`0F 92`), so the comparison stays exact instead
  of being skipped.
* The APX `EVexMap4..7` promoted-legacy rows (`wrssd, 0x66, ...|EVexMap4`) keep
  their previous verdict: there the field really is only a prefix, the opcode
  lives in the legacy row of the same name, and maps 4..7 are modelled exactly
  by the SDM/XED gate.

Effect: +87 mnemonics (2431 → 2518) and +114 encodings (5999 → 6113).  Beside
the four BMI rows and the EVEX-only `vfpclass*`/`vpblendm*` rows, the snapshot
now covers the system group — `sgdt`/`sidt`/`lgdt`/`lidt`, `monitor`/`mwait`,
`xgetbv`/`xsetbv`, `rdpkru`/`wrpkru`, `serialize`, `clac`/`stac`,
`swapgs`/`rdtscp`, `wrmsrns`, the SVM and TDX/SEAM/RMP instructions.
`--verify --strict --backend all`: **1638 rows checked, 0 findings, 0 unmodelled,
0 uncovered** (was 1628 / 0 / 0 / 9).  `SNAPSHOT_LIMITATIONS` shrinks to the
four `movs*` string moves, which the as-2.47 differential pins instead.

## Verification

| Check | Result |
|---|---|
| Rebuild of `scripts/isa/gas-2.47.json` from the upstream table | byte-identical |
| Upstream table | `opcodes/i386-opc.tbl` @ `binutils-2_47-branch`, 288187 B, sha256 `a48171c05930cbe1…ba81179` = `gas-2.47.manifest.json` `upstream.sha256` |
| BMI rows vs GNU as 2.47 (`as`/`objdump` 2.47.20260726, built from the tag-cache `binutils-2.47.tar.xz`, sha256 `154ab23b…11e4cff`) | `andn C4 E2 E8 F2 C1`; `blsi C4 E2 F8 F3 D9` (/3), `blsr …C9` (/1), `blsmsk …D1` (/2); `setc 0F 92` |
| EVEX-only rows, same oracle | `vfpclassps 62 F3 7D 08 66 C9`; `vfpclasspd` W1; `vfpclassph` pp *none* (`62 F3 7C 08`); `vfpclassbf16` pp *f2* (`62 F3 7F 08`); `vpblendmb 62 F2 6D 08 66 D9`; `vpblendmw` W1 |
| `0F 01` group, same oracle | `swapgs 0F 01 F8` (/7), `rdtscp F9` (/7), `monitor C8` (/1), `mwait C9` (/1), `xgetbv D0` (/2), `xsetbv D1` (/2), `serialize E8` (/5), `rdpkru EE` (/5), `wrpkru EF` (/5), `clac CA` (/1), `stac CB` (/1), `sgdt 0F 01 00` (/0), `sidt 08` (/1), `lgdt 10` (/2), `lidt 18` (/3), `wrmsrns C6` (/0) |
| `LCCC_REQUIRE_PINNED_ORACLE=1 … unittest scripts.test_asmdiff scripts.test_encdiff` | 101 tests OK with the pinned pair |
| `unittest discover -s tests/corpus` | 868 tests OK, 13 skipped without PyYAML; **904 OK, 5 skipped** with it (CI's configuration) |

## Runtime side (unchanged, and why)

This sandbox has no Rust toolchain — not in the image, and not in the
`harness-cache-1` tag cache, which carries binutils 2.47, GCC 16.2, the Debian
deb bundle and mold 2.42.1 and *no* Rust — so `lccc` cannot be built, and no
runtime claim is possible here.  The corpus worklist from the checked-in
evidence (`scripts/bench_rank.py`) is unchanged: `lz4_match_extend` 1.937×
(byte-compare/`memcmp` shape, PF-LZ4-1), `nbody` 1.414×, `expat_xml_scan`
1.246×, `hash_table` 1.176×, `linux_find_bit_scaled` 1.162×.
