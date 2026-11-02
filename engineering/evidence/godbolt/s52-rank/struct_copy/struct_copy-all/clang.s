.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
main:
  vxorpd %xmm1, %xmm1, %xmm1
  xorl %eax, %eax
  vmovdqa .LCPI0_0(%rip), %xmm2
  vbroadcastsd .LCPI0_1(%rip), %ymm3
  vbroadcastsd .LCPI0_2(%rip), %ymm4
  vbroadcastsd .LCPI0_3(%rip), %ymm5
  vxorpd %xmm0, %xmm0, %xmm0
.LBB0_1:
  vmovd %eax, %xmm6
  vpbroadcastd %xmm6, %xmm6
  vpaddd %xmm2, %xmm6, %xmm7
  vpunpckldq %xmm7, %xmm6, %xmm6
  vcvtdq2pd %xmm6, %xmm6
  incl %eax
  vcvtdq2pd %xmm7, %ymm12
  vmulpd %xmm3, %xmm6, %xmm10
  vpermpd $64, %ymm10, %ymm13
  vmulpd %xmm4, %xmm6, %xmm9
  vmulpd %xmm5, %xmm6, %xmm11
  vpermpd $64, %ymm9, %ymm14
  vblendpd $12, %ymm12, %ymm6, %ymm6
  vpermpd $85, %ymm12, %ymm7
  vshufpd $11, %ymm7, %ymm6, %ymm6
  vpermpd $64, %ymm11, %ymm15
  vmulpd %ymm3, %ymm6, %ymm8
  vmulpd %ymm4, %ymm6, %ymm7
  vmulpd %ymm5, %ymm12, %ymm6
  vsubpd %ymm8, %ymm13, %ymm12
  vsubpd %ymm7, %ymm14, %ymm13
  vmulpd %ymm13, %ymm13, %ymm13
  vfmadd231pd %ymm12, %ymm12, %ymm13
  vsubpd %ymm6, %ymm15, %ymm12
  vshufpd $3, %xmm10, %xmm10, %xmm10
  vextractf128 $1, %ymm8, %xmm14
  vsubsd %xmm14, %xmm10, %xmm10
  vfmadd231pd %ymm12, %ymm12, %ymm13
  vshufpd $3, %xmm9, %xmm9, %xmm9
  vshufpd $3, %xmm11, %xmm11, %xmm11
  vextractf128 $1, %ymm6, %xmm12
  vaddsd %xmm1, %xmm13, %xmm14
  vsubsd %xmm12, %xmm11, %xmm11
  vextractf128 $1, %ymm7, %xmm12
  vsubsd %xmm12, %xmm9, %xmm9
  vshufpd $1, %xmm13, %xmm13, %xmm12
  vextractf128 $1, %ymm13, %xmm13
  vmulsd %xmm9, %xmm9, %xmm9
  vfmadd231sd %xmm10, %xmm10, %xmm9
  vaddsd %xmm14, %xmm12, %xmm10
  vfmadd231sd %xmm11, %xmm11, %xmm9
  vpermpd $170, %ymm8, %ymm11
  vsubpd %xmm11, %xmm8, %xmm8
  vshufpd $1, %xmm13, %xmm13, %xmm11
  vshufpd $1, %xmm8, %xmm8, %xmm8
  vpermpd $170, %ymm7, %ymm12
  vsubpd %xmm12, %xmm7, %xmm7
  vaddsd %xmm10, %xmm13, %xmm10
  vpermpd $170, %ymm6, %ymm12
  vsubpd %xmm12, %xmm6, %xmm6
  vshufpd $1, %xmm6, %xmm6, %xmm6
  vaddsd %xmm10, %xmm11, %xmm10
  vmulpd %xmm7, %xmm7, %xmm7
  vshufpd $1, %xmm7, %xmm7, %xmm7
  vfmadd231sd %xmm8, %xmm8, %xmm7
  vaddsd %xmm10, %xmm9, %xmm8
  vfmadd231sd %xmm6, %xmm6, %xmm7
  vaddsd %xmm7, %xmm8, %xmm6
  vaddsd %xmm6, %xmm0, %xmm0
  cmpl $2000000, %eax
  jne .LBB0_1
  pushq %rax
  leaq .L.str(%rip), %rdi
  movb $1, %al
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

