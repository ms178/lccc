# Active bug queue

Each file is one verified, reproducible defect. When a task is fixed and its
regression test lands, the file is DELETED in the same commit (history lives
in git). No status journals here — that is what commit messages are for.

Verified still-open as of 2026-08-15:

| Task | Area | Repro state |
|---|---|---|
| fix_arm_asm_caspal_instruction | AArch64 asm | `caspal` rejected (LSE CAS pair) |
| fix_arm_asm_global_branch_relocs | AArch64 asm | bl/b to global resolves TU-internally |
| fix_arm_asm_org_directive | AArch64 asm | `.org` treated as size assertion only |
| fix_arm_asm_quad_prel64_relocation | AArch64 asm | three PREL64-related bugs |
| fix_arm_movw_symbolic_relocations | AArch64 asm | `:abs_g*:` movz/movk gaps |
| fix_dash | RISC-V codegen | dash test FAIL riscv only |
| fix_i686_double_param_high_word_store | i686 codegen | double param high word |
| fix_pcre2_stack_frame_bloat | x86 codegen | 792-deep nesting segfault |
| fix_riscv_va_arg_long_double_struct | RISC-V ABI | variadic struct{long double} |

Completed and removed 2026-08-15 (see git log):
string-literal dedup (compile-time + SHF_MERGE link-time), x86
symbol+expr-chain data values, `.ifb`/`.ifnb`, macro param prefix
substitution.
