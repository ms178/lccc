.Lstr0:

mul_AtAv:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $16216, %rsp
    movq %rsi, %rbx
    movq %rdx, %r12
    vmovsd .LCFP_0(%rip), %xmm2
    xorl %r11d, %r11d
.LBB1:
    cmpl $2000, %r11d
jge .LBB16
.LBB2:
    vxorpd %xmm3, %xmm3, %xmm3
    xorl %r10d, %r10d
    movq %rbx, %r8
.LBB3:
    movl $2000, %r9d
    subl %r10d, %r9d
    cmpl $3, %r9d
jg .LBB15
.LBB4:
    vmovapd %xmm3, %xmm4
    movq %r10, %rdi
    movq %r8, %rsi
.LBB5:
    cmpl $2000, %edi
jge .LBB7
.LBB6:
    leal (%r11, %rdi), %r10d
    leal 1(%r10), %r8d
    imull %r8d, %r10d
    movl %r10d, %r9d
    sarl $31, %r9d
    shrl $31, %r9d
    addl %r9d, %r10d
    sarl $1, %r10d
    addl %r11d, %r10d
    leal 1(%r10), %eax
    vcvtsi2sdl %eax, %xmm0, %xmm5
    vdivsd %xmm5, %xmm2, %xmm6
    vfmadd231sd (%rsi), %xmm6, %xmm4
    addl $1, %edi
    leaq 8(%rsi), %rsi
    cmpl $2000, %edi
jl .LBB6
.LBB7:
    movslq %r11d, %r11
    vmovsd %xmm4, 208(%rsp, %r11, 8)
    addl $1, %r11d
    cmpl $2000, %r11d
jl .LBB2
    jmp .LBB16
.LBB8:
    vxorpd %xmm10, %xmm10, %xmm10
    xorl %r10d, %r10d
    leaq 208(%rsp), %r8
.LBB9:
    movl $2000, %r9d
    subl %r10d, %r9d
    cmpl $3, %r9d
jg .LBB14
.LBB10:
    vmovapd %xmm10, %xmm11
    movq %r10, %rdi
    movq %r8, %rsi
.LBB11:
    cmpl $2000, %edi
jge .LBB13
.LBB12:
    leal (%rdi, %r11), %r10d
    leal 1(%r10), %r8d
    imull %r8d, %r10d
    movl %r10d, %r9d
    sarl $31, %r9d
    shrl $31, %r9d
    addl %r9d, %r10d
    sarl $1, %r10d
    addl %edi, %r10d
    leal 1(%r10), %eax
    vcvtsi2sdl %eax, %xmm0, %xmm12
    vdivsd %xmm12, %xmm9, %xmm13
    vfmadd231sd (%rsi), %xmm13, %xmm11
    addl $1, %edi
    leaq 8(%rsi), %rsi
    cmpl $2000, %edi
jl .LBB12
.LBB13:
    movslq %r11d, %r11
    vmovsd %xmm11, (%r12, %r11, 8)
    leal 1(%r11), %eax
    movl %eax, 28(%rsp)
    movl %eax, %r11d
    cmpl $2000, %r11d
jl .LBB8
    jmp .LBB18
.LBB14:
    leal 1(%r10), %r9d
    leal 2(%r10), %edi
    leal 3(%r10), %esi
    leal (%r10, %r11), %edx
    leal 1(%rdx), %r13d
    imull %r13d, %edx
    movl %edx, %r14d
    sarl $31, %r14d
    shrl $31, %r14d
    addl %r14d, %edx
    sarl $1, %edx
    addl %r10d, %edx
    leal 1(%rdx), %r15d
    leal (%r9, %r11), %edx
    movq %rdx, %rbp
    addl $1, %ebp
    imull %ebp, %edx
    movl %edx, %ebx
    sarl $31, %ebx
    shrl $31, %ebx
    addl %ebx, %edx
    sarl $1, %edx
    addl %r9d, %edx
    addl $1, %edx
    leal (%rdi, %r11), %r9d
    leal 1(%r9), %r13d
    imull %r13d, %r9d
    movl %r9d, %r14d
    sarl $31, %r14d
    shrl $31, %r14d
    addl %r14d, %r9d
    sarl $1, %r9d
    addl %edi, %r9d
    addl $1, %r9d
    leal (%rsi, %r11), %edi
    movq %rdi, %rbp
    addl $1, %ebp
    imull %ebp, %edi
    movl %edi, %ebx
    sarl $31, %ebx
    shrl $31, %ebx
    addl %ebx, %edi
    sarl $1, %edi
    addl %esi, %edi
    addl $1, %edi
    movslq %r10d, %rsi
    shlq $3, %rsi
    movd %r15d, %xmm0
    pinsrd $1, %edx, %xmm0
    pinsrd $2, %r9d, %xmm0
    pinsrd $3, %edi, %xmm0
    vcvtdq2pd %xmm0, %ymm0
    vbroadcastsd .Lvc2(%rip), %ymm1
    vdivpd %ymm0, %ymm1, %ymm0
    vmulpd 208(%rsp,%rsi), %ymm0, %ymm0
    vextractf128 $1, %ymm0, %xmm1
    vaddsd %xmm0, %xmm10, %xmm10
    vunpckhpd %xmm0, %xmm0, %xmm0
    vaddsd %xmm0, %xmm10, %xmm10
    vaddsd %xmm1, %xmm10, %xmm10
    vunpckhpd %xmm1, %xmm1, %xmm1
    vaddsd %xmm1, %xmm10, %xmm10
    leal 4(%r10), %eax
    movl %eax, 28(%rsp)
    leaq 32(%r8), %rax
    movl 28(%rsp), %r10d
    movq %rax, %r8
    movl $2000, %r9d
    subl %r10d, %r9d
    cmpl $3, %r9d
