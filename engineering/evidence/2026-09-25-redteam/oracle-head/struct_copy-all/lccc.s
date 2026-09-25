.Lstr0:

main:
    subq $296, %rsp
    movsd .LCFP_0(%rip), %xmm2
    movsd .LCFP_1(%rip), %xmm3
    movsd .LCFP_2(%rip), %xmm4
    xorl %eax, %eax
    vcvtsi2sdl %eax, %xmm0, %xmm5
    vmulsd %xmm2, %xmm5, %xmm6
    vmulsd %xmm3, %xmm5, %xmm7
    vmulsd %xmm4, %xmm5, %xmm5
    movq %rax, 80(%rsp)
    xorl %r11d, %r11d
    movsd %xmm6, 72(%rsp)
    movsd %xmm7, 64(%rsp)
.LBB1:
    cmpl $2000000, %r11d
jge .LBB3
.LBB2:
    xorpd %xmm0, %xmm0
    movsd 72(%rsp), %xmm0
    movsd %xmm0, 88(%rsp)
    movsd 64(%rsp), %xmm0
    movsd %xmm0, 96(%rsp)
    movsd %xmm5, 104(%rsp)
    leal 1(%r11), %edi
    vcvtsi2sdl %edi, %xmm0, %xmm9
    vmulsd %xmm2, %xmm9, %xmm10
    vmulsd %xmm3, %xmm9, %xmm11
    vmulsd %xmm4, %xmm9, %xmm9
    leal 2(%r11), %r9d
    vcvtsi2sdl %r9d, %xmm0, %xmm12
    vmulsd %xmm2, %xmm12, %xmm13
    vmulsd %xmm3, %xmm12, %xmm14
    vmulsd %xmm4, %xmm12, %xmm12
    leal 3(%r11), %r9d
    vcvtsi2sdl %r9d, %xmm0, %xmm15
    vmulsd %xmm2, %xmm15, %xmm8
    vmulsd %xmm3, %xmm15, %xmm0
    movsd %xmm0, 56(%rsp)
    vmulsd %xmm4, %xmm15, %xmm15
    movq 72(%rsp), %rax
    movq %rax, 48(%rsp)
    movsd 48(%rsp), %xmm0
    vsubsd %xmm10, %xmm0, %xmm0
    movsd %xmm0, 40(%rsp)
    movsd 64(%rsp), %xmm6
    vsubsd %xmm11, %xmm6, %xmm0
    movsd %xmm0, 32(%rsp)
    vsubsd %xmm9, %xmm5, %xmm0
    movsd %xmm0, 24(%rsp)
    movsd 40(%rsp), %xmm0
    vmulsd 40(%rsp), %xmm0, %xmm0
    movsd %xmm0, 16(%rsp)
    movsd 32(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd %xmm0, 32(%rsp)
    movsd 24(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd %xmm0, 24(%rsp)
    xorpd %xmm0, %xmm0
    vaddsd 24(%rsp), %xmm0, %xmm0
    movsd %xmm0, 40(%rsp)
    movsd 48(%rsp), %xmm0
    vsubsd %xmm13, %xmm0, %xmm0
    movsd %xmm0, 24(%rsp)
    vsubsd %xmm14, %xmm6, %xmm0
    movsd %xmm0, 32(%rsp)
    vsubsd %xmm12, %xmm5, %xmm0
    movsd %xmm0, 16(%rsp)
    movsd 24(%rsp), %xmm0
    vmulsd 24(%rsp), %xmm0, %xmm0
    movsd 32(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd %xmm0, 32(%rsp)
    movsd 16(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd %xmm0, 16(%rsp)
    movsd 40(%rsp), %xmm0
    vaddsd 16(%rsp), %xmm0, %xmm0
    movsd %xmm0, 24(%rsp)
    movsd 48(%rsp), %xmm0
    vsubsd %xmm8, %xmm0, %xmm0
    movsd %xmm0, 16(%rsp)
    vsubsd 56(%rsp), %xmm6, %xmm6
    vsubsd %xmm15, %xmm5, %xmm7
    vmulsd 16(%rsp), %xmm0, %xmm0
    movsd %xmm0, 48(%rsp)
    vfmadd231sd %xmm6, %xmm6, %xmm0
    movsd %xmm0, 16(%rsp)
    movsd %xmm0, %xmm6
    vfmadd231sd %xmm7, %xmm7, %xmm6
    vaddsd 24(%rsp), %xmm6, %xmm7
    vsubsd %xmm13, %xmm10, %xmm6
    vsubsd %xmm14, %xmm11, %xmm0
    movsd %xmm0, 24(%rsp)
    vsubsd %xmm12, %xmm9, %xmm0
    movsd %xmm0, 16(%rsp)
    vmulsd %xmm6, %xmm6, %xmm6
    movsd %xmm6, %xmm0
    movsd 24(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd 16(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    vaddsd %xmm0, %xmm7, %xmm7
    vsubsd %xmm8, %xmm10, %xmm6
    vsubsd 56(%rsp), %xmm11, %xmm0
    movsd %xmm0, 48(%rsp)
    vsubsd %xmm15, %xmm9, %xmm0
    movsd %xmm0, 16(%rsp)
    vmulsd %xmm6, %xmm6, %xmm6
    movsd %xmm6, %xmm0
    movsd 48(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd 16(%rsp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    vaddsd %xmm0, %xmm7, %xmm7
    vsubsd %xmm8, %xmm13, %xmm13
    vsubsd 56(%rsp), %xmm14, %xmm14
    vsubsd %xmm15, %xmm12, %xmm12
    vmulsd %xmm13, %xmm13, %xmm13
    vfmadd231sd %xmm14, %xmm14, %xmm13
    vfmadd231sd %xmm12, %xmm12, %xmm13
    vaddsd %xmm13, %xmm7, %xmm7
    vaddsd 80(%rsp), %xmm7, %xmm8
    movsd %xmm8, 80(%rsp)
    movq %rdi, %r11
    movsd %xmm10, 72(%rsp)
    movsd %xmm11, 64(%rsp)
    vmovapd %xmm9, %xmm5
    cmpl $2000000, %edi
jl .LBB2
.LBB3:
    leaq .Lstr0(%rip), %rdi
    movq 80(%rsp), %rax
    movq %rax, %xmm0
    movb $1, %al
    call printf@PLT
    xorl %eax, %eax
    addq $296, %rsp
    ret

.LCFP_0:
.LCFP_1:
.LCFP_2:
