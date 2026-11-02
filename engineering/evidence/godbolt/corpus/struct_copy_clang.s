.LCPI0_0:
  .quad 0x3fb999999999999a
.LCPI0_1:
  .quad 0x3fc999999999999a
.LCPI0_2:
  .quad 0x3fd3333333333333
.LCPI0_4:
  .quad 0x3ff0000000000000
.LCPI0_3:
  .long 1
  .long 2
  .zero 4
  .zero 4
.LCPI0_5:
  .quad 0x3fb999999999999a
  .quad 0x3fb999999999999a
.LCPI0_6:
  .quad 0x3fc999999999999a
  .quad 0x3fc999999999999a
.LCPI0_7:
  .quad 0x3fd3333333333333
  .quad 0x3fd3333333333333
main:
  xorl %eax, %eax
  vxorpd %xmm0, %xmm0, %xmm0
  vmovsd .LCPI0_1(%rip), %xmm2
  vmovsd .LCPI0_2(%rip), %xmm3
  vxorpd %xmm9, %xmm9, %xmm9
.LBB0_1:
  vmovsd .LCPI0_0(%rip), %xmm1
  vmulsd %xmm1, %xmm9, %xmm4
  vmulsd %xmm2, %xmm9, %xmm5
  vmulsd %xmm3, %xmm9, %xmm6
  vmovd %eax, %xmm10
  vpbroadcastd %xmm10, %xmm10
  vpaddd .LCPI0_3(%rip), %xmm10, %xmm13
  leal 3(%rax), %ecx
  vxorps %xmm11, %xmm11, %xmm11
  vcvtsi2sd %ecx, %xmm11, %xmm12
  vmulsd %xmm1, %xmm12, %xmm10
  vmulsd %xmm2, %xmm12, %xmm11
  vmulsd %xmm3, %xmm12, %xmm12
  vcvtdq2pd %xmm13, %xmm13
  vmulpd .LCPI0_5(%rip), %xmm13, %xmm14
  vmulpd .LCPI0_6(%rip), %xmm13, %xmm15
  vmulpd .LCPI0_7(%rip), %xmm13, %xmm13
  vmovddup %xmm4, %xmm7
  vsubpd %xmm14, %xmm7, %xmm7
  vmovddup %xmm5, %xmm8
  vsubpd %xmm15, %xmm8, %xmm8
  vmovddup %xmm6, %xmm1
  vsubpd %xmm13, %xmm1, %xmm1
  vmulpd %xmm8, %xmm8, %xmm8
  vfmadd231pd %xmm7, %xmm7, %xmm8
  vfmadd231pd %xmm1, %xmm1, %xmm8
  vshufpd $1, %xmm8, %xmm8, %xmm1
  vaddsd %xmm1, %xmm8, %xmm1
  vsubsd %xmm10, %xmm4, %xmm4
  vsubsd %xmm11, %xmm5, %xmm5
  vsubsd %xmm12, %xmm6, %xmm6
  vmulsd %xmm5, %xmm5, %xmm5
  vfmadd231sd %xmm4, %xmm4, %xmm5
  vfmadd231sd %xmm6, %xmm6, %xmm5
  vaddsd %xmm1, %xmm5, %xmm1
  vshufpd $1, %xmm14, %xmm14, %xmm4
  vsubsd %xmm4, %xmm14, %xmm5
  vshufpd $1, %xmm15, %xmm15, %xmm6
  vsubsd %xmm6, %xmm15, %xmm7
  vshufpd $1, %xmm13, %xmm13, %xmm8
  vmulsd %xmm7, %xmm7, %xmm7
  vfmadd231sd %xmm5, %xmm5, %xmm7
  vsubsd %xmm8, %xmm13, %xmm5
  vfmadd231sd %xmm5, %xmm5, %xmm7
  vaddsd %xmm1, %xmm7, %xmm1
  vsubsd %xmm10, %xmm14, %xmm5
  vsubsd %xmm11, %xmm15, %xmm7
  vmulsd %xmm7, %xmm7, %xmm7
  vfmadd231sd %xmm5, %xmm5, %xmm7
  vsubsd %xmm12, %xmm13, %xmm5
  vfmadd231sd %xmm5, %xmm5, %xmm7
  vaddsd %xmm1, %xmm7, %xmm1
  vsubsd %xmm10, %xmm4, %xmm4
  vsubsd %xmm11, %xmm6, %xmm5
  vsubsd %xmm12, %xmm8, %xmm6
  vmulsd %xmm5, %xmm5, %xmm5
  vfmadd231sd %xmm4, %xmm4, %xmm5
  vfmadd231sd %xmm6, %xmm6, %xmm5
  vaddsd %xmm1, %xmm5, %xmm1
  vaddsd %xmm1, %xmm0, %xmm0
  vaddsd .LCPI0_4(%rip), %xmm9, %xmm9
  incl %eax
  cmpl $2000000, %eax
  jne .LBB0_1
  pushq %rax
  leaq .L.str(%rip), %rdi
  movb $1, %al
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "struct_copy total: %.2f\n"

