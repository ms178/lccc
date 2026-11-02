hash_mul:
.cfi_startproc
    .cfi_def_cfa_offset 8
    imull $-1640531535, %edi, %edi
    movl %edi, %eax
    ret
