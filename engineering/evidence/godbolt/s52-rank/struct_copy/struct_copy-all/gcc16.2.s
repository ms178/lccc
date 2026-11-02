.LC3:
main:
  subq $248, %rsp
  vxorps %xmm13, %xmm13, %xmm13
  vxorpd %xmm7, %xmm7, %xmm7
  xorl %edx, %edx
  vmovapd .LC1(%rip), %xmm12
  vmovsd .LC2(%rip), %xmm15
  movq $0x000000000, 8(%rsp)
.L2:
  leal 2(%rdx), %eax
  leal 3(%rdx), %ecx
  vmulsd %xmm15, %xmm7, %xmm1
  addl $1, %edx
  vmovddup %xmm7, %xmm2
  vcvtsi2sdl %eax, %xmm13, %xmm14
  vcvtsi2sdl %edx, %xmm13, %xmm7
  vmovddup %xmm14, %xmm0
  vmulpd %xmm12, %xmm0, %xmm0
  vmovddup %xmm7, %xmm8
  vcvtsi2sdl %ecx, %xmm13, %xmm6
  vmovddup %xmm6, %xmm3
  vmulpd %xmm12, %xmm2, %xmm2
  vmulpd %xmm12, %xmm8, %xmm8
  vmulsd %xmm15, %xmm7, %xmm5
  vmovddup %xmm1, %xmm4
  vmovsd %xmm1, (%rsp)
  vmovapd %xmm4, 16(%rsp)
  vmulsd %xmm15, %xmm14, %xmm14
  vmulpd %xmm12, %xmm3, %xmm3
  vmulsd %xmm15, %xmm6, %xmm6
  vpermilpd $0, %xmm2, %xmm4
  vpermilpd $3, %xmm2, %xmm11
  vunpcklpd %xmm0, %xmm8, %xmm9
  vsubpd %xmm9, %xmm4, %xmm4
  vunpckhpd %xmm0, %xmm8, %xmm9
  vmovsd %xmm5, 96(%rsp)
  vmovapd 96(%rsp), %xmm1
  vsubpd %xmm9, %xmm11, %xmm9
  vmovddup %xmm5, %xmm5
  vmovsd %xmm14, 144(%rsp)
  vmovhpd 144(%rsp), %xmm1, %xmm11
  vmovapd 16(%rsp), %xmm1
  vunpckhpd %xmm3, %xmm3, %xmm10
  vmovsd %xmm6, 192(%rsp)
  vsubsd %xmm6, %xmm14, %xmm14
  vmulpd %xmm9, %xmm9, %xmm9
  vsubpd %xmm11, %xmm1, %xmm11
  vxorpd %xmm1, %xmm1, %xmm1
  vfmadd132pd %xmm4, %xmm9, %xmm4
  vfmadd231pd %xmm11, %xmm11, %xmm4
  vmovapd %xmm3, %xmm11
  vaddsd %xmm1, %xmm4, %xmm9
  vunpckhpd %xmm4, %xmm4, %xmm4
  vmovsd (%rsp), %xmm1
  vsubsd %xmm6, %xmm1, %xmm1
  vaddsd %xmm4, %xmm9, %xmm9
  vsubsd %xmm3, %xmm2, %xmm4
  vunpckhpd %xmm2, %xmm2, %xmm2
  vsubsd %xmm10, %xmm2, %xmm2
  vmulsd %xmm2, %xmm2, %xmm2
  vfmadd132sd %xmm4, %xmm2, %xmm4
  vunpcklpd %xmm3, %xmm0, %xmm2
  vunpckhpd %xmm3, %xmm0, %xmm3
  vfmadd132sd %xmm1, %xmm4, %xmm1
  vpermilpd $0, %xmm8, %xmm4
  vpermilpd $3, %xmm8, %xmm8
  vsubpd %xmm3, %xmm8, %xmm3
  vsubpd %xmm2, %xmm4, %xmm2
  vmovapd 144(%rsp), %xmm4
  vmovhpd 192(%rsp), %xmm4, %xmm4
  vmulpd %xmm3, %xmm3, %xmm3
  vsubpd %xmm4, %xmm5, %xmm5
  vaddsd %xmm9, %xmm1, %xmm1
  vfmadd132pd %xmm2, %xmm3, %xmm2
  vsubsd %xmm11, %xmm0, %xmm3
  vunpckhpd %xmm0, %xmm0, %xmm0
  vsubsd %xmm10, %xmm0, %xmm0
  vmulsd %xmm0, %xmm0, %xmm0
  vfmadd132pd %xmm5, %xmm2, %xmm5
  vfmadd132sd %xmm3, %xmm0, %xmm3
  vaddsd %xmm5, %xmm1, %xmm1
  vunpckhpd %xmm5, %xmm5, %xmm5
  vaddsd %xmm5, %xmm1, %xmm1
  vfmadd132sd %xmm14, %xmm3, %xmm14
  vaddsd %xmm1, %xmm14, %xmm14
  vaddsd 8(%rsp), %xmm14, %xmm6
  vmovsd %xmm6, 8(%rsp)
  cmpl $2000000, %edx
  jne .L2
  vmovapd %xmm6, %xmm0
  movl $.LC3, %edi
  movl $1, %eax
  call printf
  xorl %eax, %eax
  addq $248, %rsp
  ret
.LC1:
.LC2:
