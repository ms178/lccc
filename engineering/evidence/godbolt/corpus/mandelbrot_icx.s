.LCPI0_0:
  .quad 0x3f40624dd2f1a9fc
.LCPI0_1:
  .quad 0xbff0000000000000
.LCPI0_2:
  .quad 0xbff8000000000000
.LCPI0_3:
  .quad 0x4010000000000000
.LCPI0_4:
  .quad 0x3ff0000000000000
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  xorl %eax, %eax
  vxorpd %xmm0, %xmm0, %xmm0
  vmovsd .LCPI0_0(%rip), %xmm1
  vmovsd .LCPI0_1(%rip), %xmm2
  vmovsd .LCPI0_2(%rip), %xmm3
  vmovsd .LCPI0_3(%rip), %xmm4
  vmovsd .LCPI0_4(%rip), %xmm5
  xorl %esi, %esi
  jmp .LBB0_1
.LBB0_7:
  vaddsd %xmm5, %xmm0, %xmm0
  leal 1(%rax), %ecx
  cmpl $3999, %eax
  movl %ecx, %eax
  je .LBB0_8
.LBB0_1:
  vmovapd %xmm1, %xmm6
  vfmadd213sd %xmm2, %xmm0, %xmm6
  vxorpd %xmm7, %xmm7, %xmm7
  xorl %ecx, %ecx
  jmp .LBB0_2
.LBB0_5:
  incl %edx
.LBB0_6:
  addl %edx, %esi
  vaddsd %xmm5, %xmm7, %xmm7
  leal 1(%rcx), %edx
  cmpl $3999, %ecx
  movl %edx, %ecx
  je .LBB0_7
.LBB0_2:
  vmovapd %xmm1, %xmm8
  vfmadd213sd %xmm3, %xmm7, %xmm8
  vxorpd %xmm9, %xmm9, %xmm9
  xorl %edx, %edx
  vxorpd %xmm10, %xmm10, %xmm10
.LBB0_3:
  vmovapd %xmm10, %xmm11
  vaddsd %xmm9, %xmm9, %xmm10
  vfmadd213sd %xmm6, %xmm11, %xmm10
  vfmsub213sd %xmm8, %xmm11, %xmm11
  vfmsub231sd %xmm9, %xmm9, %xmm11
  vmulsd %xmm11, %xmm11, %xmm9
  vmulsd %xmm10, %xmm10, %xmm12
  vaddsd %xmm12, %xmm9, %xmm13
  vucomisd %xmm4, %xmm13
  ja .LBB0_6
  vsubsd %xmm12, %xmm9, %xmm9
  vaddsd %xmm9, %xmm8, %xmm9
  vaddsd %xmm11, %xmm11, %xmm11
  vfmadd213sd %xmm6, %xmm11, %xmm10
  vmulsd %xmm10, %xmm10, %xmm11
  vfmadd231sd %xmm9, %xmm9, %xmm11
  vucomisd %xmm4, %xmm11
  ja .LBB0_5
  addl $2, %edx
  cmpl $50, %edx
  jne .LBB0_3
  movl $50, %edx
  jmp .LBB0_6
.LBB0_8:
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "mandelbrot total iterations: %d\n"

