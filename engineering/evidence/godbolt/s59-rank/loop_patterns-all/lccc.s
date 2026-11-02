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
    subq $56, %rsp
    vpxor %xmm2, %xmm2, %xmm2
    vpxor %xmm3, %xmm3, %xmm3
    leaq array(%rip), %r11
    vpxor %xmm4, %xmm4, %xmm4
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
    leaq array(%rip), %r8
    vpxor %xmm5, %xmm5, %xmm5
    vmovdqa %xmm2, %xmm15
    vmovdqa %xmm5, %xmm14
    xorl %r9d, %r9d
.LBB4:
    cmpl $40000000, %r9d
jge .LBB6
.LBB5:
    movslq %r9d, %rdi
    vmovdqu (%r8,%rdi), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm15, %xmm15
    vmovdqu 16(%r8,%rdi), %xmm0
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
    vmovdqa %xmm15, %xmm8
.LBB7:
    cmpl $40000000, %r9d
jge .LBB9
.LBB8:
    movslq %r9d, %rsi
    vmovdqu (%r8,%rsi), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm8, %xmm8
    addl $16, %r9d
    cmpl $40000000, %r9d
jl .LBB8
.LBB9:
    vmovdqa %xmm8, %xmm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovq %xmm0, %rax
    movq %rax, 40(%rsp)
    xorl %edx, %edx
.LBB10:
    cmpq $40000000, %rdx
jge .LBB12
.LBB11:
    vmovdqu (%r8,%rdx), %xmm0
    vpxor %xmm1, %xmm1, %xmm1
    vpcmpgtd %xmm1, %xmm0, %xmm1
    vpand %xmm1, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm4, %xmm4
    addq $16, %rdx
    cmpq $40000000, %rdx
jl .LBB11
.LBB12:
    vmovdqa %xmm4, %xmm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovsd %xmm0, 32(%rsp)
.LBB13:
    movl (%r8), %eax
    leaq 4(%r8), %r10
    vmovd %eax, %xmm0
    vpbroadcastd %xmm0, %ymm0
    vmovdqa %ymm0, %ymm11
    movl $1, %r9d
    movq %r10, %rdi
.LBB14:
    cmpq $1250000, %r9
jge .LBB16
.LBB15:
    vpmaxsd (%rdi), %ymm11, %ymm11
    addq $1, %r9
    leaq 32(%rdi), %rdi
    cmpq $1250000, %r9
jl .LBB15
.LBB16:
    vmovdqa %ymm11, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpmaxsd %xmm1, %xmm0, %xmm0
    vpshufd $0x4e, %xmm0, %xmm1
    vpmaxsd %xmm1, %xmm0, %xmm0
    vpshufd $0xb1, %xmm0, %xmm1
    vpmaxsd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movq %rax, 16(%rsp)
    shll $3, %r9d
    movl %r9d, %esi
    subl $8, %esi
    movq %rax, %r11
.LBB17:
    cmpl $9999999, %esi
jge .LBB20
.LBB18:
    movslq %esi, %rdx
    movl (%r10, %rdx, 4), %ebx
    cmpl %r11d, %ebx
    cmovgl %ebx, %r11d
.LBB19:
    addl $1, %esi
    movslq %esi, %rsi
    cmpl $9999999, %esi
jl .LBB18
.LBB20:
    leaq main.buf.0(%rip), %rsi
    movl $3, %eax
    vmovd %eax, %xmm0
    vpbroadcastd %xmm0, %ymm0
    vmovdqa %ymm0, %ymm2
    movl $7, %eax
    vmovd %eax, %xmm0
    vpbroadcastd %xmm0, %ymm0
    vmovdqa %ymm0, %ymm4
    xorl %edx, %edx
.LBB21:
    cmpq $40000000, %rdx
jge .LBB23
.LBB22:
    vpmulld (%r8,%rdx), %ymm2, %ymm6
    vpaddd %ymm4, %ymm6, %ymm7
    vmovdqu %ymm7, (%rsi,%rdx)
    addq $32, %rdx
    cmpq $40000000, %rdx
