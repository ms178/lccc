zinit:
.cfi_startproc
    subq $4104, %rsp
    .cfi_def_cfa_offset 4112
    leaq 8(%rsp), %rdi
    xorl %eax, %eax
    movl $4096, %ecx
    rep stosb
    movb $0, 8(%rsp)
    leaq 8(%rsp), %rdi
    call use@PLT
    addq $4104, %rsp
    ret
