# Follow-up: x86 assembler EVEX quality (S07–S08)

Port of Agent A's GAS-verified EVEX encoder/parser fixes onto S01–S06
(APX REX2/NDD/NF, PR#462 encoder wins, EGPR asmdiff oracle). APX stays
in `apx.rs`; AVX-512 helpers do **not** fold map-4.

S08 is rebased onto `ms178/lccc` main `0d842bc` (PR #467) and proven
against GNU as **2.47** (`~/.cache/gas-2.47-x86_64/bin/as`). Distro
`/usr/bin/as` 2.44 is **not** an oracle.

## What landed

* Parser: `Register.sae` / `Register.rounding`, `MemoryOperand.broadcast`,
  `{k}`/`{z}`/`{sae}`/`{r*-sae}`/`{1toN}` suffixes, standalone `{...}`
  tokens as `Operand::Label`.
* `operand_needs_evex` sees SAE tokens and `{1toN}` so xmm SAE routes EVEX.
* `emit_evex` P3.b (`bcst`). Dest of 3src-imm / GPR-broadcast is
  **ModRM.reg** (was hard-wired 0). `evex_id` uses **`gp_id`** (r0–r31),
  not Agent A's 0–15 `gpr_id`.
* Compressed disp8*N: Full VL, pmovzx/sx Half/Quarter/Eighth, broadcast
  element size, extract i32x4 N=16, insert i32x4 N=16, rotate/shift mem.
* SAE/rounding: `{r*-sae}` is ER (`vadd`/`vmul`/`vsqrt`/`vcvt`/`FMA`);
  bare `{sae}` is SAE-only (`vmax`/`vmin`/`vcmp`). GAS 2.47 rejects the
  wrong decorator; we match (no rewrite of `{sae}` → `{rn-sae}`).
* `try_encode_evex` arms: `vcmpps`/`vcmppd` (cmp-mask 0xC2),
  `vcvtps2dq`/`vcvtdq2ps` (unary 5B).
* `encode_evex_3op` wraps `encode_evex_binary` (dest≠0 + mem disp8*N).
* Scripts: asmdiff `--32` + ELF32 reader (keep `_GP32_TO_64` r8–r31);
  encdiff `--json` + `cclang2310`; `gen_encoding_sweep` groups
  `avx_ssse3`/`sse_rex`/`evex_quality`.
* Corpora: `tests/asm-diff/evex.casefile`, `tests/asm-diff/i686/basic.casefile`.

## S08 (GAS 2.47, main `0d842bc`)

Red-teamed Agent A's later 184k patch against ours. Kept our APX
(IMULZU/SETZU/jmpabs/GOTPCRELX 43/leftover REX2). Took their casefile
coverage (SSSE3 VEX, SSE REX, `vphminposuw` ymm reject) and fixed
defects their corpus plus Intel SDM `{er}` vs `{sae}` exposed:

* **AVX-512 EGPR addressing.** `evex_gp_num` is `gp_id` (0–31). SIB/rbp
  specials use `id & 7` so `%r20`/`%r21` get SIB/disp8. APX B4 is P0
  bit3 (not inverted); X4 is P1 bit2 inverted. GAS 2.47:
  `vaddps (%r16), %zmm1, %zmm2` → `62 f9 74 48 58 10`.
* **SAE legality.** ER mnemonics reject bare `{sae}`; SAE-only reject
  `{r*-sae}`; integer EVEX rejects both. Matches SDM Vol.2 `{er}` vs `{sae}`.
* **APX NDD ALU-imm memory:** `addq $1, (%rax), %rcx` and `{evex} addq $1, (%rax)`.
* **FALSE-ACCEPT guards:** `{nf} test`/`rcl`/`rcr`/`bswap`/`mov`;
  `{evex} mov`/`bswap`/`xchg`. `{evex} test` is CTEST SCC=0xA
  (`62 f4 84 0a 85 c8`).
* Prefer VEX over EVEX when both are legal (Intel opt manual: shorter
  encoding, no 512-bit tax on Raptor Lake).

Verified: asmdiff vs GAS 2.47 **804/804** + i686 `--32` **5/5**;
`encoding_opt_tests` 7/7; `apx_*` 15/15.

## Deliberately not taken from Agent A

* map-4 in `emit_evex_mod3` / `emit_evex_memop` (incomplete APX fold).
* FOLLOWUP claiming “EGPR still rejected” / skipping `{zu}` (`imulzu`/`setzu`).
* FP VEX source-swap; `xchg %eax,%eax` → not-`0x90`.
* asmdiff `_GP32_TO_64` truncated to r8–r15.
* Agent A 3src-imm mem path used Full-VL N for `{1toN}` / vinsert —
  we scale by broadcast element size / insert tuple.
* `gpr_id("") == Some(0)` empty-name landmine.

## Follow-ups

* Full `ci_local.sh`. Do not add asmdiff to CI without `LCCC_GAS=…2.47`.
* Codegen: do **not** emit APX/EGPR from ISel on i7-14700KF (`#UD`).
* Optional: more EVEX (vscalef, vgetexp SAE tables; EVEX GOTPCREL
  `R_X86_64_CODE_6_*`).
* Restore GAS 2.47 after a harness wipe: `scripts/ensure_gas_247.sh x86_64-linux-gnu`.
