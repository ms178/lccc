.Lstr0:

main.bounds.0:

a:
b:

sum_f64:
    movslq %esi, %rdx
    xorpd %xmm2, %xmm2
    xorl %r8d, %r8d
.LBB1:
    cmpq %rdx, %r8
jge .LBB3
.LBB2:
    vaddsd (%rdi, %r8, 8), %xmm2, %xmm2
    addq $1, %r8
    cmpq %rdx, %r8
jl .LBB2
.LBB3:
    movsd %xmm2, %xmm0
    ret

dot_f64:
    movslq %edx, %r8
    xorpd %xmm2, %xmm2
    xorl %r9d, %r9d
.LBB5:
    cmpq %r8, %r9
jge .LBB7
.LBB6:
    movsd (%rdi, %r9, 8), %xmm3
    vfmadd231sd (%rsi, %r9, 8), %xmm3, %xmm2
    addq $1, %r9
    cmpq %r8, %r9
jl .LBB6
.LBB7:
    movsd %xmm2, %xmm0
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $40, %rsp
    movq %rdi, %rbx
    movq %rsi, %r12
    cmpl $1, %edi
jg .LBB10
.LBB9:
    movl $1, %ebx
    jmp .LBB11
.LBB10:
    movq 8(%r12), %rdi
    xorl %esi, %esi
    movl $10, %edx
    call strtol@PLT
    movq %rax, %r8
    movslq %eax, %rbx
.LBB11:
    leaq a(%rip), %r9
    leaq b(%rip), %rdi
    xorl %r12d, %r12d
.LBB12:
    cmpq $65, %r12
jge .LBB14
.LBB13:
    leaq 1(%r12), %rsi
    vcvtsi2sdq %rsi, %xmm0, %xmm2
    movsd %xmm2, (%r9, %r12, 8)
    leaq 2(%r12), %rax
    vcvtsi2sdq %rax, %xmm0, %xmm3
    movsd %xmm3, (%rdi, %r12, 8)
    movq %rsi, %r12
    cmpq $65, %rsi
jl .LBB13
.LBB14:
    leaq main.bounds.0(%rip), %r13
    xorl %r14d, %r14d
    xorpd %xmm4, %xmm4
.LBB15:
    cmpl %ebx, %r14d
jge .LBB20
.LBB16:
    xorl %r15d, %r15d
    movsd %xmm4, 24(%rsp)
.LBB17:
    cmpq $18, %r15
jae .LBB19
.LBB18:
    movl (%r13, %r15, 4), %ebp
    leaq a(%rip), %rdi
    movq %rbp, %rsi
    call sum_f64
    movapd %xmm0, %xmm5
    movsd 24(%rsp), %xmm0
    vaddsd %xmm5, %xmm0, %xmm0
    movsd %xmm0, 16(%rsp)
    leaq a(%rip), %rdi
    leaq b(%rip), %rsi
    movq %rbp, %rdx
    call dot_f64
    movapd %xmm0, %xmm6
    vaddsd 16(%rsp), %xmm6, %xmm7
    addq $1, %r15
    movsd %xmm7, 24(%rsp)
    cmpq $18, %r15
jb .LBB18
.LBB19:
    addl $1, %r14d
    movsd 24(%rsp), %xmm4
    cmpl %ebx, %r14d
jl .LBB16
.LBB20:
    leaq .Lstr0(%rip), %rdi
    movsd %xmm4, %xmm0
    movb $1, %al
    call printf@PLT
    xorl %eax, %eax
    addq $40, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret


