.LCPI0_0:
  .quad 0x40af400000000000
.LCPI0_1:
  .quad 0xbff0000000000000
.LCPI0_2:
  .quad 0xbff8000000000000
.LCPI0_3:
  .quad 0x4010000000000000
main:
  xorl %esi, %esi
  vmovsd .LCPI0_0(%rip), %xmm0
  vmovsd .LCPI0_1(%rip), %xmm1
  vmovsd .LCPI0_2(%rip), %xmm2
  vmovsd .LCPI0_3(%rip), %xmm3
  xorl %eax, %eax
  jmp .LBB0_1
.LBB0_7:
  incl %eax
  cmpl $4000, %eax
  je .LBB0_8
.LBB0_1:
  leal (%rax,%rax), %ecx
  vcvtsi2sd %ecx, %xmm15, %xmm4
  vdivsd %xmm0, %xmm4, %xmm4
  vaddsd %xmm1, %xmm4, %xmm4
  xorl %ecx, %ecx
  jmp .LBB0_2
.LBB0_9:
  incl %edx
.LBB0_10:
  addl %edx, %esi
  incl %ecx
  cmpl $4000, %ecx
  je .LBB0_7
.LBB0_2:
  leal (%rcx,%rcx), %edx
  vcvtsi2sd %edx, %xmm15, %xmm5
  vdivsd %xmm0, %xmm5, %xmm5
  vaddsd %xmm2, %xmm5, %xmm5
  vxorpd %xmm6, %xmm6, %xmm6
  xorl %edx, %edx
  vxorpd %xmm7, %xmm7, %xmm7
.LBB0_3:
  vmovapd %xmm6, %xmm9
  vmulsd %xmm6, %xmm6, %xmm6
  vfmsub231sd %xmm7, %xmm7, %xmm6
  vaddsd %xmm6, %xmm5, %xmm8
  vaddsd %xmm7, %xmm7, %xmm6
  vfmadd213sd %xmm4, %xmm9, %xmm6
  vmulsd %xmm6, %xmm6, %xmm7
  vmovapd %xmm8, %xmm9
  vfmadd213sd %xmm7, %xmm8, %xmm9
  vucomisd %xmm3, %xmm9
  ja .LBB0_10
  vfmsub231sd %xmm8, %xmm8, %xmm7
  vaddsd %xmm7, %xmm5, %xmm7
  vaddsd %xmm8, %xmm8, %xmm8
  vfmadd213sd %xmm4, %xmm8, %xmm6
  vmulsd %xmm6, %xmm6, %xmm8
  vfmadd231sd %xmm7, %xmm7, %xmm8
  vucomisd %xmm3, %xmm8
  ja .LBB0_9
  addl $2, %edx
  cmpl $50, %edx
  jne .LBB0_3
  movl $50, %edx
  jmp .LBB0_10
.LBB0_8:
  pushq %rax
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "mandelbrot total iterations: %d\n"

