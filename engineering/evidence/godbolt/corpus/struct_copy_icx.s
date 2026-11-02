.LCPI0_0:
  .quad 0x3fb999999999999a
.LCPI0_1:
  .quad 0x3fc999999999999a
.LCPI0_2:
  .quad 0x3fd3333333333333
.LCPI0_3:
  .quad 0x3ff0000000000000
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  vxorpd %xmm0, %xmm0, %xmm0
  movl $3, %eax
  vmovsd .LCPI0_0(%rip), %xmm1
  vmovsd .LCPI0_1(%rip), %xmm2
  vmovsd .LCPI0_2(%rip), %xmm3
  vxorpd %xmm5, %xmm5, %xmm5
.LBB0_1:
  leal -2(%rax), %ecx
  vxorps %xmm11, %xmm11, %xmm11
  vcvtsi2sd %ecx, %xmm11, %xmm7
  vmulsd %xmm1, %xmm7, %xmm11
  vmulsd %xmm2, %xmm7, %xmm6
  leal -1(%rax), %ecx
  vxorps %xmm10, %xmm10, %xmm10
  vcvtsi2sd %ecx, %xmm10, %xmm9
  vmulsd %xmm3, %xmm7, %xmm10
  vmulsd %xmm1, %xmm9, %xmm7
  vmulsd %xmm2, %xmm9, %xmm8
  vmulsd %xmm3, %xmm9, %xmm9
  vxorps %xmm12, %xmm12, %xmm12
  vcvtsi2sd %eax, %xmm12, %xmm14
  vmulsd %xmm1, %xmm14, %xmm12
  vmulsd %xmm2, %xmm14, %xmm13
  vmovapd %xmm5, %xmm15
  vfmsub213sd %xmm6, %xmm2, %xmm15
  vfmadd213sd %xmm0, %xmm15, %xmm15
  vmovapd %xmm5, %xmm0
  vfmsub213sd %xmm11, %xmm1, %xmm0
  vfmadd231sd %xmm0, %xmm0, %xmm15
  vmovapd %xmm5, %xmm0
  vfmsub213sd %xmm10, %xmm3, %xmm0
  vfmadd213sd %xmm15, %xmm0, %xmm0
  vmovapd %xmm5, %xmm15
  vfmsub213sd %xmm7, %xmm1, %xmm15
  vfmadd231sd %xmm15, %xmm15, %xmm0
  vmovapd %xmm5, %xmm15
  vfmsub213sd %xmm8, %xmm2, %xmm15
  vfmadd213sd %xmm0, %xmm15, %xmm15
  vmovapd %xmm5, %xmm0
  vfmsub213sd %xmm9, %xmm3, %xmm0
  vfmadd231sd %xmm0, %xmm0, %xmm15
  vmovapd %xmm5, %xmm0
  vfmsub213sd %xmm12, %xmm1, %xmm0
  vfmadd213sd %xmm15, %xmm0, %xmm0
  vmovapd %xmm5, %xmm15
  vfmsub213sd %xmm13, %xmm2, %xmm15
  vfmadd231sd %xmm15, %xmm15, %xmm0
  vsubsd %xmm7, %xmm11, %xmm15
  vfmadd213sd %xmm0, %xmm15, %xmm15
  vaddsd .LCPI0_3(%rip), %xmm5, %xmm4
  vmulsd %xmm3, %xmm14, %xmm14
  vfmsub213sd %xmm14, %xmm3, %xmm5
  vfmadd231sd %xmm5, %xmm5, %xmm15
  vsubsd %xmm8, %xmm6, %xmm0
  vfmadd231sd %xmm0, %xmm0, %xmm15
  vsubsd %xmm9, %xmm10, %xmm0
  vfmadd231sd %xmm0, %xmm0, %xmm15
  vsubsd %xmm12, %xmm11, %xmm5
  vfmadd213sd %xmm15, %xmm5, %xmm5
  vsubsd %xmm13, %xmm6, %xmm0
  vfmadd231sd %xmm0, %xmm0, %xmm5
  vsubsd %xmm14, %xmm10, %xmm0
  vfmadd231sd %xmm0, %xmm0, %xmm5
  vsubsd %xmm12, %xmm7, %xmm0
  vsubsd %xmm13, %xmm8, %xmm6
  vsubsd %xmm14, %xmm9, %xmm7
  vfmadd213sd %xmm5, %xmm0, %xmm0
  vfmadd231sd %xmm6, %xmm6, %xmm0
  vfmadd231sd %xmm7, %xmm7, %xmm0
  incl %eax
  vmovapd %xmm4, %xmm5
  cmpl $2000003, %eax
  jne .LBB0_1
  movl $.L.str, %edi
  movb $1, %al
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "struct_copy total: %.2f\n"

