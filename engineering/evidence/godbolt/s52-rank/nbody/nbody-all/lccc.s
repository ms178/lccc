.Lstr0:

bodies:

main:
    pushq %rbp
    movq %rsp, %rbp
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    subq $120, %rsp
    xorpd %xmm15, %xmm15
    movsd .LCFP_0(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm14
    leaq bodies(%rip), %rbx
    movsd bodies+48(%rip), %xmm2
    vmovsd %xmm2, %xmm2, %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm13
    vmulpd 24(%rbx), %xmm13, %xmm12
    addpd %xmm12, %xmm15
    movsd bodies+40(%rip), %xmm3
    xorpd %xmm0, %xmm0
    vfmadd213sd %xmm0, %xmm2, %xmm3
    movsd bodies+104(%rip), %xmm4
    vmovsd %xmm4, %xmm4, %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm12
    vmulpd 80(%rbx), %xmm12, %xmm13
    addpd %xmm13, %xmm15
    movsd bodies+96(%rip), %xmm5
    vfmadd231sd %xmm4, %xmm5, %xmm3
    leaq 136(%rbx), %r8
    movsd bodies+160(%rip), %xmm6
    vmovsd %xmm6, %xmm6, %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm13
    vmulpd (%r8), %xmm13, %xmm12
    addpd %xmm12, %xmm15
    movsd bodies+152(%rip), %xmm7
    vfmadd231sd %xmm6, %xmm7, %xmm3
    leaq 192(%rbx), %r9
    movsd bodies+216(%rip), %xmm8
    vmovsd %xmm8, %xmm8, %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm12
    vmulpd (%r9), %xmm12, %xmm13
    addpd %xmm13, %xmm15
    movsd bodies+208(%rip), %xmm2
    vfmadd231sd %xmm8, %xmm2, %xmm3
    movsd bodies+272(%rip), %xmm4
    vmovsd %xmm4, %xmm4, %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm13
    vmulpd 248(%rbx), %xmm13, %xmm12
    addpd %xmm12, %xmm15
    movsd bodies+264(%rip), %xmm5
    vfmadd231sd %xmm4, %xmm5, %xmm3
    leaq 24(%rbx), %rsi
    vxorpd .LCVEC_1(%rip), %xmm15, %xmm15
    divpd %xmm14, %xmm15
    movupd %xmm15, (%rsi)
    movq %xmm3, %rax
    movabsq $-9223372036854775808, %rcx
    xorq %rcx, %rax
    movq %rax, %xmm6
    vdivsd .LCFP_0(%rip), %xmm6, %xmm6
    movsd %xmm6, bodies+40(%rip)
    movsd .LCFP_2(%rip), %xmm7
    xorpd %xmm8, %xmm8
    xorl %r11d, %r11d
    movq %rbx, %r10
.LBB1:
    cmpl $5, %r11d
jge .LBB6
.LBB2:
    leaq 48(%r10), %r12
    movsd (%r12), %xmm2
    vmulsd %xmm7, %xmm2, %xmm2
    movsd 24(%r10), %xmm3
    vmulsd %xmm3, %xmm3, %xmm3
    movsd 32(%r10), %xmm4
    vfmadd231sd %xmm4, %xmm4, %xmm3
    movsd 40(%r10), %xmm5
    vfmadd231sd %xmm5, %xmm5, %xmm3
    vmovsd %xmm8, %xmm8, %xmm6
    vfmadd231sd %xmm3, %xmm2, %xmm6
    leal 1(%r11), %eax
    movslq %eax, %rax
    movslq %eax, %r9
    leaq 16(%r10), %r13
    leaq 8(%r10), %rsi
    imulq $56, %r9, %r8
    leaq bodies(%rip), %rcx
    leaq (%rcx, %r8), %r8
.LBB3:
    cmpq $5, %r9
jge .LBB5
.LBB4:
    movsd (%r10), %xmm8
    vsubsd (%r8), %xmm8, %xmm8
    movsd (%rsi), %xmm3
    vsubsd 8(%r8), %xmm3, %xmm3
    movsd (%r13), %xmm5
    vsubsd 16(%r8), %xmm5, %xmm5
    movsd (%r12), %xmm4
    vmulsd 48(%r8), %xmm4, %xmm4
    vmulsd %xmm8, %xmm8, %xmm8
    vfmadd231sd %xmm3, %xmm3, %xmm8
    vfmadd231sd %xmm5, %xmm5, %xmm8
    vsqrtsd %xmm8, %xmm8, %xmm8
    vdivsd %xmm8, %xmm4, %xmm4
    vsubsd %xmm4, %xmm6, %xmm6
    addq $1, %r9
    leaq 56(%r8), %r8
    cmpq $5, %r9
jl .LBB4
.LBB5:
    addl $1, %r11d
    leaq 56(%r10), %rax
    movq %rax, -152(%rbp)
    vmovapd %xmm6, %xmm8
    movq %rax, %r10
    cmpl $5, %r11d
jl .LBB2
.LBB6:
    leaq .Lstr0(%rip), %rdi
    movq %xmm8, %rax
    movq %rax, %xmm0
    movb $1, %al
    call printf@PLT
    leaq 248(%rbx), %rax
    movq %rax, -72(%rbp)
    leaq 224(%rbx), %rax
    movq %rax, -80(%rbp)
    leaq 192(%rbx), %rax
    movq %rax, -88(%rbp)
    leaq 168(%rbx), %rax
    movq %rax, -96(%rbp)
    leaq 136(%rbx), %rax
    movq %rax, -104(%rbp)
    leaq 112(%rbx), %rax
    movq %rax, -112(%rbp)
    leaq 80(%rbx), %rax
    movq %rax, -120(%rbp)
    leaq 56(%rbx), %rax
    movq %rax, -128(%rbp)
    leaq 24(%rbx), %rax
    movq %rax, -136(%rbp)
    movsd .LCFP_3(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm15
    movsd .LCFP_3(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm14
    movsd .LCFP_3(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm12
    movsd .LCFP_3(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm13
    movsd .LCFP_3(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm11
    leaq bodies(%rip), %r15
    movsd .LCFP_3(%rip), %xmm4
    movq $0, -56(%rbp)
.LBB7:
    cmpl $5000000, -56(%rbp)
    setl %r9b
    movzbl %r9b, %r9d
    movsd .LCFP_2(%rip), %xmm5
    testb %r9b, %r9b
je .LBB15
.LBB8:
    movq $0, -64(%rbp)
    movq %r15, %r10
.LBB9:
    movl $0, %eax
    cmpl $5, %eax
jge .LBB14
.LBB10:
    movl -64(%rbp), %eax
    addl $1, %eax
    movslq %eax, %r11
    leaq 48(%r10), %r12
    leaq 40(%r10), %rdi
    leaq 32(%r10), %r9
    leaq 24(%r10), %r14
    leaq 16(%r10), %r13
    leaq 8(%r10), %rax
    movq %rax, -144(%rbp)
    imulq $56, %r11, %r8
    movq %r8, %rcx
    leaq bodies(%rip), %rax
    addq %r8, %rax
    movq %rax, -152(%rbp)
    movq %rax, %r8
.LBB11:
    cmpq $5, %r11
jge .LBB13
.LBB12:
    movsd (%r10), %xmm6
    vsubsd (%r8), %xmm6, %xmm6
    movq -144(%rbp), %rcx
    movsd (%rcx), %xmm8
    vsubsd 8(%r8), %xmm8, %xmm8
    movsd (%r13), %xmm3
    vsubsd 16(%r8), %xmm3, %xmm3
    vmulsd %xmm6, %xmm6, %xmm7
    vfmadd231sd %xmm8, %xmm8, %xmm7
    vfmadd231sd %xmm3, %xmm3, %xmm7
    vsqrtsd %xmm7, %xmm7, %xmm2
    vmulsd %xmm2, %xmm7, %xmm7
    vdivsd %xmm7, %xmm4, %xmm5
    movsd 48(%r8), %xmm7
    vmulsd %xmm7, %xmm6, %xmm2
    vmulsd %xmm5, %xmm2, %xmm2
    movsd (%r14), %xmm0
    movsd %xmm0, -152(%rbp)
    vsubsd %xmm2, %xmm0, %xmm0
    movsd %xmm0, -160(%rbp)
    movsd %xmm0, (%r14)
    vmulsd %xmm7, %xmm8, %xmm2
    vmulsd %xmm5, %xmm2, %xmm2
    movsd (%r9), %xmm0
    movsd %xmm0, -160(%rbp)
    vsubsd %xmm2, %xmm0, %xmm0
    movsd %xmm0, -152(%rbp)
    movsd %xmm0, (%r9)
    vmulsd %xmm7, %xmm3, %xmm2
    vmulsd %xmm5, %xmm2, %xmm2
    movsd (%rdi), %xmm7
    vsubsd %xmm2, %xmm7, %xmm7
    movsd %xmm7, (%rdi)
    movsd (%r12), %xmm2
    vmulsd %xmm2, %xmm6, %xmm6
    leaq 24(%r8), %rsi
    movsd (%rsi), %xmm7
    vfmadd231sd %xmm5, %xmm6, %xmm7
    movsd %xmm7, (%rsi)
    vmulsd %xmm2, %xmm8, %xmm8
    leaq 32(%r8), %rsi
    movsd (%rsi), %xmm6
    vfmadd231sd %xmm5, %xmm8, %xmm6
    movsd %xmm6, (%rsi)
    vmulsd %xmm2, %xmm3, %xmm3
    leaq 40(%r8), %rsi
    movsd (%rsi), %xmm7
    vfmadd231sd %xmm5, %xmm3, %xmm7
    movsd %xmm7, (%rsi)
    addq $1, %r11
    leaq 56(%r8), %r8
    cmpq $5, %r11
jl .LBB12
.LBB13:
    movl -64(%rbp), %eax
    addl $1, %eax
    movq %rax, -64(%rbp)
    leaq 56(%r10), %rax
    movq %rax, -160(%rbp)
    movq %rax, %r10
    movl -64(%rbp), %eax
    cmpl $5, %eax
jl .LBB10
.LBB14:
    movq -136(%rbp), %rax
    movupd (%rax), %xmm10
    mulpd %xmm15, %xmm10
    vaddpd (%rbx), %xmm10, %xmm9
    movupd %xmm9, (%rbx)
    movsd bodies+40(%rip), %xmm8
    movsd bodies+16(%rip), %xmm2
    vfmadd231sd %xmm4, %xmm8, %xmm2
    movsd %xmm2, bodies+16(%rip)
    movq -120(%rbp), %rax
    movupd (%rax), %xmm9
    mulpd %xmm14, %xmm9
    movq -128(%rbp), %rax
    movupd (%rax), %xmm10
    addpd %xmm9, %xmm10
    movupd %xmm10, (%rax)
    movsd bodies+96(%rip), %xmm3
    movsd bodies+72(%rip), %xmm5
    vfmadd231sd %xmm4, %xmm3, %xmm5
    movsd %xmm5, bodies+72(%rip)
    movq -104(%rbp), %rax
    movupd (%rax), %xmm10
    mulpd %xmm12, %xmm10
    movq -112(%rbp), %rax
    movupd (%rax), %xmm9
    addpd %xmm10, %xmm9
    movupd %xmm9, (%rax)
    movsd bodies+152(%rip), %xmm6
    movsd bodies+128(%rip), %xmm7
    vfmadd231sd %xmm4, %xmm6, %xmm7
    movsd %xmm7, bodies+128(%rip)
    movq -88(%rbp), %rax
    movupd (%rax), %xmm9
    mulpd %xmm13, %xmm9
    movq -96(%rbp), %rax
    movupd (%rax), %xmm10
    addpd %xmm9, %xmm10
    movupd %xmm10, (%rax)
    movsd bodies+208(%rip), %xmm8
    movsd bodies+184(%rip), %xmm2
    vfmadd231sd %xmm4, %xmm8, %xmm2
    movsd %xmm2, bodies+184(%rip)
    movq -72(%rbp), %rax
    movupd (%rax), %xmm10
    mulpd %xmm11, %xmm10
    movq -80(%rbp), %rax
    movupd (%rax), %xmm9
    addpd %xmm10, %xmm9
    movupd %xmm9, (%rax)
    movsd bodies+264(%rip), %xmm3
    movsd bodies+240(%rip), %xmm5
    vfmadd231sd %xmm4, %xmm3, %xmm5
    movsd %xmm5, bodies+240(%rip)
    movl -56(%rbp), %eax
    addl $1, %eax
    movq %rax, -56(%rbp)
    jmp .LBB7
.LBB15:
    xorpd %xmm6, %xmm6
    xorl %r11d, %r11d
    movq %r15, %r10
.LBB16:
    cmpl $5, %r11d
jge .LBB21
.LBB17:
    leaq 48(%r10), %r12
    movsd (%r12), %xmm7
    vmulsd %xmm5, %xmm7, %xmm7
    movsd 24(%r10), %xmm8
    vmulsd %xmm8, %xmm8, %xmm8
    movsd 32(%r10), %xmm2
    vfmadd231sd %xmm2, %xmm2, %xmm8
    movsd 40(%r10), %xmm3
    vfmadd231sd %xmm3, %xmm3, %xmm8
    vmovsd %xmm6, %xmm6, %xmm4
    vfmadd231sd %xmm8, %xmm7, %xmm4
    leal 1(%r11), %eax
    movslq %eax, %r9
    leaq 16(%r10), %r13
    leaq 8(%r10), %rsi
    imulq $56, %r9, %r8
    leaq bodies(%rip), %r14
    addq %r8, %r14
    movq %r14, %r8
.LBB18:
    cmpq $5, %r9
jge .LBB20
.LBB19:
    movsd (%r10), %xmm6
    vsubsd (%r8), %xmm6, %xmm6
    movsd (%rsi), %xmm8
    vsubsd 8(%r8), %xmm8, %xmm8
    movsd (%r13), %xmm3
    vsubsd 16(%r8), %xmm3, %xmm3
    movsd (%r12), %xmm2
    vmulsd 48(%r8), %xmm2, %xmm2
    vmulsd %xmm6, %xmm6, %xmm6
    vfmadd231sd %xmm8, %xmm8, %xmm6
    vfmadd231sd %xmm3, %xmm3, %xmm6
    vsqrtsd %xmm6, %xmm6, %xmm6
    vdivsd %xmm6, %xmm2, %xmm2
    vsubsd %xmm2, %xmm4, %xmm4
    addq $1, %r9
    leaq 56(%r8), %r8
    cmpq $5, %r9
jl .LBB19
.LBB20:
    addl $1, %r11d
    leaq 56(%r10), %rax
    movq %rax, -152(%rbp)
    vmovapd %xmm4, %xmm6
    movq %rax, %r10
    cmpl $5, %r11d
jl .LBB17
.LBB21:
    leaq .Lstr0(%rip), %rdi
    movsd %xmm6, %xmm0
    movb $1, %al
    call printf@PLT
    xorl %eax, %eax
    leaq -40(%rbp), %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    popq %rbp
    ret

.LCFP_0:
.LCFP_2:
.LCFP_3:
.LCVEC_1:

