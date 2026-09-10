# Follow-up: x86 assembler APX (REX2 / EGPR / NDD / NF)

Encodings reverse-engineered against GNU as 2.47 and unit-tested in
`encoder::apx_tests`. Size wins vs GAS are kept (REX2 over EVEX, VEX over
EVEX, `movl` zero-extend over `movabs`). Default ISel does **not** emit
APX: i7-14700KF (Raptor Lake) #UDs EGPR / REX2 / NDD EVEX.

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
* **PUSH2/POP2/PUSHP/POPP**, **jmpabs**, **CCMP/CTEST** (DFV raw in **P2.vvvv**,
  SCC in P3[3:0] / EVEX.aaa+P2 bits — GAS `ccmpeq {dfv=cf} %rcx,%rax` =
  `62 f4 8c 04 39 c8`). `{evex} setcc` is ND=0 (not ZU). `{nf}` remaps for
  lzcnt/tzcnt/popcnt/shld-imm/shrd-imm/imul/mul/div.

### S04 — leftovers, IMULZU, CFCMOV, NDD SHLD, REX2 GOTPCRELX
* Leftover `self.rex` paths emit REX2 for EGPR.
* **`R_X86_64_CODE_4_GOTPCRELX` (43)** for RIP-relative GOT loads whose
  instruction starts with REX2 `0xD5`.

### S05 — PR #462 encoder wins, without its CI regressions
VEX2 integer commutative swap, SSSE3 VEX arms, EVEX xmm16+, cvtsi/extractps
REX, i686 SIB fold. Did **not** VEX-swap FP add/mul. Did **not** strip
script `100755` bits.

### S06 — oracle harness matches S05 encodings
`_GP32_TO_64` includes `%r16d`–`%r31d`. asmdiff 789/789 at the time.

### S07 / S08 — EVEX dest / tuple / SAE / EGPR mem
Dest-in-ModRM.reg, compressed disp8*N, SAE vs ER, AVX-512 EGPR B4/X4.

### S09 — CODE_6 GOTPCRELX / CODE_4 GOTTPOFF / `-mapx` reject
GAS 2.47 (verified):

| insn | prefix | reloc |
|---|---|---|
| `{evex}`/`{nf}`/NDD `addq foo@GOTPCREL(%rip)` | `62 f4 fc {08,0c,10} 03 05` | **49** `CODE_6_GOTPCRELX` |
| `{rex2}` / `%r16` `addq foo@GOTPCREL(%rip)` | `d5 {08,48} 03 05` | **43** `CODE_4_GOTPCRELX` |
| `vmovdqa64` / `{evex} crc32q` / `{evex} andnq` GOTPCREL | EVEX, not ALU-map-4-relaxable | **9** `GOTPCREL` |
| `movq foo@GOTTPOFF(%rip), %r16` | `d5 48 8b 05` | **44** `CODE_4_GOTTPOFF` |
| `{evex} addq foo@GOTTPOFF(%rip)` | APX EVEX ALU | **50** `CODE_6_GOTTPOFF` |
| `leaq foo@TLSDESC(%rip), %rax` | REX.W | **34** `GOTPC32_TLSDESC` |
| `{rex2} leaq foo@TLSDESC(%rip)` | `d5 08 8d 05` | **45** `CODE_4_GOTPC32_TLSDESC` |

Encoder: `gotpcrel_x_type` / `gottpoff_type` / `tlsdesc_type` look at the
first non-legacy-prefix byte. CODE_6 only when EVEX P0.mmm==4 **and** the
opcode is a relaxable legacy ALU (`add`/`or`/`adc`/`sbb`/`and`/`sub`/`xor`/`cmp`/`test`/`mov`/`imul`).
AVX-512 (mmm=1/2/3) and APX map-4 BMI/crc32/adcx stay type 9.

