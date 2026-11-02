sha256_transform:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $376, %rsp
    .cfi_def_cfa_offset 432
    # LCCC_RET_XMM 0
    movq %rdi, 360(%rsp)
    leaq 104(%rsp), %rdi
    subq %rsi, %rdi
    subq $4, %rdi
    cmpq $25, %rdi
jb .LBB2
.LBB1:
    xorl %r8d, %r8d
    jmp .LBB3
.LBB2:
    xorl %r9d, %r9d
    jmp .LBB13
.LBB3:
    cmpq $64, %r8
jge .LBB12
.p2align 5,,15
.p2align 4
.LBB4:
    vmovdqu (%rsi,%r8), %ymm2
    vmovdqu %ymm2, 104(%rsp,%r8)
    addq $32, %r8
    cmpq $64, %r8
jl .LBB4
    jmp .LBB12
.LBB5:
    leaq 168(%rsp), %r11
    movl $16, %r10d
.LBB6:
    cmpq $64, %r10
jge .LBB8
.p2align 4,,10
.p2align 3
.LBB7:
    movl 96(%rsp, %r10, 4), %edx
    movl %edx, %r8d
    roll $15, %r8d
    movl %edx, %r9d
    roll $13, %r9d
    xorl %r9d, %r8d
    shrl $10, %edx
    xorl %edx, %r8d
    addl 76(%rsp, %r10, 4), %r8d
    movl 44(%rsp, %r10, 4), %esi
    movl %esi, %edx
    rorl $7, %edx
    movl %esi, %r9d
    roll $14, %r9d
    xorl %r9d, %edx
    shrl $3, %esi
    xorl %esi, %edx
    addl %edx, %r8d
    movl 40(%rsp, %r10, 4), %edx
    addl %r8d, %edx
    movl %edx, (%r11)
    addq $1, %r10
    leaq 4(%r11), %r11
    cmpq $64, %r10
jl .LBB7
.LBB8:
    movq 360(%rsp), %rcx
    movl (%rcx), %ebx
    movl 4(%rcx), %r11d
    movl 8(%rcx), %edi
    movl 12(%rcx), %edx
    movl 16(%rcx), %r12d
    movl 20(%rcx), %r9d
    movl 24(%rcx), %r13d
    movl 28(%rcx), %r14d
    leaq K(%rip), %r15
    movl %r11d, %eax
    andl %edi, %eax
    movq %rax, 80(%rsp)
    movq %r11, 96(%rsp)
    movq %r9, 88(%rsp)
    xorl %r8d, %r8d
    jmp .LBB9
.p2align 4,,10
.p2align 3
.LBB9:
    movq %rbx, %r10
    movq %r14, 72(%rsp)
    movq %r12, 48(%rsp)
    movq 96(%rsp), %rax
    movq %rax, 24(%rsp)
    movq 88(%rsp), %rax
    movq %rax, 56(%rsp)
    movq %rdi, 32(%rsp)
    movq %r13, 64(%rsp)
    movq %rdx, 40(%rsp)
    cmpq $64, %r8
jge .LBB11
.LBB10:
    movl %r12d, %esi
    rorl $6, %esi
    movl %r12d, %ebp
    rorl $11, %ebp
    xorl %ebp, %esi
    movl 48(%rsp), %ebp
    roll $7, %ebp
    xorl %ebp, %esi
    movl 72(%rsp), %ebp
    addl %esi, %ebp
    movl 48(%rsp), %esi
    andl 56(%rsp), %esi
    movl 48(%rsp), %eax
    notl %eax
    andl 64(%rsp), %eax
    movl %eax, 20(%rsp)
    xorl %eax, %esi
    addl %esi, %ebp
    movl (%r15, %r8, 4), %eax
    movl %eax, 20(%rsp)
    addl %eax, %ebp
    movl 104(%rsp, %r8, 4), %esi
    leal (%rbp, %rsi), %r11d
    movl %r10d, %esi
    rorl $2, %esi
    movq %r10, %rbp
    rorl $13, %ebp
    xorl %ebp, %esi
    movq %r10, %rbp
    roll $10, %ebp
    xorl %ebp, %esi
    movl %r10d, %r9d
    andl 24(%rsp), %r9d
    movq %r10, %rbp
    andl 32(%rsp), %ebp
    movq %rbp, %rax
    movq %r9, %rbp
    xorl %eax, %ebp
    xorl 80(%rsp), %ebp
    addl %ebp, %esi
    movl 40(%rsp), %r12d
    addl %r11d, %r12d
    leal (%r11, %rsi), %ebx
    movq %r8, %rax
    addq $1, %rax
    movq %rax, 8(%rsp)
    movq 64(%rsp), %r14
    movq %r10, 96(%rsp)
    movq 48(%rsp), %rax
    movq %rax, 88(%rsp)
    movq 24(%rsp), %rdi
    movq 56(%rsp), %r13
    movq 32(%rsp), %rdx
    movq 8(%rsp), %r8
    movq %r9, 80(%rsp)
    jmp .LBB9
.LBB11:
    movq 360(%rsp), %rcx
    movl %r10d, %eax
    addl (%rcx), %eax
    movq 360(%rsp), %rcx
    movl %eax, (%rcx)
    movq 360(%rsp), %rax
    leaq 4(%rax), %rax
    movq %rax, %r8
    movl (%rax), %eax
    addl 24(%rsp), %eax
    movl %eax, (%r8)
    movq 360(%rsp), %rax
    leaq 8(%rax), %rax
    movq %rax, %r11
    movl (%rax), %eax
    addl 32(%rsp), %eax
    movl %eax, (%r11)
    movq 360(%rsp), %rax
    leaq 12(%rax), %rax
    movq %rax, %rdi
    movl (%rax), %eax
    addl 40(%rsp), %eax
    movl %eax, (%rdi)
    movq 360(%rsp), %rax
    leaq 16(%rax), %rax
    movq %rax, %rdx
    movl (%rax), %eax
    addl 48(%rsp), %eax
    movl %eax, (%rdx)
    movq 360(%rsp), %rax
    leaq 20(%rax), %rax
    movq %rax, %r9
    movl (%rax), %eax
    addl 56(%rsp), %eax
    movl %eax, (%r9)
    movq 360(%rsp), %rax
    leaq 24(%rax), %rax
    movq %rax, %r10
    movl (%rax), %eax
    addl 64(%rsp), %eax
    movl %eax, (%r10)
    movq 360(%rsp), %rsi
    leaq 28(%rsi), %rsi
    movl (%rsi), %edx
    movl %edx, %eax
    addl 72(%rsp), %eax
    movl %eax, (%rsi)
    vzeroupper
    addq $376, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB12:
    shrq $2, %r8
    movslq %r8d, %r9
.LBB13:
    cmpl $16, %r9d
jge .LBB5
.p2align 4,,10
.p2align 3
.LBB14:
    movslq %r9d, %r8
    movl (%rsi, %r8, 4), %eax
    movl %eax, 104(%rsp, %r8, 4)
    addl $1, %r9d
    cmpl $16, %r9d
jl .LBB14
    jmp .LBB5
