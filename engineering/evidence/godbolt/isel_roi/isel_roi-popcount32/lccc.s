popcount32:
.cfi_startproc
    .cfi_def_cfa_offset 8
    popcntl %edi, %eax
    movl %eax, %eax
    ret
