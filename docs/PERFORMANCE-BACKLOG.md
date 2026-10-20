# Generated-code performance backlog

Keep unimplemented opportunities here only when a reproducible workload or
assembly witness exists. Move completed work to its change review with measured
results; do not turn unmeasured expectations into claims.

## Resolved items

- **Population-count loop idiom (`loop_idiom`):** `while (x) { x &= x - 1; n++; }`
  is recognized by `try_match_popcount_loop` / `rewrite_popcount_loop` in
  `src/passes/loop_idiom.rs` when `Driver::resolved_fast_popcount()` confirms the
  active target emits a hardware popcount instruction without a libcall, and
  `integer_bounds` collapses `U64 -> I32 -> I64` popcount round-trip casts in
  64-bit accumulator loops (`k17_popcnt64.c`: inner loop folded to `popcntq %rax, %rax`;
  `tests/regression/popcount_loop_idiom.c`).
- **CRC32 table index narrowing (`bit_idioms`, `integer_bounds`, x86 SIB lowering):**
  narrow unsigned cast indices (`(uint8_t)(crc ^ *p++)`) and signed masked indices
  (`((int)crc ^ (int)*p++) & 0xff`) are canonicalized in `bit_idioms` and narrowed
  by range propagation in `integer_bounds` + `collect_bitop_nonneg_values` so x86-64
  SIB addressing uses the zero-extended 32-bit index directly without `movslq` or
  redundant `andl $255`, and `emit_alu_reg_direct` stages commutative `%rax` RHS
  operands directly into `dest_phys` (`crc32_nibble_hot.c` inner loop reduced from
  10 to 8 instructions, matching GCC 16.2; verified by `check_crc_index_home.sh`
  and `tests/regression/crc32_index_narrowing_variants.c`).

