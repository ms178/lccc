clz32:
.cfi_startproc
    .cfi_def_cfa_offset 8
    lzcntl %edi, %edi
    movl %edi, %eax
    ret
