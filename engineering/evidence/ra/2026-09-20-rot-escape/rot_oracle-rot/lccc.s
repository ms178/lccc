rot:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    .cfi_def_cfa_offset 80
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl (%rdi), %ebx
    movl 4(%rdi), %r11d
    movl 8(%rdi), %r9d
    movl 12(%rdi), %r12d
    movl 16(%rdi), %r13d
    movl 20(%rdi), %r8d
    movl 24(%rdi), %r15d
    movl 28(%rdi), %ebp
    movslq %edx, %rax
    movq %rax, 8(%rsp)
    movq %rbp, %rdx
    xorl %r10d, %r10d
.LBB1:
    cmpq %rax, %r10
jge .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    leal (%rdx, %r13), %edi
    addl %r8d, %edi
    addl %r15d, %edi
    addl (%rsi, %r10, 4), %edi
    leal (%rbx, %r11), %r14d
    addl %r9d, %r14d
    movq %r12, %rbp
    addl %edi, %ebp
    addl %r14d, %edi
    addq $1, %r10
    movq %r15, %rdx
    movq %r8, %r15
    movq %r9, %r12
    movq %r13, %r8
    movq %r11, %r9
    movq %rbp, %r13
    movq %rbx, %r11
    movq %rdi, %rbx
    cmpq 8(%rsp), %r10
jl .LBB2
.LBB3:
    xorl %r11d, %ebx
    xorl %r9d, %ebx
    xorl %r12d, %ebx
    xorl %r13d, %ebx
    xorl %r8d, %ebx
    xorl %r15d, %ebx
    xorl %edx, %ebx
    movl %ebx, %eax
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
