min_u32:
.cfi_startproc
    .cfi_def_cfa_offset 8
    cmpl %esi, %edi
    cmovbq %rdi, %rsi
    movq %rsi, %rax
    ret