Linker (`emit_exec` / `emit_script` / `emit_shared` / `plt_got`): CODE_4/6
GOTPCREL + GOTTPOFF. TLSDESC 45/51 in exec+script (prefix-agnostic
`8d 05` → `c7 c0`). `is_tls_reloc` (x86-64): `16..=23 | 34..=36 | 44 | 45 | 47 | 48 | 50 | 51`
(29..=31 is GOTPC64/GOTPLT64/PLTOFF64, **not** TLS).

CLI overlay from this session: CODE_6 / GOTTPOFF / TLSDESC only. **`-mapx`
codegen is PR #470** (gated, off by default) — see
`docs/FOLLOWUP_CODEGEN_APX.md`. Default ISel still never emits EGPR/NDD.

PUSH2 memory operands are GAS-illegal; encoder rejects them.

RIP-less `foo@GOTPCREL(%reg)` stays type **9** even with REX2 (GAS 2.47).

AVX-512 `encode_evex_mem` RIP-relative now shares `encode_modrm_mem`
(so `vmovdqa64 foo@GOTPCREL(%rip), %zmm0` encodes and is type 9, not CODE_6).

### S10 — rebase onto PR #470 + NDD quality
Rebased CODE_6 overlay onto `919572ce` (PR #470 gated `-mapx`). Kept 470's
enable of `-mapx`/`-mapxf`. NDD dest==rhs stays 2-address; 64-bit Add
stays LEA (ungated MachInst fold); `{nf}` on NDD when flags are dead;
I16/I8 EGPR names; thread-local APX flag; window EGPR pool when `-mapx`.
**Do not** PUSH2 in prologues (6B EVEX vs 3B two pushes).

## Not yet (next session)

1. Full `ci_local.sh` (cargo-test --all-targets, clippy, regression).
   Do not add asmdiff to CI without `LCCC_GAS`.
2. Default ISel / `-march=raptorlake` still must not emit APX (`#UD` on
   14700KF). `-mapx` is gated codegen, not a march alias.
3. `emit_shared` has no GOTPC32_TLSDESC apply/relax arm (pre-existing for
   type 34 too). Shared objects keep TLSDESC dynamic; not needed for the
   static/exec path this session covers.
4. CODE_5_* (VEX3, types 46–48): no GAS 2.47 oracle case yet; VEX3 GOTPCREL
   stays type 9.
5. `{evex} lea` / `{evex} movq GOTPCREL` are not GAS forms — do not chase.
6. Optional remaining exotic APX. `{zu}` as a prefix is junk (GAS); ZU is
   `setzu*` / `imulzu`.

## Oracle notes

* 2-op EGPR → REX2 (2-byte prefix), never EVEX (4-byte). Size is the
  quality metric.
* `{evex}` / `{nf}` / NDD of *legacy map 0/1* → EVEX map-4.
* BMI 0F38 with EGPR/`{nf}`/`{evex}` → EVEX **mmm=2**, not map-4. No-EGPR
  VEX is shorter and kept.
* CODE_6 is **relaxable APX EVEX ALU**, not every `0x62`.
* ZU is **not** `{evex} setcc`. ZU is `setzuCC` / `imulzu` (ND=1, no extra dest).
* Group-1 prefixes (`lock`/`rep`) splice *before* `0xD5`/`0x62`.

## Files

* `src/backend/x86/assembler/encoder/{apx,avx,core,gp_integer,mod,registers,sse,system,x87_misc}.rs`
* `src/backend/x86/linker/{elf,emit_exec,emit_script,emit_shared,plt_got}.rs`
* `src/backend/elf/symbol_table.rs`
* `src/backend/x86/codegen/emit.rs` (ISel EGPR guard)
* `src/driver/cli.rs`
* `tests/asm-diff/apx.casefile`, `scripts/gen_apx_asmdiff.py`

See also `docs/FOLLOWUP_ASSEMBLER_EVEX.md` (S07 dest-in-ModRM.reg / tuple /
SAE; **S08** GAS 2.47, rebased onto `6461190`: AVX-512 EGPR B4/X4, SAE vs ER
legality, NDD ALU-imm memory, FALSE-ACCEPT guards).
