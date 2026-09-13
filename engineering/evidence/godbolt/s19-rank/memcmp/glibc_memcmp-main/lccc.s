main:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $104, %rsp
    .cfi_def_cfa_offset 160
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movq $0, 64(%rsp)
    movabsq $72623859790382856, %rax
    movq %rax, 72(%rsp)
    movq $-1, 80(%rsp)
    movq $7, 88(%rsp)
    movq 64(%rsp), %rax
    movq %rax, 32(%rsp)
    movq 72(%rsp), %rax
    movq %rax, 40(%rsp)
    movq 80(%rsp), %rax
    movq %rax, 48(%rsp)
    movq 88(%rsp), %rax
    movq %rax, 56(%rsp)
    leaq 64(%rsp), %rdi
    leaq 32(%rsp), %rsi
    movl $4, %edx
    call glibc_memcmp_common_alignment
    testl %eax, %eax
jne .LBB25
.LBB24:
    movabsq $72623859790382857, %rax
    movq %rax, 40(%rsp)
    leaq 64(%rsp), %rdi
    leaq 32(%rsp), %rsi
    movl $4, %edx
    call glibc_memcmp_common_alignment
    testl %eax, %eax
    movl $0, %ecx
    movl $1, %r8d
    cmovgel %ecx, %r8d
    testl %r8d, %r8d
jne .LBB26
.LBB25:
    movl $2, %eax
.L__lccc_epilogue_3:
    addq $104, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB26:
    leaq glibc_left(%rip), %rbx
    leaq glibc_right(%rip), %r12
    xorl %r13d, %r13d
    movabsq $-7046029254386353131, %r14
.LBB27:
    cmpq $8192, %r13
jb .LBB29
.LBB28:
    xorl %r15d, %r15d
    xorl %ebp, %ebp
    jmp .LBB30
.p2align 4,,10
.p2align 3
.LBB29:
    movq %r14, %r9
    shlq $7, %r9
    movq %r14, %rdi
    xorq %r9, %rdi
    movq %rdi, %rsi
    shrq $9, %rsi
    xorq %rsi, %rdi
    movq %rdi, %rdx
    shlq $8, %rdx
    movq %rdi, %r14
    xorq %rdx, %r14
    movq %r14, (%rbx, %r13, 8)
    movq %r14, (%r12, %r13, 8)
    addq $1, %r13
    cmpq $8192, %r13
jb .LBB29
    jmp .LBB28
.LBB30:
    cmpl $4096, %r15d
jae .LBB32
.p2align 4,,10
.p2align 3
.LBB31:
    movl %r15d, %r13d
    imulq $4051, %r13, %r9
    andq $8191, %r9
    imull $11, %r15d, %edi
    andl $63, %edi
    movl $1, %esi
    shlxq %rdi, %rsi, %rsi
    movq %r9, %r14
    movq (%rbx, %r14, 8), %rax
    movq %rax, 24(%rsp)
    movq %rsi, %rax
    xorq 24(%rsp), %rax
    movq %rax, (%r12, %r14, 8)
    leaq glibc_left(%rip), %rdi
    leaq glibc_right(%rip), %rsi
    movl $8192, %edx
    call glibc_memcmp_common_alignment
    addl $257, %eax
    movslq %eax, %r10
    leaq 1(%r13), %r8
    imulq %r8, %r10
    addq %r10, %rbp
    movq 24(%rsp), %rax
    movq %rax, (%r12, %r14, 8)
    addl $1, %r15d
    cmpl $4096, %r15d
jb .LBB31
.LBB32:
    leaq .Lstr0(%rip), %rdi
    movq %rbp, %rsi
    xorl %eax, %eax
    call printf@PLT
    # LCCC_VA_CALL
    xorl %eax, %eax
    jmp .L__lccc_epilogue_3
