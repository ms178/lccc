.Lstr0:

main:
    subq $8, %rsp
    movsd .LCFP_0(%rip), %xmm2
    movsd .LCFP_1(%rip), %xmm3
    movsd .LCFP_2(%rip), %xmm4
    movsd .LCFP_3(%rip), %xmm5
    movsd .LCFP_4(%rip), %xmm6
    xorl %r11d, %r11d
    xorl %r10d, %r10d
.LBB1:
    cmpl $4000, %r10d
jge .LBB10
.LBB2:
    vcvtsi2sdl %r10d, %xmm0, %xmm7
    vmulsd %xmm2, %xmm7, %xmm7
    vdivsd %xmm3, %xmm7, %xmm7
    vsubsd %xmm5, %xmm7, %xmm7
    movq %r11, %r8
    xorl %r9d, %r9d
.LBB3:
    cmpl $4000, %r9d
jge .LBB9
.LBB4:
    vcvtsi2sdl %r9d, %xmm0, %xmm8
    vmulsd %xmm2, %xmm8, %xmm8
    vdivsd %xmm3, %xmm8, %xmm8
    vsubsd %xmm4, %xmm8, %xmm8
    xorpd %xmm9, %xmm9
    xorl %edi, %edi
    xorpd %xmm10, %xmm10
.LBB5:
    cmpl $50, %edi
jge .LBB8
.LBB6:
    vmulsd %xmm9, %xmm9, %xmm11
    vfnmadd231sd %xmm10, %xmm10, %xmm11
    vaddsd %xmm8, %xmm11, %xmm11
    vmulsd %xmm2, %xmm9, %xmm9
    vfmadd132sd %xmm9, %xmm7, %xmm10
    vmulsd %xmm11, %xmm11, %xmm14
    vfmadd231sd %xmm10, %xmm10, %xmm14
    ucomisd %xmm6, %xmm14
ja .LBB8
.LBB7:
    addl $1, %edi
    vmovapd %xmm11, %xmm9
    cmpl $50, %edi
jl .LBB6
.LBB8:
    addl %edi, %r8d
    addl $1, %r9d
    cmpl $4000, %r9d
jl .LBB4
.LBB9:
    addl $1, %r10d
    movq %r8, %r11
    cmpl $4000, %r10d
jl .LBB2
.LBB10:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $8, %rsp
    ret

.LCFP_0:
.LCFP_1:
.LCFP_2:
.LCFP_3:
.LCFP_4:

