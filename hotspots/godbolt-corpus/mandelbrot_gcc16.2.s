.LC5:
  .string "mandelbrot total iterations: %d\n"
main:
  subq $8, %rsp
  vmovsd .LC1(%rip), %xmm7
  xorl %ecx, %ecx
  xorl %esi, %esi
  vmovsd .LC3(%rip), %xmm9
  vmovsd .LC4(%rip), %xmm6
  vxorps %xmm8, %xmm8, %xmm8
.L5:
  vcvtsi2sdl %ecx, %xmm8, %xmm0
  vaddsd %xmm0, %xmm0, %xmm0
  xorl %edx, %edx
  vdivsd %xmm7, %xmm0, %xmm5
  vsubsd .LC2(%rip), %xmm5, %xmm5
.L4:
  vcvtsi2sdl %edx, %xmm8, %xmm4
  vaddsd %xmm4, %xmm4, %xmm4
  vxorpd %xmm3, %xmm3, %xmm3
  xorl %eax, %eax
  vmovapd %xmm3, %xmm2
  vmovapd %xmm3, %xmm1
  vdivsd %xmm7, %xmm4, %xmm4
  vsubsd %xmm9, %xmm4, %xmm4
  jmp .L3
.L11:
  addl $1, %eax
  cmpl $50, %eax
  je .L2
.L3:
  vmovapd %xmm2, %xmm0
  vfmsub231sd %xmm2, %xmm2, %xmm1
  vaddsd %xmm0, %xmm0, %xmm0
  vfmadd132sd %xmm0, %xmm5, %xmm3
  vaddsd %xmm4, %xmm1, %xmm2
  vmovapd %xmm2, %xmm0
  vmulsd %xmm3, %xmm3, %xmm1
  vfmadd132sd %xmm2, %xmm1, %xmm0
  vcomisd %xmm6, %xmm0
  jbe .L11
.L2:
  addl $1, %edx
  addl %eax, %esi
  cmpl $4000, %edx
  jne .L4
  addl $1, %ecx
  cmpl $4000, %ecx
  jne .L5
  movl $.LC5, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
.LC1:
  .long 0
  .long 1085227008
.LC2:
  .long 0
  .long 1072693248
.LC3:
  .long 0
  .long 1073217536
.LC4:
  .long 0
  .long 1074790400