jg .LBB14
    jmp .LBB10
.LBB15:
    leal 1(%r10), %edi
    leal 2(%r10), %esi
    leal 3(%r10), %edx
    leal (%r11, %r10), %r9d
    leal 1(%r9), %r13d
    imull %r13d, %r9d
    movl %r9d, %r14d
    sarl $31, %r14d
    shrl $31, %r14d
    addl %r14d, %r9d
    sarl $1, %r9d
    addl %r11d, %r9d
    leal 1(%r9), %r15d
    leal (%r11, %rdi), %r9d
    leal 1(%r9), %edi
    imull %edi, %r9d
    movl %r9d, %edi
    sarl $31, %edi
    shrl $31, %edi
    addl %edi, %r9d
    sarl $1, %r9d
    addl %r11d, %r9d
    addl $1, %r9d
    leal (%r11, %rsi), %edi
    leal 1(%rdi), %esi
    imull %esi, %edi
    movl %edi, %esi
    sarl $31, %esi
    shrl $31, %esi
    addl %esi, %edi
    sarl $1, %edi
    addl %r11d, %edi
    addl $1, %edi
    leal (%r11, %rdx), %esi
    leal 1(%rsi), %edx
    imull %edx, %esi
    movl %esi, %edx
    sarl $31, %edx
    shrl $31, %edx
    addl %edx, %esi
    sarl $1, %esi
    addl %r11d, %esi
    addl $1, %esi
    movslq %r10d, %rdx
    shlq $3, %rdx
    movd %r15d, %xmm0
    pinsrd $1, %r9d, %xmm0
    pinsrd $2, %edi, %xmm0
    pinsrd $3, %esi, %xmm0
    vcvtdq2pd %xmm0, %ymm0
    vbroadcastsd .Lvc2(%rip), %ymm1
    vdivpd %ymm0, %ymm1, %ymm0
    vmulpd (%rbx,%rdx), %ymm0, %ymm0
    vextractf128 $1, %ymm0, %xmm1
    vaddsd %xmm0, %xmm3, %xmm3
    vunpckhpd %xmm0, %xmm0, %xmm0
    vaddsd %xmm0, %xmm3, %xmm3
    vaddsd %xmm1, %xmm3, %xmm3
    vunpckhpd %xmm1, %xmm1, %xmm1
    vaddsd %xmm1, %xmm3, %xmm3
    addl $4, %r10d
    leaq 32(%r8), %r8
    movl $2000, %r9d
    subl %r10d, %r9d
    cmpl $3, %r9d
jg .LBB15
    jmp .LBB4
.LBB16:
    vmovsd .LCFP_0(%rip), %xmm9
    xorl %r11d, %r11d
.LBB17:
    cmpl $2000, %r11d
jl .LBB8
.LBB18:
    vzeroupper
    addq $16216, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret

.Lvc2:
main:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $32016, %rsp
    xorl %ebx, %ebx
.LBB20:
    cmpq $2000, %rbx
jge .LBB22
.LBB21:
    movsd .LCFP_0(%rip), %xmm0
    movsd %xmm0, 16008(%rsp, %rbx, 8)
    addq $1, %rbx
    cmpq $2000, %rbx
jl .LBB21
.LBB22:
    xorl %r12d, %r12d
.LBB23:
    cmpl $10, %r12d
jge .LBB25
.LBB24:
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
jl .LBB24
.LBB25:
    xorpd %xmm2, %xmm2
    xorl %r13d, %r13d
    xorpd %xmm3, %xmm3
.LBB26:
    cmpq $2000, %r13
jge .LBB28
.LBB27:
    movsd 16008(%rsp, %r13, 8), %xmm4
    movsd 8(%rsp, %r13, 8), %xmm5
    vfmadd231sd %xmm5, %xmm4, %xmm3
    vfmadd231sd %xmm5, %xmm5, %xmm2
    addq $1, %r13
    cmpq $2000, %r13
jl .LBB27
.LBB28:
    vdivsd %xmm2, %xmm3, %xmm3
    vsqrtsd %xmm3, %xmm3, %xmm3
    leaq .Lstr0(%rip), %rdi
    movsd %xmm3, %xmm0
    movb $1, %al
    call printf@PLT
    xorl %eax, %eax
    addq $32016, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret

.LCFP_0:

