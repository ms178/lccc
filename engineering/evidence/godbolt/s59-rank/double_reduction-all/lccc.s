.Lstr0:

a:
b:
c:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    leaq a(%rip), %r11
    leaq b(%rip), %r10
    leaq c(%rip), %r8
    vpxor %ymm2, %ymm2, %ymm2
    vpxor %ymm3, %ymm3, %ymm3
    vpxor %ymm4, %ymm4, %ymm4
    vpxor %ymm5, %ymm5, %ymm5
    movl $1, %r9d
    xorl %edi, %edi
.LBB1:
    cmpq $1048576, %rdi
jge .LBB3
.LBB2:
    imull $1664525, %r9d, %esi
    addl $1013904223, %esi
    movl %esi, %edx
    shrl $16, %edx
    movq %rdx, %rax
    andq $15, %rax
    leal -7(%rax), %edx
    movl %edx, (%r11, %rdi, 4)
    imull $1664525, %esi, %esi
    addl $1013904223, %esi
    movl %esi, %edx
    shrl $16, %edx
    movq %rdx, %rax
    andq $15, %rax
    leal -7(%rax), %edx
    movl %edx, (%r10, %rdi, 4)
    imull $1664525, %esi, %esi
    leal 1013904223(%rsi), %r9d
    movl %r9d, %edx
    shrl $16, %edx
    movq %rdx, %rax
    andq $15, %rax
    leal -7(%rax), %eax
    movl %eax, (%r8, %rdi, 4)
    addq $1, %rdi
    cmpq $1048576, %rdi
jl .LBB2
.LBB3:
    leaq a(%rip), %rdx
    leaq b(%rip), %rbx
    leaq a(%rip), %r9
    leaq b(%rip), %rdi
    leaq c(%rip), %rsi
    xorl %r11d, %r11d
    xorl %r12d, %r12d
.LBB4:
    cmpl $128, %r12d
jge .LBB12
.LBB5:
    vmovdqa %ymm3, %ymm6
    vmovdqa %ymm2, %ymm7
    xorl %r8d, %r8d
.LBB6:
    cmpl $4194304, %r8d
jge .LBB8
.LBB7:
    movslq %r8d, %r10
    vmovdqu (%r9,%r10), %ymm8
    vpmulld (%rdi,%r10), %ymm8, %ymm10
    vpaddd %ymm10, %ymm6, %ymm6
    vpmulld (%rsi,%r10), %ymm8, %ymm13
    vpaddd %ymm13, %ymm7, %ymm7
    addl $32, %r8d
    cmpl $4194304, %r8d
jl .LBB7
.LBB8:
    vmovdqa %ymm6, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movq %rax, 24(%rsp)
    vmovdqa %ymm7, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movq %rax, 16(%rsp)
    movl 24(%rsp), %eax
    addl 16(%rsp), %eax
    movslq %eax, %r10
    leaq (%r11, %r10, 1), %r13
    vmovdqa %ymm4, %ymm15
    xorl %r10d, %r10d
    vmovdqa %ymm5, %ymm6
.LBB9:
    cmpl $4194304, %r10d
jge .LBB11
.LBB10:
    movslq %r10d, %r8
    vpaddd (%rdx,%r8), %ymm6, %ymm6
    vpaddd (%rbx,%r8), %ymm15, %ymm15
    addl $32, %r10d
    cmpl $4194304, %r10d
jl .LBB10
.LBB11:
    vmovdqa %ymm15, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movq %rax, 24(%rsp)
    vmovdqa %ymm6, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movq %rax, 16(%rsp)
    addl 24(%rsp), %eax
    movslq %eax, %r8
    leaq (%r13, %r8, 1), %r11
    addl $1, %r12d
    cmpl $128, %r12d
jl .LBB5
.LBB12:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    vzeroupper
    addq $40, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


