.Lstr0:

buf:
out:

rint:
    movsd %xmm0, %xmm2
    vaddsd .LCFP_0(%rip), %xmm2, %xmm3
    vsubsd .LCFP_0(%rip), %xmm3, %xmm3
    vsubsd .LCFP_0(%rip), %xmm2, %xmm4
    vaddsd .LCFP_0(%rip), %xmm4, %xmm4
    vcmpsd $13, .LCFP_1(%rip), %xmm2, %xmm1
    vmovsd %xmm3, %xmm3, %xmm0
    vblendvpd %xmm1, %xmm0, %xmm4, %xmm0
    movsd %xmm0, %xmm0
    ret

nearbyint:
    movsd %xmm0, %xmm2
    vaddsd .LCFP_0(%rip), %xmm2, %xmm3
    vsubsd .LCFP_0(%rip), %xmm3, %xmm3
    vsubsd .LCFP_0(%rip), %xmm2, %xmm4
    vaddsd .LCFP_0(%rip), %xmm4, %xmm4
    vcmpsd $13, .LCFP_1(%rip), %xmm2, %xmm1
    vmovsd %xmm3, %xmm3, %xmm0
    vblendvpd %xmm1, %xmm0, %xmm4, %xmm0
    movsd %xmm0, %xmm0
    ret

roundeven:
    movsd %xmm0, %xmm2
    vaddsd .LCFP_0(%rip), %xmm2, %xmm3
    vsubsd .LCFP_0(%rip), %xmm3, %xmm3
    vsubsd .LCFP_0(%rip), %xmm2, %xmm4
    vaddsd .LCFP_0(%rip), %xmm4, %xmm4
    vcmpsd $13, .LCFP_1(%rip), %xmm2, %xmm1
    vmovsd %xmm3, %xmm3, %xmm0
    vblendvpd %xmm1, %xmm0, %xmm4, %xmm0
    movsd %xmm0, %xmm0
    ret

floor:
    movsd %xmm0, %xmm2
    vaddsd .LCFP_0(%rip), %xmm2, %xmm3
    vsubsd .LCFP_0(%rip), %xmm3, %xmm3
    vsubsd .LCFP_0(%rip), %xmm2, %xmm4
    vaddsd .LCFP_0(%rip), %xmm4, %xmm4
    vcmpsd $13, .LCFP_1(%rip), %xmm2, %xmm1
    vmovsd %xmm3, %xmm3, %xmm0
    vblendvpd %xmm1, %xmm0, %xmm4, %xmm0
    movq %xmm0, %rsi
    movq %rsi, %xmm5
    vsubsd .LCFP_2(%rip), %xmm5, %xmm5
    movq %rsi, %xmm0
    vmovsd %xmm2, %xmm2, %xmm1
    ucomisd %xmm1, %xmm0
    movq %xmm5, %rcx
    cmovaq %rcx, %rsi
    movq %rsi, %rax
    movq %rsi, %xmm0
    ret

ceil:
    movsd %xmm0, %xmm2
    vaddsd .LCFP_0(%rip), %xmm2, %xmm3
    vsubsd .LCFP_0(%rip), %xmm3, %xmm3
    vsubsd .LCFP_0(%rip), %xmm2, %xmm4
    vaddsd .LCFP_0(%rip), %xmm4, %xmm4
    vcmpsd $13, .LCFP_1(%rip), %xmm2, %xmm1
    vmovsd %xmm3, %xmm3, %xmm0
    vblendvpd %xmm1, %xmm0, %xmm4, %xmm0
    movq %xmm0, %rsi
    movq %rsi, %xmm5
    vaddsd .LCFP_2(%rip), %xmm5, %xmm5
    vmovsd %xmm2, %xmm2, %xmm0
    movq %rsi, %xmm1
    ucomisd %xmm1, %xmm0
    movq %xmm5, %rcx
    cmovaq %rcx, %rsi
    movq %rsi, %rax
    movq %rsi, %xmm0
    ret

trunc:
    movsd %xmm0, %xmm2
    ucomisd .LCFP_1(%rip), %xmm2
    setae %al
    movzbl %al, %eax
    movq %rax, %rdi
    vaddsd .LCFP_0(%rip), %xmm2, %xmm3
    vsubsd .LCFP_0(%rip), %xmm3, %xmm3
    vsubsd .LCFP_0(%rip), %xmm2, %xmm4
    vaddsd .LCFP_0(%rip), %xmm4, %xmm4
    testb %al, %al
    movq %xmm3, %rcx
    movq %xmm4, %rsi
    cmovneq %rcx, %rsi
    testb %al, %al
je .LBB7
.LBB6:
    movq %rsi, %xmm5
    vsubsd .LCFP_2(%rip), %xmm5, %xmm5
    movq %rsi, %xmm0
    vmovsd %xmm2, %xmm2, %xmm1
    ucomisd %xmm1, %xmm0
    movq %xmm5, %rcx
    movq %rsi, %r8
    cmovaq %rcx, %r8
    movq %r8, %rax
    movq %r8, %xmm0
    ret
.LBB7:
    movq %rsi, %xmm6
    vaddsd .LCFP_2(%rip), %xmm6, %xmm6
    vmovsd %xmm2, %xmm2, %xmm0
    movq %rsi, %xmm1
    ucomisd %xmm1, %xmm0
    movq %xmm6, %rcx
    movq %rsi, %r11
    cmovaq %rcx, %r11
    movq %r11, %rax
    movq %r11, %xmm0
    ret

copysign:
    movsd %xmm0, %xmm2
    movsd %xmm1, %xmm3
    movq %xmm2, %rdi
    movabsq $9223372036854775807, %rax
    andq %rax, %rdi
    movq %xmm3, %rsi
    movabsq $-9223372036854775808, %rax
    andq %rax, %rsi
    orq %rsi, %rdi
    movq %rdi, %rax
    movq %rdi, %xmm0
    ret