jl .LBB22
.LBB23:
    shrq $2, %rdx
    movslq %edx, %r9
    movq %r9, %rdi
    shlq $2, %rdi
    leaq main.buf.0(%rip), %rcx
    leaq (%rcx, %rdi), %rdx
    leaq array(%rip), %rcx
    leaq (%rcx, %rdi), %r10
.LBB24:
    cmpq $10000000, %r9
jge .LBB26
.LBB25:
    movl (%r10), %edi
    leaq (%rdi, %rdi, 2), %rdi
    leal 7(%rdi), %eax
    movl %eax, (%rdx)
    addq $1, %r9
    leaq 4(%rdx), %rdx
    leaq 4(%r10), %r10
    cmpq $10000000, %r9
jl .LBB25
.LBB26:
    vpxor %xmm8, %xmm8, %xmm8
    vmovdqa %xmm3, %xmm15
    vmovdqa %xmm8, %xmm14
    xorl %edx, %edx
.LBB27:
    cmpl $40000000, %edx
jge .LBB29
.LBB28:
    movslq %edx, %r10
    vmovdqu (%rsi,%r10), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm15, %xmm15
    vmovdqu 16(%rsi,%r10), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm14, %xmm14
    addl $32, %edx
    cmpl $40000000, %edx
jl .LBB28
.LBB29:
    vpaddq %xmm14, %xmm15, %xmm15
    vmovdqa %xmm15, %xmm11
.LBB30:
    cmpl $40000000, %edx
jge .LBB32
.LBB31:
    movslq %edx, %r9
    vmovdqu (%rsi,%r9), %xmm0
    vpmovsxdq %xmm0, %xmm1
    vpunpckhqdq %xmm0, %xmm0, %xmm0
    vpmovsxdq %xmm0, %xmm0
    vpaddq %xmm0, %xmm1, %xmm1
    vpaddq %xmm1, %xmm11, %xmm11
    addl $16, %edx
    cmpl $40000000, %edx
jl .LBB31
.LBB32:
    vmovdqa %xmm11, %xmm0
    vpshufd $0xEE, %xmm0, %xmm1
    vpaddq %xmm1, %xmm0, %xmm0
    vmovq %xmm0, %rax
    movq %rax, 24(%rsp)
    xorl %r10d, %r10d
    xorl %edi, %edi
.LBB33:
    cmpq $1000000, %rdi
jge .LBB35
.LBB34:
    movslq (%r8, %rdi, 4), %rax
    movslq %eax, %r9
    movslq (%rsi, %rdi, 4), %rax
    movslq %eax, %rdx
    imulq %rdx, %r9
    addq %r9, %r10
    addq $1, %rdi
    cmpq $1000000, %rdi
jl .LBB34
.LBB35:
    leaq array(%rip), %r9
    movl $42, %edi
    xorl %esi, %esi
.LBB36:
    cmpq $10000, %rsi
jge .LBB38
.LBB37:
    imull $1664525, %edi, %edx
    leal 1013904223(%rdx), %edi
    movl %edi, %r13d
    imulq $1374389535, %r13, %r13
    shrq $37, %r13
    movl %r13d, %r14d
    imull $100, %r14d, %r14d
    movl %edi, %eax
    subl %r14d, %eax
    movq %rsi, %rbp
    shlq $2, %rbp
    movl %eax, (%r9, %rsi, 4)
    addq $1, %rsi
    cmpq $10000, %rsi
jl .LBB37
.LBB38:
    movl (%r8), %r9d
    leaq 4(%r8), %rdi
    movl $1, %esi
.LBB39:
    cmpq $10000, %rsi
jge .LBB41
.LBB40:
    addl (%rdi), %r9d
    movl %r9d, (%rdi)
    addq $1, %rsi
    leaq 4(%rdi), %rdi
    cmpq $10000, %rsi
jl .LBB40
.LBB41:
    movslq array+39996(%rip), %r12
    subq $8, %rsp
    movq %r12, %rax
    pushq %rax
    leaq .Lstr0(%rip), %rdi
    movq 56(%rsp), %rsi
    movq 48(%rsp), %rdx
    movq %r11, %rcx
    movq 40(%rsp), %r8
    movq %r10, %r9
    xorl %eax, %eax
    call printf@PLT
    addq $16, %rsp
    xorl %eax, %eax
    vzeroupper
    addq $56, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


