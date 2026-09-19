.Lstr0:

input:
output:

stencil5:
    movl $2, %edx
    movl $8, %r8d
.LBB1:
    leal 2(%rdx), %r9d
    cmpl $8195, %r9d
jge .LBB4
.LBB2:
    vmovups -8(%rsi,%r8), %ymm3
    vaddps -4(%rsi,%r8), %ymm3, %ymm4
    vaddps (%rsi,%r8), %ymm4, %ymm6
    vaddps 4(%rsi,%r8), %ymm6, %ymm8
    vaddps 8(%rsi,%r8), %ymm8, %ymm10
    vmovups %ymm10, (%rdi,%r8)
    addl $1, %edx
    addq $32, %r8
    leal 2(%rdx), %r11d
    cmpl $8195, %r11d
jl .LBB2
    jmp .LBB4
.LBB3:
    vzeroupper
    ret
.LBB4:
    subl $2, %edx
    shll $3, %edx
    addl $2, %edx
    movslq %edx, %r10
.LBB5:
    leaq 2(%r10), %rdx
    cmpq $65536, %rdx
jge .LBB3
.LBB6:
    vmovss -8(%rsi, %r10, 4), %xmm11
    vmovss -4(%rsi, %r10, 4), %xmm12
    vmovss (%rsi, %r10, 4), %xmm13
    vmovss 4(%rsi, %r10, 4), %xmm14
    vmovss 8(%rsi, %r10, 4), %xmm15
    vaddss %xmm12, %xmm11, %xmm11
    vaddss %xmm13, %xmm11, %xmm11
    vaddss %xmm14, %xmm11, %xmm11
    vaddss %xmm15, %xmm11, %xmm11
    vmovss %xmm11, (%rdi, %r10, 4)
    addq $1, %r10
    leaq 2(%r10), %r11
    cmpq $65536, %r11
jl .LBB6
    jmp .LBB3

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    movq %rdi, %r15
    movq %rsi, %rbp
    cmpl $1, %edi
jg .LBB9
.LBB8:
    movl $1000, %ebx
    jmp .LBB10
.LBB9:
    movq 8(%rbp), %rdi
    xorl %esi, %esi
    movl $10, %edx
    call strtol@PLT
    movq %rax, %r8
    movslq %eax, %rbx
.LBB10:
    leaq input(%rip), %r9
    movss .LCFP_0(%rip), %xmm2
    xorl %r12d, %r12d
.LBB11:
    cmpq $65536, %r12
jge .LBB13
.LBB12:
    movq %r12, %rax
    andq $15, %rax
    vcvtsi2ssq %rax, %xmm0, %xmm3
    vmulss %xmm2, %xmm3, %xmm3
    movss %xmm3, (%r9, %r12, 4)
    addq $1, %r12
    cmpq $65536, %r12
jl .LBB12
.LBB13:
    xorl %r13d, %r13d
.LBB14:
    cmpl %ebx, %r13d
jge .LBB16
.LBB15:
    leaq output(%rip), %rdi
    leaq input(%rip), %rsi
    movl $65536, %edx
    call stencil5
    addl $1, %r13d
    cmpl %ebx, %r13d
jl .LBB15
.LBB16:
    leaq output(%rip), %rdx
    xorl %eax, %eax
    movq %rax, 8(%rsp)
    xorl %r14d, %r14d
.LBB17:
    cmpq $65536, %r14
jge .LBB19
.LBB18:
    movss (%rdx, %r14, 4), %xmm4
    vmovss %xmm4, %xmm4, %xmm0
    cvtss2sd %xmm0, %xmm0
    movapd %xmm0, %xmm5
    vaddsd 8(%rsp), %xmm5, %xmm6
    addq $1, %r14
    movsd %xmm6, 8(%rsp)
    cmpq $65536, %r14
jl .LBB18
.LBB19:
    leaq .Lstr0(%rip), %rdi
    movq 8(%rsp), %rax
    movq %rax, %xmm0
    movb $1, %al
    call printf@PLT
    movsd .LCFP_1(%rip), %xmm0
    ucomisd 8(%rsp), %xmm0
    movl $0, %ecx
    movl $1, %edi
    cmoval %ecx, %edi
    movsd 8(%rsp), %xmm0
    ucomisd .LCFP_2(%rip), %xmm0
    movl $1, %esi
    cmoval %edi, %esi
    movl %esi, %eax
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

.LCFP_0:
.LCFP_1:
.LCFP_2:

