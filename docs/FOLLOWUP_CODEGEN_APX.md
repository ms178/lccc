# Follow-up: APX codegen (`-mapx` / `-mapxf`)

Assembler coverage (REX2 / EGPR / NDD / NF / CCMP / …) lives in
`encoder::apx_tests` and `docs/FOLLOWUP_ASSEMBLER_APX.md`. This note is the
**ISel / RA contract**: those encodings must not reach a non-APX host.

## Gate

* **Off by default.** `-march=raptorlake` / `x86-64-v3` / the i7-14700KF
  sandbox do **not** imply APX. Emitting `%r16`–`%r31` or NDD on those
  parts is `#UD`.
* **On** only with explicit `-mapx` or `-mapxf` (aliases). `-mno-apx` /
  `-mno-apxf` clear it. `-march=diamondrapids` still errors (AVX10 is
  unimplemented); it does **not** silently turn APX on.
* The flag is `CodegenOptions.apx` → `X86Codegen.apx_enabled` and the
  **thread-local** `isel::set_apx_enabled` used by MachInst emission /
  the window scratch pool (two TUs compiled on different threads cannot
  leak `-mapx` into a Raptor Lake object).

## What fires when the gate is on

1. **NDD 3-operand integer ALU** for ops LEA cannot replace: Sub/And/Or/Xor
   and register-register Imul. `mov %src1, %dst; sub %src, %dst` becomes
   `{nf} subq %src, %src1, %dst` (MachInst fusion) when dest ≠ src1.
   Dest-equals-lhs **and dest-equals-rhs** keep the two-address form
   (3 bytes vs EVEX NDD 6). Immediate IMUL stays with `mul_const_plan` /
   legacy `imul $k, src, dst` — NDD must not steal those.
2. **`{nf}` on already-EVEX NDD** when no later MachInst in the window
   reads EFLAGS (`Cmov` / `SetCC` / `Jcc`). Flag writers (`Alu`, `Cmp`,
   `Test`, `Shift`, `Neg`, `Not`, `Div`, …) kill the flags. `{nf}` is a
   P2 bit on the same EVEX — free; it is **not** applied to 2-address ALU
   (that would inflate 3-byte legacy to 6-byte EVEX).
3. **Extra GPRs `%r16`–`%r31`.** PhysReg 40..=55, caller-saved, appended
   to the main RA pool **and** the MachInst window scratch pool only when
   the gate is on. Name tables (64/32/16/8) in `emit.rs` and
   `machinst_emit.rs`. XMM identity stays 20..=33 (`is_xmm_reg`); GPR
   tests use `is_gpr_reg` so an EGPR is never mistaken for an XMM.

## What fires even without `-mapx` (Raptor Lake win)

* MachInst `mov %src1, %dst; add %src, %dst` with dest ≠ src1 folds to
  `lea (src1, src), dst` (1 µop, 4 bytes, no flags). NDD Add would be
  6-byte EVEX and is never selected for this shape. 32- and 64-bit.

## What stays off

* Default ISel / `-march=raptorlake` / no `-mapx`: no EGPR, no NDD, no
  `{nf}`. Existing BMI2 `shlx` is unchanged.
* `{zu}` / SETZU / IMULZU: assembler-only until a consumer needs them.
* **PUSH2/POP2 in prologues: declined.** GAS 2.47 `push2 %rbx, %r12` is
  6-byte EVEX (`62 f4 1c 18 ff f3`) vs `push %rbx; push %r12` = 3 bytes.
  EGPR pushes are caller-saved and never in `used_callee_saved`.

## Proof

* Unit: `apx_egpr_names_are_gprs_not_xmm` (incl. `r16w`/`r16b`),
  `isel_homes_never_include_egpr_by_default`,
  `mov_plus_add_folds_to_lea_even_without_apx`,
  `apx_ndd_fuses_mov_plus_sub_only_when_enabled` (`{nf}` + cmov live flags).
* Assembler: `encoder::apx_tests` + `scripts/asmdiff.py tests/asm-diff/apx.casefile`
  vs GAS 2.47 (`LCCC_GAS`).
* Compile a TU **without** `-mapx` / with `-march=raptorlake` and grep: no
  `%r16`–`%r31`, no 3-operand integer ALU.
* With `-mapx`, extra GPRs and NDD Sub/And/Or/Xor/Imul are legal; Add
  dest≠lhs stays LEA.

## Remaining (do not chase without a measured win)

1. Text-path `{nf}` (no next-insn window; flags may be live).
2. Main-RA EGPR *preference* vs callee-saved (window pool already prefers
   caller-saved EGPRs).
3. NDD memory operands / NDD ADC/SBB (no MachInst form).
4. Full `ci_local.sh`. Do not add asmdiff to CI without `LCCC_GAS`.
