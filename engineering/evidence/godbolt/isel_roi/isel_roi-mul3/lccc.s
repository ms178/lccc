mul3:
.cfi_startproc
    .cfi_def_cfa_offset 8
    leaq (%rdi, %rdi, 2), %rdi
    movl %edi, %eax
    ret
