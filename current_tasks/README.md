# Active bug queue

Each file is one verified, reproducible defect. When a task is fixed and its
regression test lands, the file is DELETED in the same commit (history lives
in git). No status journals here — that is what commit messages are for.

Verified still-open as of 2026-08-28:

| Task | Area | Repro state |
|---|---|---|
| fix_dash | RISC-V codegen | dash test FAIL riscv only |

Completed and removed 2026-08-28 (see git log / updates/followup_2026-08-28_session09.md):

CAS pair (`caspal`) encoder + register-propagation skip, ARM global/weak
branch relocation triage, ARM `.org` directive (GAS parity), PREL64
(`sym+offset - .` decomposition + size-based selection — audited, already
correct), MOVW `:abs_g*:` symbolic relocations (encoder + ELF writer +
**linker MOVW type-number repair**), i686 double-param high-word store
(audited, already correct — locked with a regression test), pcre2 small-slot
frame bloat (audited, already landed), RISC-V va_arg struct{long double}
alignment (caller padding + named-layout padding + space accounting; ARM
va_arg `align` honored — cross-backend transfer).
