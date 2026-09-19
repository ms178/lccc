.LCPI0_0:
.LCPI0_1:
rint:
  vmovsd .LCPI0_0(%rip), %xmm1
  vaddsd %xmm1, %xmm0, %xmm2
  vmovsd .LCPI0_1(%rip), %xmm3
  vaddsd %xmm3, %xmm2, %xmm2
  vaddsd %xmm3, %xmm0, %xmm3
  vaddsd %xmm1, %xmm3, %xmm1
  vxorpd %xmm3, %xmm3, %xmm3
  vcmpnlesd %xmm0, %xmm3, %xmm0
  vblendvpd %xmm0, %xmm1, %xmm2, %xmm0
  retq

.LCPI1_0:
.LCPI1_1:
nearbyint:
  vmovsd .LCPI1_0(%rip), %xmm1
  vaddsd %xmm1, %xmm0, %xmm2
  vmovsd .LCPI1_1(%rip), %xmm3
  vaddsd %xmm3, %xmm2, %xmm2
  vaddsd %xmm3, %xmm0, %xmm3
  vaddsd %xmm1, %xmm3, %xmm1
  vxorpd %xmm3, %xmm3, %xmm3
  vcmpnlesd %xmm0, %xmm3, %xmm0
  vblendvpd %xmm0, %xmm1, %xmm2, %xmm0
  retq

.LCPI2_0:
.LCPI2_1:
roundeven:
  vmovsd .LCPI2_0(%rip), %xmm1
  vaddsd %xmm1, %xmm0, %xmm2
  vmovsd .LCPI2_1(%rip), %xmm3
  vaddsd %xmm3, %xmm2, %xmm2
  vaddsd %xmm3, %xmm0, %xmm3
  vaddsd %xmm1, %xmm3, %xmm1
  vxorpd %xmm3, %xmm3, %xmm3
  vcmpnlesd %xmm0, %xmm3, %xmm0
  vblendvpd %xmm0, %xmm1, %xmm2, %xmm0
  retq

.LCPI3_0:
.LCPI3_1:
.LCPI3_2:
floor:
  vmovsd .LCPI3_0(%rip), %xmm1
  vaddsd %xmm1, %xmm0, %xmm2
  vmovsd .LCPI3_1(%rip), %xmm3
  vaddsd %xmm3, %xmm2, %xmm2
  vaddsd %xmm3, %xmm0, %xmm3
  vaddsd %xmm1, %xmm3, %xmm1
  vxorpd %xmm3, %xmm3, %xmm3
  vcmpnlesd %xmm0, %xmm3, %xmm3
  vblendvpd %xmm3, %xmm1, %xmm2, %xmm1
  vaddsd .LCPI3_2(%rip), %xmm1, %xmm2
  vcmpltsd %xmm1, %xmm0, %xmm0
  vblendvpd %xmm0, %xmm2, %xmm1, %xmm0
  retq

.LCPI4_0:
.LCPI4_1:
.LCPI4_2:
ceil:
  vmovsd .LCPI4_0(%rip), %xmm1
  vaddsd %xmm1, %xmm0, %xmm2
  vmovsd .LCPI4_1(%rip), %xmm3
  vaddsd %xmm3, %xmm2, %xmm2
  vaddsd %xmm3, %xmm0, %xmm3
  vaddsd %xmm1, %xmm3, %xmm1
  vxorpd %xmm3, %xmm3, %xmm3
  vcmpnlesd %xmm0, %xmm3, %xmm3
  vblendvpd %xmm3, %xmm1, %xmm2, %xmm1
  vaddsd .LCPI4_2(%rip), %xmm1, %xmm2
  vcmpltsd %xmm0, %xmm1, %xmm0
  vblendvpd %xmm0, %xmm2, %xmm1, %xmm0
  retq

.LCPI5_0:
.LCPI5_1:
.LCPI5_2:
.LCPI5_3:
trunc:
  vxorpd %xmm1, %xmm1, %xmm1
  vucomisd %xmm1, %xmm0
  jae .LBB5_1
  vaddsd .LCPI5_1(%rip), %xmm0, %xmm1
  vaddsd .LCPI5_0(%rip), %xmm1, %xmm1
  vaddsd .LCPI5_3(%rip), %xmm1, %xmm2
  vcmpltsd %xmm0, %xmm1, %xmm0
  vblendvpd %xmm0, %xmm2, %xmm1, %xmm0
  retq
.LBB5_1:
  vaddsd .LCPI5_0(%rip), %xmm0, %xmm1
  vaddsd .LCPI5_1(%rip), %xmm1, %xmm1
  vaddsd .LCPI5_2(%rip), %xmm1, %xmm2
  vcmpltsd %xmm1, %xmm0, %xmm0
  vblendvpd %xmm0, %xmm2, %xmm1, %xmm0
  retq

.LCPI6_0:
.LCPI6_1:
copysign:
  vandps .LCPI6_0(%rip), %xmm1, %xmm1
  vandps .LCPI6_1(%rip), %xmm0, %xmm0
  vorps %xmm1, %xmm0, %xmm0
  retq

fma:
  vfmadd213sd %xmm2, %xmm1, %xmm0
  retq

