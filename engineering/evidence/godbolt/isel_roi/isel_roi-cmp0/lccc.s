cmp0:
.cfi_startproc
    .cfi_def_cfa_offset 8
    testl %edi, %edi
    setne %sil
    movzbl %sil, %esi
    movslq %esi, %rdx
    movl %edx, %eax
    ret
