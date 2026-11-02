# Subsystem: x86 APX / EVEX encoder + ISel

**Status: landed, OFF by default.** Consolidated 2026-09-16 from
`docs/FOLLOWUP_ASSEMBLER_APX.md` (deleted), `docs/FOLLOWUP_ASSEMBLER_EVEX.md` (deleted),
`docs/FOLLOWUP_CODEGEN_APX.md` (deleted). Contract file for the
`-mapx` / `-mapxf` surface and the EVEX `.casefile` corpus.

---

## 1. Gate contract (hard invariants)

- `_Insn_feature` `-mapx` / `-mapxf` explicit opt-in only; the codegen
  ISel EGPR guard is thread-local (`isel::set_apx_enabled`).
- **`-march=raptorlake` (or any `-march`) must NEVER emit APX/EGPR** —
  #UD on the 14700KF-class hosts. "APX is gated codegen, not a march
  alias." Regression gates on this stay.
- Prefer **REX2 over EVEX**, **VEX over EVEX** when both are legal
  (Intel optimisation manual: shorter prefix, no L0/L1 stalls); keep the
  `movl` zero-extension form over `movabs` where width permits
  (`movq $0xffffffff, %r16` = 7-byte `movl`, `d5 10 b8` + imm32).
- Do **not** emit PUSH2 in prologues: 6 B EVEX vs 3 B for two plain
  pushes (size wins are kept, bloat is not).

## 2. Encoder facts (reverse-engineered against GAS 2.47)

`/usr/bin/as` (2.44) is **not an oracle**. Pinned:
`~/.cache/gas-2.47-x86_64/bin/as` via `scripts/ensure_gas_247.sh`;
re-generate corpora with `scripts/gen_apx_asmdiff.py` into
`tests/asm-diff/{apx,evex}.casefile` (+ `tests/asm-diff/i686/basic.casefile`).

- 2-op EGPR → **REX2** (`0xD5`; map-1 `D5 pp 0F opc` → `D5 (pp|0x80) opc`).
  `gp_id` is 5-bit; `is_accum` ≡ `gp_id == 0`.
- 3-op NDD / `{nf}` / `{evex}` of legacy map 0/1 → **APX EVEX map-4**
  (B4 is *not* inverted). BMI/0F38 with EGPR/`{nf}`/`{evex}` → EVEX
  **mmm=2**, not map-4; no-EGPR BMI stays VEX.
- Relocation pairing (measured against GAS):

| insn | prefix | reloc |
|---|---|---|
| `{evex}`/`{nf}`/NDD `addq foo@GOTPCREL(%rip)` | `62 f4 fc {08,0c,10} 03 05` | **49** `CODE_6_GOTPCRELX` |
| `{rex2}` / `%r16` `addq foo@GOTPCREL(%rip)` | `d5 {08,48} 03 05` | **43** `CODE_4_GOTPCRELX` |
| `vmovdqa64` / `{evex} crc32q` / `{evex} andnq` GOTPCREL | EVEX (not ALU-relaxable) | **9** `GOTPCREL` |
| `movq foo@GOTTPOFF(%rip), %r16` | `d5 48 8b 05` | **44** `CODE_4_GOTTPOFF` |
| `{evex} addq foo@GOTTPOFF(%rip)` | APX EVEX ALU | **50** `CODE_6_GOTTPOFF` |
| `leaq foo@TLSDESC(%rip), %rax` / `{rex2}` variant | REX.W / `d5 08` | **34/45** `GOTPC32_TLSDESC` / `CODE_4_*` |

  `CODE_6` is **only** for relaxable APX EVEX ALU (`add/or/adc/sbb/and/
  sub/xor/cmp/test/mov/imul`), not every `0x62`-prefixed form. Group-1
  prefixes (`lock`/`rep`) splice **before** `0xD5`/`0x62`.
- **ZU is not `{evex} setcc`** — ZU is `setzuCC`/`imulzu` (ND=1, no extra
  destination register). False-accept guards pinned:
  `{nf} test/rcl/rcr/bswap/mov` and `{evex} mov/bswap/xchg` reject;
  `{evex} test` is CTEST SCC=0xA only.
- EVEX: `operand_needs_evex` sees `{sae}`-class tokens and `{1toN}`;
  compressed disp8×N (Full VL, pmovzx/sx Half/Quarter/Eighth, broadcast);
  ER only on `vadd/vmul/vsqrt/vcvt/FMA` (ER mnemonics reject bare
  `{sae}`, SAE-only reject `{er}`); `vcmpps/pd` cmp-mask `0xC2` arm;
  AVX-512 EGPR addressing uses `evex_gp_num` = 0–31 (SIB/rbp arms found).
- `R_X86_64_CODE_5_*` (VEX3, types 46–48): no GAS 2.47 oracle case yet —
  do not speculate; add the oracle case first.

## 3. Deliberately rejected (do not re-adopt)

- map-4 in `emit_evex_mod3`/`emit_evex_memop` (incomplete APX fold from
  an outside patch).
- FP VEX source-swap "symmetry" patch; `xchg %eax,%eax` → non-`0x90`
  rewrite (breaks oracle parity).

## 4. Where things live

Encoder: `src/backend/x86/assembler/encoder/{apx,avx,core,gp_integer,mod,
registers,sse,system,x87_misc}.rs`. Relocs: `src/backend/x86/linker/{elf,
emit_exec,emit_script,emit_shared,plt_got}.rs`, `src/backend/elf/
symbol_table.rs`. ISel guard: `src/backend/x86/codegen/emit.rs`. CLI:
`src/driver/cli.rs`. Unit: `encoder::apx_tests` (15), `encoding_opt_tests`
(7). Corpora: `tests/asm-diff/apx.casefile`, `evex.casefile`.
