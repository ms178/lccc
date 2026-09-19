.Lstr0:

input_a:
input_b:
sink:

sum_f32:
    xorps %xmm2, %xmm2
    xorl %esi, %esi
.LBB1:
    cmpq $65536, %rsi
jge .LBB3
.LBB2:
    vaddss (%rdi, %rsi, 4), %xmm2, %xmm2
    addq $1, %rsi
    cmpq $65536, %rsi
jl .LBB2
.LBB3:
    movss %xmm2, %xmm0
    ret

dot_f32:
    xorps %xmm2, %xmm2
    xorl %edx, %edx
.LBB5:
    cmpq $65536, %rdx
jge .LBB7
.LBB6:
    movss (%rdi, %rdx, 4), %xmm3
    vfmadd231ss (%rsi, %rdx, 4), %xmm3, %xmm2
    addq $1, %rdx
    cmpq $65536, %rdx
jl .LBB6
.LBB7:
    movss %xmm2, %xmm0
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    subq $16, %rsp
    movq %rdi, %r14
    movq %rsi, %r15
    cmpl $1, %edi
jg .LBB10
.LBB9:
    movl $5000, %ebx
    jmp .LBB11
.LBB10:
    movq 8(%r15), %rdi
    xorl %esi, %esi
    movl $10, %edx
    call strtol@PLT
    movq %rax, %r8
    movslq %eax, %rbx
.LBB11:
    leaq input_a(%rip), %r9
    leaq input_b(%rip), %rdi
    movss .LCFP_0(%rip), %xmm2
    movss .LCFP_1(%rip), %xmm3
    xorl %r12d, %r12d
.LBB12:
    cmpq $65536, %r12
jge .LBB14
.LBB13:
    movq %r12, %rax
    andq $15, %rax
    vcvtsi2ssq %rax, %xmm0, %xmm4
    vmulss %xmm2, %xmm4, %xmm4
    movss %xmm4, (%r9, %r12, 4)
    movq %r12, %rax
    andq $7, %rax
    vcvtsi2ssq %rax, %xmm0, %xmm5
    vmulss %xmm3, %xmm5, %xmm5
    movss %xmm5, (%rdi, %r12, 4)
    addq $1, %r12
    cmpq $65536, %r12
jl .LBB13
.LBB14:
    xorl %r13d, %r13d
.LBB15:
    cmpl %ebx, %r13d
jge .LBB17
.LBB16:
    leaq input_a(%rip), %rdi
    movl $65536, %esi
    call sum_f32
    movapd %xmm0, %xmm6
    movss %xmm6, sink(%rip)
    leaq input_a(%rip), %rdi
    leaq input_b(%rip), %rsi
    movl $65536, %edx
    call dot_f32
    movapd %xmm0, %xmm7
    movss sink(%rip), %xmm8
    vaddss %xmm7, %xmm8, %xmm8
    movss %xmm8, sink(%rip)
    addl $1, %r13d
    cmpl %ebx, %r13d
jl .LBB16
.LBB17:
    movss sink(%rip), %xmm9
    vmovss %xmm9, %xmm9, %xmm0
    cvtss2sd %xmm0, %xmm0
    movapd %xmm0, %xmm10
    leaq .Lstr0(%rip), %rdi
    movq %xmm10, %rax
    movq %rax, %xmm0
    movb $1, %al
    call printf@PLT
    movss sink(%rip), %xmm11
    ucomiss .LCFP_2(%rip), %xmm11
    setnp %al
    sete %cl
    andb %cl, %al
    movzbl %al, %eax
    movq %rax, %r8
    testb %al, %al
    movl $0, %ecx
    movl $1, %r9d
    cmovnel %ecx, %r9d
    movl %r9d, %eax
    addq $16, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

.LCFP_0:
.LCFP_1:
.LCFP_2:

