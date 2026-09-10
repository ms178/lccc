# Follow-up: x86 assembler APX (REX2 / EGPR / NDD / NF)

Encodings reverse-engineered against GNU as 2.47 and unit-tested in
`encoder::apx_tests`. Size wins vs GAS are kept (REX2 over EVEX, VEX over
EVEX, `movl` zero-extend over `movabs`).

## What landed

### S01 — REX2 / NDD / `{nf}` ALU
* `%r16`–`%r31`; `gp_id` is 5-bit; `is_accum` is `gp_id == 0`.
* 2-op EGPR → REX2 (`0xD5`). Map-1 `D5 pp 0F opc` → `D5 (pp|0x80) opc`.
* 3-op NDD / `{nf}` / `{evex}` → APX EVEX map-4. B4 is **not** inverted.
* `movq $0xffffffff, %r16` = 7-byte `movl` (`d5 10 b8`+imm32).

### S02 — map-4 0F38 promotions
* `crc32` / `adcx` / `adox` / `movbe` with EGPR or `{evex}` → EVEX map-4
  (opcodes F0/F1, 66, 60/61). `{nf}` rejected. No REX2 on map 2/3.

### S03 — BMI2, PUSH2, CCMP, remaining NF
* **BMI/BMI2**: no-EGPR stays VEX. EGPR / `{nf}` / `{evex}` → EVEX **mmm=2**
  (`rorx` **mmm=3**). NF legal for `andn`/`bextr`/`bzhi`/`bls*` only.
* **PUSH2/POP2/PUSHP/POPP**, **jmpabs**, **CCMP/CTEST** (DFV raw in P1.vvvv,
  SCC in P2[3:0]). `{evex} setcc` is ND=0 (not ZU). `{nf}` remaps for
  lzcnt/tzcnt/popcnt/shld-imm/shrd-imm/imul/mul/div.

### S04 — leftovers, IMULZU, CFCMOV, NDD SHLD, REX2 GOTPCRELX
* **Leftover `self.rex`** converted to `emit_rex_or_rex2` /
  `emit_rex_unary` / `emit_rex_rr` / `emit_rex_rm`: `rdpid`, fsgsbase,
  sldt/str/smsw/lmsw, fxsaveq/fxrstorq, movq xmm↔gp, movq mmx↔gp,
  mov seg, mov cr/dr. EGPR on those paths now encodes as REX2 (map-1
  collapsed by `fixup_rex2_map1`).
* **XSAVE family** still rejects EGPR base/index (ISA: #UD; GAS agrees).
  FXSAVEQ/FXRSTORQ **do** accept EGPR via REX2.
* **`smsw %rax`** now emits REX.W (`48 0f 01 e0`), matching GAS; SLDT
  still omits W (upper bits zero without it).
* **`R_X86_64_CODE_4_GOTPCRELX` (43)** for RIP-relative GOT loads whose
  instruction starts with REX2 `0xD5`. `gotpcrel_x_type()` scans past
  legacy prefixes: `0x40-0x4F` → 42, `0xD5` → 43, else 41. Linker
  relaxation treats 43 like the other X variants (`8b`→`8d`).
* **IMULZU**: 16-bit only (`imulzu` / `imulzuw`), EVEX map-4 ND=1 vvvv=0,
  opcode 6B/69. `{nf}` sets P2.NF. 32/64-bit forms rejected (ZU is a no-op
  there; GAS also refuses them).
* **SETZU**: `setzuCC[b]` is the true ZU form (ND=1, pp=3, opcode 40+cc).
  `{evex} setcc` stays ND=0. Shorter EGPR `setzb %r16b` remains REX2.
  Memory dest rejected (`setzub (%rax)`): ZU zeros unused GPR bits.
* **CFCMOV**: 2-op load/reg ND=0 NF=0; 2-op store ND=0 NF=1; 3-op NDD
  ND=1 NF=1. Distinct from 3-op `cmov` (NF=0) and from `{evex}` 2-op
  `cmov` (still rejected).
* **4-op NDD SHLD/SHRD**: `shldq $imm/%cl, %src, %src1, %ndd`. Imm
  remaps to map-4 24/2C; CL keeps A5/AD. `{nf}` ORs P2.NF. 16/32-bit
  via pp. 3-op `shldw` now emits the missing `0x66`.
* **xadd/cmpxchg** register-register (including EGPR) — previously only
  the memory dest form existed.
* **CCMP** 16-bit / byte / X4-index coverage (`ccmpnew`, `ccmpneb`,
  `(%rax,%r16,4)`).

### S05 — PR #462 encoder wins, without its CI regressions
Adopted the *valid* encoder optimizations from
[PR #462](https://github.com/ms178/lccc/pull/462) (closed) and skipped the
parts that broke CI:

* **VEX2 commutative source-swap** on `vpmuludq` / `vpsadbw` / `vpmaddwd` /
  `vpmulhuw` (integer only; FP add/mul still declined — NaN payload).
* **SSSE3 VEX arms**: `vpmulhrsw`, `vphsubw/d/sw`, `vphaddsw`, `vpmuldq`,
  `vmpsadbw`, `vphminposuw` (128-bit only).
* **EVEX xmm/ymm16–31**: `vec_reg_id` + `operand_needs_evex` force the EVEX
  path; `emit_evex_mod3` / `emit_evex_memop` thread R′/V′/X so high ids no
  longer wrap to xmm0. `vpermq`/`vpermpd` dest is ModRM.reg (vvvv unused).
* **`extractps` / unsuffixed `cvtsi2ss/sd`**: one REX via `emit_rex_*`;
  `%r8d` is 32-bit.
* **i686 scale-1 SIB fold**: `mov 0(,%eax,1),%ecx` → `8b 08` (ICC win).
* **Suffix-less** `blsi`/`blsr`/`blsmsk`; **AMD** `clzero`/`rdpru`/`mcommit`.
* **Fuzz OOM**: `m_absurd_alignment` is 2^20, not 2^40 (1 TiB freeze).
* Did **not** strip script `100755` bits (the PR #462 CI killer).

### S06 — oracle harness matches S05 encodings
S05 `ci_local.sh --fast` was 14/14; snapshot `16e9ffc`. Whole-object
`asmdiff.py` vs GNU as 2.47 was 788/789: the only miss was
`apx_mov_imm betterok` (`movq $0xffffffff, %r16` as 7-byte `movl`).
The bytes were right; the harness did not treat an EGPR 32-bit write as
zero-extending, so `semantically_equal` rejected a verified size win.

* `_GP32_TO_64` now includes `%r16d`–`%r31d` in `asmdiff.py`,
  `insndiff.py`, and `encdiff.py`.
* `_COMMUTATIVE_VEX` includes `vpmuludq` / `vpsadbw` / `vpmaddwd` so a
  VEX2 source-swap is scored BETTER, not unverified SHORTER. FP add/mul
  still not swapped.
* `insndiff --sweep` expands repeated `{XMM}` / `{R64}` independently
  (dest × src1 × src2), matching the documented
  `imul{S} ${IMM}, %{R{S}}, %{R{S}}` example.
* `insndiff` exit 0 on BETTER (same as asmdiff `betterok`).

Checked vs GAS 2.47: asmdiff **789/789**. AVX sweep 864 (vpmuludq /
vpsadbw / vpmaddwd / vpmulhuw × XMM³): 648 ok + 216 BETTER. ALU/mov/lea
sweeps 2692: 2592 ok + 100 both-reject (illegal `ah`/`spl` mix etc.).

## Not yet (next session)

1. Full `ci_local.sh` (cargo-test --all-targets, clippy, regression).
   Do not add asmdiff to CI without `LCCC_GAS`.
2. **Codegen**: do **not** emit APX from ISel for `-march=raptorlake`
   (i7-14700KF has no APX). Assembler support is for hand-written /
   future `-mapx` assembly.
3. Optional: EVEX GOTPCREL (`R_X86_64_CODE_6_*`) if anyone writes
   `{evex} movq foo@GOTPCREL(%rip), %reg`. REX2 covers the EGPR case.
4. Optional: remaining exotic APX (PUSH2 mem). `{zu}` as a prefix is
   junk (GAS); ZU is `setzu*` / `imulzu`.

## Oracle notes

* 2-op EGPR → REX2 (2-byte prefix), never EVEX (4-byte). Size is the
  quality metric.
* `{evex}` / `{nf}` / NDD of *legacy map 0/1* → EVEX map-4.
* BMI 0F38 with EGPR/`{nf}`/`{evex}` → EVEX **mmm=2**, not map-4. No-EGPR
  VEX is shorter and kept.
* ZU is **not** `{evex} setcc`. ZU is `setzuCC` / `imulzu` (ND=1, no extra dest).
* CFCMOV store uses NF as the “reverse operand” bit, not a no-flags hint.
* Group-1 prefixes (`lock`/`rep`) splice *before* `0xD5`/`0x62`.

## Files

* `src/backend/x86/assembler/encoder/{apx,avx,core,gp_integer,mod,registers,sse,system,x87_misc}.rs`
* `src/backend/x86/linker/{elf,emit_exec,emit_script,emit_shared,plt_got}.rs`
* `src/backend/elf/symbol_table.rs`
* `tests/asm-diff/apx.casefile`, `scripts/gen_apx_asmdiff.py`


See also `docs/FOLLOWUP_ASSEMBLER_EVEX.md` (S07 dest-in-ModRM.reg / tuple /
SAE; **S08** GAS 2.47 on `0d842bc`: AVX-512 EGPR B4/X4, SAE vs ER legality,
NDD ALU-imm memory, FALSE-ACCEPT guards).

