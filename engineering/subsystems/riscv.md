# RISC-V backend and assembler: immediate shifts

The RV64 encoder's shift/rotate immediate entry points must validate the
original signed value **before** narrowing to `u32`. RV64 SLLI/SRLI/SRAI/RORI
accept 0..63; SLLIW/SRLIW/SRAIW/RORIW accept 0..31. Immediate pseudo spellings
such as `sll rd, rs, imm` share that check. Register-count instructions remain
register-count instructions: their architectural masking is not permission to
truncate an assembler immediate silently.

The normative RV64I shift-field review used `riscv/riscv-isa-manual` commit
`7a90b34e1a04e57a65bfd9de9a3b4e8dc2c5ddc8`, `src/unpriv/rv64.adoc`, sections
`rv64i-slli` and `rv64i-slliw`. The word-shift text calls encodings with imm[5]
nonzero *reserved*, not universally illegal-instruction traps. The diagnostic
policy rejects out-of-range source operands rather than emitting another,
truncated instruction. Zbb rotate encodings share the same field widths.

Unit regressions cover every legal immediate and exact encoding bits, negative
and narrowing-overflow values, immediate aliases, and register-count controls.
This is a scoped encoder contract, not certification of every RV64 extension,
the vectorizer, calling convention, linker, compressed encodings or runtime.