fma:
    movsd %xmm1, %xmm3
    movsd %xmm2, %xmm4
    movsd %xmm0, %xmm2
    vfmadd132sd %xmm3, %xmm4, %xmm2
    movsd %xmm2, %xmm0
    ret

round_family_pass:
    leaq out(%rip), %rdi
    leaq buf(%rip), %rsi
    xorpd %xmm2, %xmm2
    xorl %edx, %edx
    movq %rsi, %r8
    movq %rdi, %r9
.LBB11:
    cmpl $4096, %edx
jge .LBB13
.LBB12:
    leal 2(%rdx), %r10d
    movsd (%r8), %xmm3
    vroundsd $9, %xmm3, %xmm3, %xmm4
    vroundsd $10, %xmm3, %xmm3, %xmm5
    vroundsd $11, %xmm3, %xmm3, %xmm6
    vroundsd $4, %xmm3, %xmm3, %xmm7
    vroundsd $12, %xmm3, %xmm3, %xmm8
    vroundsd $8, %xmm3, %xmm3, %xmm9
    vandpd .LCFP_4(%rip), %xmm3, %xmm1
    vandpd .LCFP_3(%rip), %xmm6, %xmm0
    vorpd %xmm1, %xmm0, %xmm0
    movsd %xmm0, %xmm10
    vmovsd %xmm5, %xmm5, %xmm11
    vfmadd231sd .LCFP_5(%rip), %xmm4, %xmm11
    vaddsd %xmm7, %xmm11, %xmm11
    vaddsd %xmm8, %xmm11, %xmm11
    vaddsd %xmm9, %xmm11, %xmm11
    vaddsd %xmm10, %xmm11, %xmm11
    movsd %xmm11, (%r9)
    vsubsd %xmm6, %xmm11, %xmm11
    vaddsd %xmm11, %xmm2, %xmm12
    movslq %edx, %rdx
    movsd 8(%rsi, %rdx, 8), %xmm13
    vroundsd $9, %xmm13, %xmm13, %xmm14
    vroundsd $10, %xmm13, %xmm13, %xmm15
    vroundsd $11, %xmm13, %xmm13, %xmm3
    vroundsd $4, %xmm13, %xmm13, %xmm4
    vroundsd $12, %xmm13, %xmm13, %xmm5
    vroundsd $8, %xmm13, %xmm13, %xmm6
    vandpd .LCFP_4(%rip), %xmm13, %xmm1
    vandpd .LCFP_3(%rip), %xmm3, %xmm0
    vorpd %xmm1, %xmm0, %xmm0
    movsd %xmm0, %xmm7
    vmovsd %xmm15, %xmm15, %xmm8
    vfmadd231sd .LCFP_5(%rip), %xmm14, %xmm8
    vaddsd %xmm4, %xmm8, %xmm8
    vaddsd %xmm5, %xmm8, %xmm8
    vaddsd %xmm6, %xmm8, %xmm8
    vaddsd %xmm7, %xmm8, %xmm8
    movslq %edx, %rdx
    movsd %xmm8, 8(%rdi, %rdx, 8)
    vsubsd %xmm3, %xmm8, %xmm8
    vaddsd %xmm8, %xmm12, %xmm2
    leaq 16(%r8), %r8
    leaq 16(%r9), %r9
    movl %r10d, %edx
    cmpl $4096, %edx
jl .LBB12
.LBB13:
    movsd %xmm2, %xmm0
    ret

main:
    pushq %rbx
    pushq %r12
    pushq %r13
    subq $32, %rsp
    leaq buf(%rip), %r11
    movsd .LCFP_6(%rip), %xmm2
    movsd .LCFP_7(%rip), %xmm3
    xorl %ebx, %ebx
.LBB15:
    cmpq $4096, %rbx
jge .LBB17
.LBB16:
    leaq -2048(%rbx), %rax
    vcvtsi2sdq %rax, %xmm0, %xmm4
    vmulsd %xmm2, %xmm4, %xmm4
    movq %rbx, %rax
    andq $3, %rax
    vcvtsi2sdq %rax, %xmm0, %xmm5
    vfmadd231sd %xmm3, %xmm5, %xmm4
    movsd %xmm4, (%r11, %rbx, 8)
    addq $1, %rbx
    cmpq $4096, %rbx
jl .LBB16
.LBB17:
    leaq buf(%rip), %r12
    xorl %eax, %eax
    movq %rax, 16(%rsp)
    xorl %r13d, %r13d
.LBB18:
    cmpq $20000, %r13
jae .LBB20
.LBB19:
    call round_family_pass
    movapd %xmm0, %xmm6
    vaddsd 16(%rsp), %xmm6, %xmm7
    movq %r13, %r9
    andq $4095, %r9
    movsd (%r12, %r9, 8), %xmm8
    movq %xmm8, %rax
    movabsq $-9223372036854775808, %rcx
    xorq %rcx, %rax
    movq %rax, %xmm9
    movsd %xmm9, (%r12, %r9, 8)
    addq $1, %r13
    movsd %xmm7, 16(%rsp)
    cmpq $20000, %r13
jb .LBB19
.LBB20:
    leaq .Lstr0(%rip), %rdi
    movq 16(%rsp), %rax
    movq %rax, %xmm0
    movb $1, %al
    call printf@PLT
    xorl %eax, %eax
    addq $32, %rsp
    popq %r13
    popq %r12
    popq %rbx
    ret

.LCFP_0:
.LCFP_1:
.LCFP_2:
.LCFP_3:
.LCFP_4:
.LCFP_5:
.LCFP_6:
.LCFP_7:

