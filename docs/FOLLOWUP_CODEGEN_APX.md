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
  process-global `isel::set_apx_enabled` used by MachInst emission.

## What fires when the gate is on

1. **NDD 3-operand integer ALU.** `mov %src1, %dst; add %src, %dst`
   becomes `addq %src, %src1, %dst` (MachInst fusion in
   `machinst_emit::emit_machinsts`, and the text path
   `emit_alu_reg_direct` via `try_emit_ndd_alu`). Add/Sub/And/Or/Xor.
   Dest-equals-lhs keeps the two-address form (shorter, no EVEX).
   64-bit Add with dest ≠ lhs still prefers LEA (1 µop, no flags) —
   NDD is for the ops LEA cannot replace.
2. **Extra GPRs `%r16`–`%r31`.** PhysReg 40..=55, caller-saved, appended
   to the RA pool only when the gate is on. Name tables in `emit.rs` and
   `machinst_emit.rs`. XMM identity stays 20..=33 (`is_xmm_reg`); GPR
   tests use `is_gpr_reg` so an EGPR is never mistaken for an XMM.

## What stays off

* Default ISel / `-march=raptorlake` / no `-mapx`: no EGPR, no NDD, no
  `{nf}`. Existing BMI2 `shlx` is unchanged.
* `{zu}` / SETZU / IMULZU: assembler-only until a consumer needs them.
* PUSH2/POP2 in prologues: not yet; the 2×push is still cheaper on
  non-APX and the APX form needs a matched epilogue.
* Window-allocator scratch does **not** draw from the EGPR file (the
  static `MACHINST_ALLOCATABLE_GPRS` pool is legacy GPRs). Main RA is
  the extra-register win.

## Proof

* Unit: `apx_egpr_names_are_gprs_not_xmm`,
  `apx_ndd_fuses_mov_plus_add_only_when_enabled`.
* Assembler: `encoder::apx_tests` + `scripts/asmdiff.py tests/asm-diff/apx.casefile`
  → 13/13 vs GAS 2.47 (`LCCC_GAS`). Clippy `-D warnings` clean on `--lib`.
* Compile a TU **without** `-mapx` / with `-march=raptorlake` and grep: no
  `%r16`–`%r31`, no 3-operand integer ALU. Confirmed on `add3`/`many`.
* With `-mapx`, `many()` drops three callee-saved push/pops, uses `%r16`–
  `%r18`, and emits NDD `imulq %src, %src1, %dst` / `addq %src, %src1, %dst`.
  GAS 2.47 accepts the object.