.LCPI8_0:
.LCPI8_1:
.LCPI8_2:
.LCPI8_3:
.LCPI8_4:
.LCPI8_5:
.LCPI8_8:
.LCPI8_6:
.LCPI8_7:
main:
  pushq %rbp
  pushq %rbx
  pushq %rax
  vmovdqa .LCPI8_0(%rip), %xmm0
  xorl %eax, %eax
  vpbroadcastd .LCPI8_1(%rip), %xmm1
  vpbroadcastd .LCPI8_2(%rip), %xmm2
  vpbroadcastd .LCPI8_3(%rip), %xmm3
  vpbroadcastd .LCPI8_4(%rip), %xmm4
  vpbroadcastd .LCPI8_5(%rip), %xmm5
  vbroadcastsd .LCPI8_6(%rip), %ymm6
  vbroadcastsd .LCPI8_7(%rip), %ymm7
  leaq buf(%rip), %rbx
  vpbroadcastd .LCPI8_8(%rip), %xmm8
  vmovdqa %xmm0, %xmm9
.LBB8_1:
  vpaddd %xmm1, %xmm0, %xmm10
  vpaddd %xmm2, %xmm0, %xmm11
  vpaddd %xmm3, %xmm0, %xmm12
  vpaddd %xmm4, %xmm0, %xmm13
  vcvtdq2pd %xmm10, %ymm10
  vcvtdq2pd %xmm11, %ymm11
  vcvtdq2pd %xmm12, %ymm12
  vcvtdq2pd %xmm13, %ymm13
  vpand %xmm5, %xmm9, %xmm14
  vcvtdq2pd %xmm14, %ymm14
  vmulpd %ymm6, %ymm14, %ymm14
  vfmadd213pd %ymm14, %ymm7, %ymm10
  vfmadd213pd %ymm14, %ymm7, %ymm11
  vfmadd213pd %ymm14, %ymm7, %ymm12
  vfmadd213pd %ymm14, %ymm7, %ymm13
  vmovupd %ymm10, (%rbx,%rax,8)
  vmovupd %ymm11, 32(%rbx,%rax,8)
  vmovupd %ymm12, 64(%rbx,%rax,8)
  vmovupd %ymm13, 96(%rbx,%rax,8)
  addq $16, %rax
  vpaddd %xmm0, %xmm8, %xmm0
  vpaddd %xmm8, %xmm9, %xmm9
  cmpq $4096, %rax
  jne .LBB8_1
  vpxor %xmm0, %xmm0, %xmm0
  xorl %ebp, %ebp
.LBB8_3:
  vmovq %xmm0, (%rsp)
  vzeroupper
  callq round_family_pass
  movl %ebp, %eax
  andl $4095, %eax
  xorb $-128, 7(%rbx,%rax,8)
  vmovsd (%rsp), %xmm1
  vaddsd %xmm0, %xmm1, %xmm1
  vmovsd %xmm1, (%rsp)
  vmovsd (%rsp), %xmm0
  incl %ebp
  cmpl $20000, %ebp
  jne .LBB8_3
  leaq .L.str(%rip), %rdi
  movb $1, %al
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %rbp
  retq

.LCPI9_2:
.LCPI9_3:
.LCPI9_4:
round_family_pass:
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %eax, %eax
  leaq buf(%rip), %rcx
  vmovddup .LCPI9_3(%rip), %xmm1
  vmovddup .LCPI9_4(%rip), %xmm2
  vmovsd .LCPI9_2(%rip), %xmm3
.LBB9_1:
  vmovsd (%rcx,%rax,8), %xmm4
  vmovsd 8(%rcx,%rax,8), %xmm5
  vroundsd $9, %xmm4, %xmm4, %xmm6
  vroundsd $10, %xmm4, %xmm4, %xmm7
  vroundsd $11, %xmm4, %xmm4, %xmm8
  vroundsd $4, %xmm4, %xmm4, %xmm9
  vroundsd $12, %xmm4, %xmm4, %xmm10
  vroundsd $8, %xmm4, %xmm4, %xmm11
  vandpd %xmm1, %xmm8, %xmm12
  vandpd %xmm2, %xmm4, %xmm4
  vorpd %xmm4, %xmm12, %xmm4
  vfmadd231sd %xmm6, %xmm3, %xmm7
  vaddsd %xmm7, %xmm9, %xmm6
  vaddsd %xmm6, %xmm10, %xmm6
  vaddsd %xmm6, %xmm11, %xmm6
  vaddsd %xmm6, %xmm4, %xmm4
  vsubsd %xmm8, %xmm4, %xmm4
  vaddsd %xmm4, %xmm0, %xmm0
  vroundsd $9, %xmm5, %xmm5, %xmm4
  vroundsd $10, %xmm5, %xmm5, %xmm6
  vroundsd $11, %xmm5, %xmm5, %xmm7
  vroundsd $4, %xmm5, %xmm5, %xmm8
  vroundsd $12, %xmm5, %xmm5, %xmm9
  vroundsd $8, %xmm5, %xmm5, %xmm10
  vandnpd %xmm5, %xmm1, %xmm5
  vandpd %xmm1, %xmm7, %xmm11
  vorpd %xmm5, %xmm11, %xmm5
  vfmadd231sd %xmm4, %xmm3, %xmm6
  vaddsd %xmm6, %xmm8, %xmm4
  vaddsd %xmm4, %xmm9, %xmm4
  vaddsd %xmm4, %xmm10, %xmm4
  vaddsd %xmm4, %xmm5, %xmm4
  vsubsd %xmm7, %xmm4, %xmm4
  vaddsd %xmm4, %xmm0, %xmm0
  addq $2, %rax
  cmpq $4096, %rax
  jne .LBB9_1
  retq

.L.str:

