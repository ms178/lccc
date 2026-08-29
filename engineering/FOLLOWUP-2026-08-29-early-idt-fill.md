# Follow-up — 2026-08-29: early IDT stub `.fill` over-pad

**Symptom.** 64-bit kernel reached long mode then halted in
`early_fixup_exception` → `pv_ops` `native_halt`. pt_regs.ip was
`ffffffff83903aa7`, the `endbr64` of the #GP stub. IDT[n] is
`early_idt_handler_array + n * EARLY_IDT_HANDLER_SIZE` (13 with IBT).

**Not this bug.** CPUID/FPU userspace is fine. Do not “fix”
`.Lcommon_startup_64` (the quad at `ffffffff83a449c8` is
`common_startup_64`). Do not pass `earlyprintk=` on this bzImage
(16-bit `early_serial_init` livelocks). Do not re-land `-u`.

**Root cause.** `arch/x86/kernel/head_64.S`:

```
.fill early_idt_handler_array + i*EARLY_IDT_HANDLER_SIZE - ., 1, 0xcc
```

after each stub (`i = i + 1` inside `.rept`). The assembler lowered
that to a deferred `SkipExpr`. Jump relaxation first *speculatively
shortens* every `jmp early_idt_handler_common` (even ones that cannot
fit in rel8), then sizes the skip against the short layout, then grows
the jmps back to rel32. Extra `0xcc` bytes stay behind.

Hex dump of vmlinux:

- vec0–7: 13-byte body, no extra fill
- vec8 (errcode): 11-byte body + 2×cc = 13
- **vec9 (no err): 13-byte body + 2×cc = 15** ← first overflow
- later slots accumulate more `cc`

From vector 9 the IDT slots no longer land on stub entry points; #GP
hits padding / mid-instruction.

**Fix.** `LABEL + N - .` (named label, not numeric `0b`) is a
location-counter *target*. Lower it to `.org LABEL+N` with the fill
byte, and restretch org padding after jump relaxation. Org padding
must keep the fill byte (0xcc), not multi-byte NOPs.

**Regression.** `early_idt_handler_fill_slots_are_exact` and
`parse_org_style_skip_recognizes_named_label_dot_diff` in
`elf_writer_common.rs`.

**Also in this patch.** i386 `-T` archive load now seeds `-u` the same
way as x86-64 (`script_undefined_pulls_archive_i386`).
