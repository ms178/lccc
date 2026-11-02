.Lstr0:

array:
main.buf.0:

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    vpxor %xmm2, %xmm2, %xmm2
    vpxor %xmm3, %xmm3, %xmm3
    leaq array(%rip), %r11
    movl $42, %r10d
    xorl %r8d, %r8d
.LBB1:
    cmpq $10000000, %r8
jge .LBB3
.LBB2:
    imull $1664525, %r10d, %r9d
    leal 1013904223(%r9), %r10d
    movl %r10d, %eax
    shrl $1, %eax
    leal -1000000000(%rax), %edi
    movl %edi, (%r11, %r8, 4)
    addq $1, %r8
    cmpq $10000000, %r8
jl .LBB2
.LBB3:
    leaq array(%rip), %rbx
    vpxor %xmm4, %xmm4, %xmm4
    vmovdqa %xmm2, %xmm15
    vmovdqa %xmm4, %xmm14
    xorl %r9d, %r9d
.LBB4:
    cmpl $40000000, %r9d
jge .LBB6
.LBB5:
    movslq %r9d, %rdi
    vmovdqu (%rbx,%rdi), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm15, %xmm15
    vmovdqu 16(%rbx,%rdi), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm14, %xmm14
    addl $32, %r9d
    cmpl $40000000, %r9d
jl .LBB5
.LBB6:
    vpaddq %xmm14, %xmm15, %xmm15
    vmovdqa %xmm15, %xmm7
.LBB7:
    cmpl $40000000, %r9d
jge .LBB9
.LBB8:
    movslq %r9d, %rsi
    vmovdqu (%rbx,%rsi), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm7, %xmm7
    addl $16, %r9d
    cmpl $40000000, %r9d
jl .LBB8
.LBB9:
    vmovdqa %xmm7, %xmm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovq %xmm0, %rax
    movq %rax, 24(%rsp)
    xorl %r11d, %r11d
    xorl %edx, %edx
.LBB10:
    cmpq $10000000, %rdx
jge .LBB12
.LBB11:
    movslq (%rbx, %rdx, 4), %rdi
    movslq %edi, %r10
    leaq (%r11, %r10, 1), %r9
    testl %edi, %edi
    cmovgq %r9, %r11
    addq $1, %rdx
    cmpq $10000000, %rdx
jl .LBB11
.LBB12:
    movl (%rbx), %r10d
    leaq 4(%rbx), %rdi
    movl $1, %esi
.LBB13:
    cmpq $10000000, %rsi
jge .LBB15
.LBB14:
    movl (%rdi), %edx
    cmpl %r10d, %edx
    cmovgl %edx, %r10d
    addq $1, %rsi
    leaq 4(%rdi), %rdi
    cmpq $10000000, %rsi
jl .LBB14
.LBB15:
    leaq main.buf.0(%rip), %r9
    movl $3, %eax
    vmovd %eax, %xmm0
    vpbroadcastd %xmm0, %ymm0
    vmovdqa %ymm0, %ymm9
    movl $7, %eax
    vmovd %eax, %xmm0
    vpbroadcastd %xmm0, %ymm0
    vmovdqa %ymm0, %ymm10
    xorl %edi, %edi
.LBB16:
    cmpq $40000000, %rdi
jge .LBB18
.LBB17:
    vpmulld (%rbx,%rdi), %ymm9, %ymm12
    vpaddd %ymm10, %ymm12, %ymm13
    vmovdqu %ymm13, (%r9,%rdi)
    addq $32, %rdi
    cmpq $40000000, %rdi
jl .LBB17
.LBB18:
    shrq $2, %rdi
    movslq %edi, %rdx
    movq %rdx, %rdi
    shlq $2, %rdi
    leaq main.buf.0(%rip), %rcx
    leaq (%rcx, %rdi), %rsi
    leaq array(%rip), %rcx
    leaq (%rcx, %rdi), %r8
.LBB19:
    cmpq $10000000, %rdx
jge .LBB21
.LBB20:
    movl (%r8), %edi
    leaq (%rdi, %rdi, 2), %rdi
    leal 7(%rdi), %eax
    movl %eax, (%rsi)
    addq $1, %rdx
    leaq 4(%rsi), %rsi
    leaq 4(%r8), %r8
    cmpq $10000000, %rdx
jl .LBB20
.LBB21:
    vpxor %xmm2, %xmm2, %xmm2
    vmovdqa %xmm3, %xmm15
    vmovdqa %xmm2, %xmm14
    xorl %esi, %esi
.LBB22:
    cmpl $40000000, %esi
jge .LBB24
.LBB23:
    movslq %esi, %rdx
    vmovdqu (%r9,%rdx), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm15, %xmm15
    vmovdqu 16(%r9,%rdx), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm14, %xmm14
    addl $32, %esi
    cmpl $40000000, %esi
jl .LBB23
.LBB24:
    vpaddq %xmm14, %xmm15, %xmm15
    vmovdqa %xmm15, %xmm5
.LBB25:
    cmpl $40000000, %esi
jge .LBB27
.LBB26:
    movslq %esi, %r8
    vmovdqu (%r9,%r8), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm5, %xmm5
    addl $16, %esi
    cmpl $40000000, %esi
jl .LBB26
.LBB27:
    vmovdqa %xmm5, %xmm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovq %xmm0, %rax
    movq %rax, 16(%rsp)
    xorl %r12d, %r12d
    xorl %edi, %edi
.LBB28:
    cmpq $1000000, %rdi
jge .LBB30
.LBB29:
    movslq (%rbx, %rdi, 4), %rax
    movslq %eax, %r8
    movslq (%r9, %rdi, 4), %rax
    movslq %eax, %rsi
    imulq %rsi, %r8
    addq %r8, %r12
    addq $1, %rdi
    cmpq $1000000, %rdi
jl .LBB29
.LBB30:
    leaq array(%rip), %rdx
    movl $42, %r8d
    xorl %r9d, %r9d
.LBB31:
    cmpq $10000, %r9
jge .LBB33
.LBB32:
    imull $1664525, %r8d, %edi
    leal 1013904223(%rdi), %r8d
    movl %r8d, %esi
    imulq $1374389535, %rsi, %rsi
    shrq $37, %rsi
    movl %esi, %r13d
    imull $100, %r13d, %esi
    movl %r8d, %eax
    subl %esi, %eax
    movl %eax, (%rdx, %r9, 4)
    addq $1, %r9
    cmpq $10000, %r9
jl .LBB32
.LBB33:
    movl (%rbx), %edx
    leaq 4(%rbx), %r8
    movl $1, %r9d
.LBB34:
    cmpq $10000, %r9
jge .LBB36
.LBB35:
    addl (%r8), %edx
    movl %edx, (%r8)
    addq $1, %r9
    leaq 4(%r8), %r8
    cmpq $10000, %r9
jl .LBB35
.LBB36:
    movslq array+39996(%rip), %rbp
    subq $8, %rsp
    movq %rbp, %rax
    pushq %rax
    leaq .Lstr0(%rip), %rdi
    movq 40(%rsp), %rsi
    movq %r11, %rdx
    movq %r10, %rcx
    movq 32(%rsp), %r8
    movq %r12, %r9
    xorl %eax, %eax
    call printf@PLT
    addq $16, %rsp
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


