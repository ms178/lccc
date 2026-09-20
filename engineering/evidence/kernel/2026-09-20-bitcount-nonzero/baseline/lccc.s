branch_index_store:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    leaq branch_slots(%rip), %rsi
    xorl %edx, %edx
.LBB38:
    testq %rdi, %rdi
je .LBB43
.p2align 4,,10
.p2align 3
.LBB39:
    bsfq %rdi, %rax
    movq %rax, %r8
    cmpl %edx, %eax
    movq %rdx, %r9
.LBB40:
    je .LBB42
.LBB41:
    movl (%rsi, %r8, 4), %r9d
.LBB42:
    movslq %edx, %rdx
    movl %r9d, (%rsi, %rdx, 4)
    addl $1, %edx
    leaq -1(%rdi), %r10
    andq %r10, %rdi
jne .LBB39
.LBB43:
    ret
