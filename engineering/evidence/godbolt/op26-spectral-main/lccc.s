main:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $32016, %rsp
    .cfi_def_cfa_offset 32048
    xorl %ebx, %ebx
.LBB14:
    cmpq $2000, %rbx
jl .LBB16
.LBB15:
    xorl %r12d, %r12d
    jmp .LBB17
.LBB16:
    movsd .LCFP_0(%rip), %xmm0
    movsd %xmm0, 16008(%rsp, %rbx, 8)
    leaq 1(%rbx), %rbx
    cmpq $2000, %rbx
jl .LBB16
    jmp .LBB15
.LBB17:
    cmpl $10, %r12d
jl .LBB19
.LBB18:
    xorpd %xmm2, %xmm2
    xorl %r13d, %r13d
    xorpd %xmm3, %xmm3
    jmp .LBB20
.LBB19:
    movl $2000, %edi
    leaq 16008(%rsp), %rsi
    leaq 8(%rsp), %rdx
    call mul_AtAv
    movl $2000, %edi
    leaq 8(%rsp), %rsi
    leaq 16008(%rsp), %rdx
    call mul_AtAv
    addl $1, %r12d
    cmpl $10, %r12d
jl .LBB19
    jmp .LBB18
.LBB20:
    cmpq $2000, %r13
jge .LBB22
.LBB21:
    movsd 16008(%rsp, %r13, 8), %xmm4
    movsd 8(%rsp, %r13, 8), %xmm5
    vmulsd %xmm5, %xmm4, %xmm4
    vaddsd %xmm4, %xmm3, %xmm3
    vmulsd %xmm5, %xmm5, %xmm5
    vaddsd %xmm5, %xmm2, %xmm2
    leaq 1(%r13), %r13
    cmpq $2000, %r13
jl .LBB21
.LBB22:
    leaq .Lstr0(%rip), %rsi
    vdivsd %xmm2, %xmm3, %xmm3
    vsqrtsd %xmm3, %xmm3
    movq %rsi, %rax
    movq %rsi, %rdi
    movsd %xmm3, %xmm0
    movb $1, %al
    call printf@PLT
    xorl %eax, %eax
    addq $32016, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret
