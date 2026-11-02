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
    xorpd %xmm0, %xmm0
    movdqu %xmm0, -64(%rbp)
    movsd .LCFP_0(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm15
    leaq bodies(%rip), %rbx
    movsd bodies+48(%rip), %xmm2
    vmovddup %xmm2, %xmm14
    movupd 24(%rbx), %xmm13
    movdqu -64(%rbp), %xmm3
    vfmadd132pd %xmm14, %xmm3, %xmm13
    movsd bodies+40(%rip), %xmm3
    xorpd %xmm0, %xmm0
    vfmadd132sd %xmm2, %xmm0, %xmm3
    movsd bodies+104(%rip), %xmm4
    vmovddup %xmm4, %xmm14
    movupd 80(%rbx), %xmm12
    vfmadd132pd %xmm14, %xmm13, %xmm12
    vfmadd231sd bodies+96(%rip), %xmm4, %xmm3
    movsd bodies+160(%rip), %xmm6
    vmovddup %xmm6, %xmm14
    movupd 136(%rbx), %xmm13
    vfmadd132pd %xmm14, %xmm12, %xmm13
    vfmadd231sd bodies+152(%rip), %xmm6, %xmm3
    movsd bodies+216(%rip), %xmm8
    vmovddup %xmm8, %xmm14
    movupd 192(%rbx), %xmm12
    vfmadd132pd %xmm14, %xmm13, %xmm12
    vfmadd231sd bodies+208(%rip), %xmm8, %xmm3
    movsd bodies+272(%rip), %xmm10
    vmovddup %xmm10, %xmm14
    movupd 248(%rbx), %xmm13
    vfmadd132pd %xmm14, %xmm12, %xmm13
    vfmadd231sd bodies+264(%rip), %xmm10, %xmm3
    vxorpd .LCVEC_1(%rip), %xmm13, %xmm13
    vdivpd %xmm15, %xmm13, %xmm13
    movupd %xmm13, 24(%rbx)
    vxorpd .LCFP_2(%rip), %xmm3, %xmm2
    vdivsd .LCFP_0(%rip), %xmm2, %xmm2
    movsd %xmm2, bodies+40(%rip)
    movsd .LCFP_3(%rip), %xmm3
    xorpd %xmm4, %xmm4
    xorl %r11d, %r11d
    movq %rbx, %r10
.LBB1:
    cmpl $5, %r11d
jge .LBB6
.LBB2:
    leaq 48(%r10), %r12
    movsd (%r12), %xmm5
    vmulsd %xmm3, %xmm5, %xmm5
    movsd 24(%r10), %xmm6
    vmulsd %xmm6, %xmm6, %xmm6
    movsd 32(%r10), %xmm7
    vfmadd231sd %xmm7, %xmm7, %xmm6
    movsd 40(%r10), %xmm8
    vfmadd231sd %xmm8, %xmm8, %xmm6
    vmovsd %xmm4, %xmm4, %xmm9
    vfmadd231sd %xmm6, %xmm5, %xmm9
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
    movsd (%r10), %xmm10
    vsubsd (%r8), %xmm10, %xmm10
    movsd (%rsi), %xmm2
    vsubsd 8(%r8), %xmm2, %xmm2
    movsd (%r13), %xmm5
    vsubsd 16(%r8), %xmm5, %xmm5
    movsd (%r12), %xmm7
    vmulsd 48(%r8), %xmm7, %xmm7
    vmulsd %xmm10, %xmm10, %xmm10
    vfmadd231sd %xmm2, %xmm2, %xmm10
    vfmadd231sd %xmm5, %xmm5, %xmm10
    vsqrtsd %xmm10, %xmm10, %xmm10
    vdivsd %xmm10, %xmm7, %xmm7
    vsubsd %xmm7, %xmm9, %xmm9
    addq $1, %r9
    leaq 56(%r8), %r8
    cmpq $5, %r9
jl .LBB4
.LBB5:
    addl $1, %r11d
    leaq 56(%r10), %rax
    movq %rax, -152(%rbp)
    vmovapd %xmm9, %xmm4
    movq %rax, %r10
    cmpl $5, %r11d
jl .LBB2
.LBB6:
    leaq .Lstr0(%rip), %rdi
    movq %xmm4, %rax
    movq %rax, %xmm0
    movb $1, %al
    call printf@PLT
    leaq 248(%rbx), %rax
    movq %rax, -80(%rbp)
    leaq 224(%rbx), %rax
    movq %rax, -88(%rbp)
    leaq 192(%rbx), %rax
    movq %rax, -96(%rbp)
    leaq 168(%rbx), %rax
    movq %rax, -104(%rbp)
    leaq 136(%rbx), %rax
    movq %rax, -112(%rbp)
    leaq 112(%rbx), %rax
    movq %rax, -120(%rbp)
    leaq 80(%rbx), %rax
    movq %rax, -128(%rbp)
    leaq 56(%rbx), %rax
    movq %rax, -136(%rbp)
    leaq 24(%rbx), %rax
    movq %rax, -144(%rbp)
    movsd .LCFP_4(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm13
    movsd .LCFP_4(%rip), %xmm11
    movq $0, -72(%rbp)
.LBB7:
    cmpl $5000000, -72(%rbp)
    setl %r10b
    movzbl %r10b, %r10d
    movsd .LCFP_3(%rip), %xmm2
    testb %r10b, %r10b
je .LBB15
.LBB8:
    xorl %r13d, %r13d
.LBB9:
    cmpl $5, %r13d
jge .LBB14
.LBB10:
    leal 1(%r13), %r9d
    movslq %r13d, %r11
    imulq $56, %r11, %r11
    leaq bodies(%rip), %rcx
    leaq (%rcx, %r11), %r15
    leaq 24(%r15), %r10
.LBB11:
    cmpl $5, %r9d
jge .LBB13
.LBB12:
    movslq %r9d, %r8
    imulq $56, %r8, %r8
    leaq bodies(%rip), %rcx
    leaq (%rcx, %r8), %rsi
    movupd (%rbx,%r11), %xmm15
    vsubpd (%rbx,%r8), %xmm15, %xmm15
    movapd %xmm15, %xmm0
    movsd %xmm15, -152(%rbp)
    pshufd $0x0E, %xmm15, %xmm0
    movsd %xmm0, -160(%rbp)
    movsd 16(%r15), %xmm3
    vsubsd 16(%rsi), %xmm3, %xmm3
    movsd -152(%rbp), %xmm5
    vmulsd %xmm5, %xmm5, %xmm5
    movsd %xmm5, %xmm0
    movsd -160(%rbp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd %xmm0, %xmm5
    vfmadd231sd %xmm3, %xmm3, %xmm5
    vsqrtsd %xmm5, %xmm5, %xmm8
    vmulsd %xmm8, %xmm5, %xmm5
    vdivsd %xmm5, %xmm11, %xmm9
    vmovddup %xmm9, %xmm14
    movsd 48(%rsi), %xmm10
    vmovddup %xmm10, %xmm12
    vmulpd %xmm12, %xmm15, %xmm12
    vfnmadd213pd 24(%rbx,%r11), %xmm14, %xmm12
    movupd %xmm12, 24(%rbx,%r11)
    vmulsd %xmm10, %xmm3, %xmm4
    movsd 40(%r15), %xmm5
    vfnmadd231sd %xmm9, %xmm4, %xmm5
    movsd %xmm5, 40(%r15)
    movsd 48(%r15), %xmm6
    vmovddup %xmm6, %xmm12
    vmulpd %xmm12, %xmm15, %xmm15
    vfmadd213pd 24(%rbx,%r8), %xmm14, %xmm15
    movupd %xmm15, 24(%rbx,%r8)
    vmulsd %xmm6, %xmm3, %xmm3
    movsd 40(%rsi), %xmm8
    vfmadd231sd %xmm9, %xmm3, %xmm8
    movsd %xmm8, 40(%rsi)
    addl $1, %r9d
    cmpl $5, %r9d
jl .LBB12
.LBB13:
    addl $1, %r13d
    cmpl $5, %r13d
jl .LBB10
.LBB14:
    movq -144(%rbp), %rax
    movupd (%rax), %xmm15
    vfmadd213pd (%rbx), %xmm13, %xmm15
    movupd %xmm15, (%rbx)
    movsd bodies+40(%rip), %xmm10
    movsd bodies+16(%rip), %xmm2
    vfmadd231sd %xmm11, %xmm10, %xmm2
    movsd %xmm2, bodies+16(%rip)
    movq -128(%rbp), %rax
    movupd (%rax), %xmm15
    movq -136(%rbp), %rax
    movupd (%rax), %xmm3
    vfmadd132pd %xmm13, %xmm3, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+96(%rip), %xmm4
    movsd bodies+72(%rip), %xmm5
    vfmadd231sd %xmm11, %xmm4, %xmm5
    movsd %xmm5, bodies+72(%rip)
    movq -112(%rbp), %rax
    movupd (%rax), %xmm15
    movq -120(%rbp), %rax
    movupd (%rax), %xmm6
    vfmadd132pd %xmm13, %xmm6, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+152(%rip), %xmm7
    movsd bodies+128(%rip), %xmm8
    vfmadd231sd %xmm11, %xmm7, %xmm8
    movsd %xmm8, bodies+128(%rip)
    movq -96(%rbp), %rax
    movupd (%rax), %xmm15
    movq -104(%rbp), %rax
    movupd (%rax), %xmm9
    vfmadd132pd %xmm13, %xmm9, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+208(%rip), %xmm10
    movsd bodies+184(%rip), %xmm2
    vfmadd231sd %xmm11, %xmm10, %xmm2
    movsd %xmm2, bodies+184(%rip)
    movq -80(%rbp), %rax
    movupd (%rax), %xmm15
    movq -88(%rbp), %rax
    movupd (%rax), %xmm3
    vfmadd132pd %xmm13, %xmm3, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+264(%rip), %xmm4
    movsd bodies+240(%rip), %xmm5
    vfmadd231sd %xmm11, %xmm4, %xmm5
    movsd %xmm5, bodies+240(%rip)
    movl -72(%rbp), %eax
    addl $1, %eax
    movq %rax, -72(%rbp)
    jmp .LBB7
.LBB15:
    xorpd %xmm6, %xmm6
    xorl %r9d, %r9d
    leaq bodies(%rip), %rdi
.LBB16:
    cmpl $5, %r9d
jge .LBB21
.LBB17:
    leaq 48(%rdi), %r15
    movsd (%r15), %xmm7
    vmulsd %xmm2, %xmm7, %xmm7
    movsd 24(%rdi), %xmm8
    vmulsd %xmm8, %xmm8, %xmm8
    movsd 32(%rdi), %xmm9
    vfmadd231sd %xmm9, %xmm9, %xmm8
    movsd 40(%rdi), %xmm10
    vfmadd231sd %xmm10, %xmm10, %xmm8
    vmovsd %xmm6, %xmm6, %xmm11
    vfmadd231sd %xmm8, %xmm7, %xmm11
    leal 1(%r9), %eax
    movslq %eax, %r11
    leaq 16(%rdi), %r12
    leaq 8(%rdi), %r8
    imulq $56, %r11, %rsi
    leaq bodies(%rip), %r13
    addq %rsi, %r13
    movq %r13, %rsi
.LBB18:
    cmpq $5, %r11
jge .LBB20
.LBB19:
    movsd (%rdi), %xmm3
    vsubsd (%rsi), %xmm3, %xmm3
    movsd (%r8), %xmm5
    vsubsd 8(%rsi), %xmm5, %xmm5
    movsd (%r12), %xmm7
    vsubsd 16(%rsi), %xmm7, %xmm7
    movsd (%r15), %xmm9
    vmulsd 48(%rsi), %xmm9, %xmm9
    vmulsd %xmm3, %xmm3, %xmm3
    vfmadd231sd %xmm5, %xmm5, %xmm3
    vfmadd231sd %xmm7, %xmm7, %xmm3
    vsqrtsd %xmm3, %xmm3, %xmm3
    vdivsd %xmm3, %xmm9, %xmm9
    vsubsd %xmm9, %xmm11, %xmm11
    addq $1, %r11
    leaq 56(%rsi), %rsi
    cmpq $5, %r11
jl .LBB19
.LBB20:
    addl $1, %r9d
    leaq 56(%rdi), %rax
    movq %rax, -152(%rbp)
    vmovapd %xmm11, %xmm6
    movq %rax, %rdi
    cmpl $5, %r9d
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
.LCFP_4:
.LCVEC_1:
