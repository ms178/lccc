.LC7:
  .string "struct_copy total: %.2f\n"
main:
  subq $216, %rsp
  vmovdqa .LC1(%rip), %xmm11
  vmovdqa .LC2(%rip), %xmm10
  xorl %edi, %edi
  vmovdqa .LC3(%rip), %xmm9
  vmovapd .LC4(%rip), %xmm8
  vxorpd %xmm5, %xmm5, %xmm5
  vmovapd .LC5(%rip), %xmm7
  vmovapd .LC6(%rip), %xmm6
.L2:
  vmovd %edi, %xmm2
  movq %rsp, %rax
  movl $3, %ecx
  movl $1, %esi
  vpbroadcastd %xmm2, %xmm0
  vpaddd %xmm11, %xmm0, %xmm2
  vpaddd %xmm10, %xmm0, %xmm1
  vpaddd %xmm9, %xmm0, %xmm0
  vpshufd $238, %xmm2, %xmm4
  vcvtdq2pd %xmm1, %xmm12
  vpshufd $238, %xmm1, %xmm1
  vcvtdq2pd %xmm0, %xmm3
  vmulpd %xmm7, %xmm12, %xmm12
  vcvtdq2pd %xmm4, %xmm4
  vcvtdq2pd %xmm2, %xmm2
  vmulpd %xmm8, %xmm4, %xmm4
  vpshufd $238, %xmm0, %xmm0
  vcvtdq2pd %xmm1, %xmm1
  vmulpd %xmm8, %xmm3, %xmm3
  vcvtdq2pd %xmm0, %xmm0
  vmulpd %xmm6, %xmm2, %xmm2
  vmulpd %xmm7, %xmm0, %xmm0
  vmulpd %xmm6, %xmm1, %xmm1
  vmovhpd %xmm12, 64(%rsp)
  vmovlpd %xmm4, 16(%rsp)
  vshufpd $1, %xmm12, %xmm4, %xmm4
  vxorpd %xmm12, %xmm12, %xmm12
  vmovlpd %xmm3, 112(%rsp)
  vmovapd %xmm2, (%rsp)
  vshufpd $1, %xmm0, %xmm3, %xmm3
  vmovapd %xmm4, 48(%rsp)
  vmovapd %xmm1, 96(%rsp)
  vmovapd %xmm3, 144(%rsp)
  vmovhpd %xmm0, 160(%rsp)
.L8:
  vmovsd (%rax), %xmm3
  vmovsd 8(%rax), %xmm4
  vmovsd 16(%rax), %xmm2
  cmpl $3, %esi
  je .L9
  vmovapd 48(%rax), %xmm15
  vmovddup %xmm3, %xmm1
  vmovddup %xmm2, %xmm0
  leal 2(%rsi), %edx
  vmovhpd 96(%rax), %xmm15, %xmm13
  vmovapd 96(%rax), %xmm15
  vmovlpd 56(%rax), %xmm15, %xmm14
  vsubpd %xmm13, %xmm1, %xmm1
  vmovddup %xmm4, %xmm13
  vsubpd %xmm14, %xmm13, %xmm13
  vmovapd 64(%rax), %xmm14
  vmovhpd 112(%rax), %xmm14, %xmm14
  vmulpd %xmm13, %xmm13, %xmm13
  vsubpd %xmm14, %xmm0, %xmm0
  vfmadd132pd %xmm1, %xmm13, %xmm1
  vfmadd132pd %xmm0, %xmm1, %xmm0
  vaddsd %xmm0, %xmm12, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm0
  vaddsd %xmm0, %xmm1, %xmm12
  cmpl $2, %ecx
  je .L12
.L3:
  leaq (%rdx,%rdx,2), %rdx
  salq $4, %rdx
  vsubsd 8(%rsp,%rdx), %xmm4, %xmm4
  vsubsd (%rsp,%rdx), %xmm3, %xmm3
  vsubsd 16(%rsp,%rdx), %xmm2, %xmm2
  vmulsd %xmm4, %xmm4, %xmm4
  vfmadd132sd %xmm3, %xmm4, %xmm3
  vfmadd132sd %xmm2, %xmm3, %xmm2
  vaddsd %xmm2, %xmm12, %xmm12
  subl $1, %ecx
  je .L5
  addl $1, %esi
  addq $48, %rax
  jmp .L8
.L5:
  addl $1, %edi
  vaddsd %xmm12, %xmm5, %xmm5
  cmpl $2000000, %edi
  jne .L2
  vmovapd %xmm5, %xmm0
  movl $.LC7, %edi
  movl $1, %eax
  call printf
  xorl %eax, %eax
  addq $216, %rsp
  ret
.L12:
  addl $1, %esi
  addq $48, %rax
  movl $1, %ecx
  jmp .L8
.L9:
  movl $3, %edx
  jmp .L3
.LC1:
  .long 0
  .long 0
  .long 0
  .long 1
.LC2:
  .long 1
  .long 1
  .long 2
  .long 2
.LC3:
  .long 2
  .long 3
  .long 3
  .long 3
.LC4:
  .long 858993459
  .long 1070805811
  .long -1717986918
  .long 1069128089
.LC5:
  .long -1717986918
  .long 1070176665
  .long 858993459
  .long 1070805811
.LC6:
  .long -1717986918
  .long 1069128089
  .long -1717986918
  .long 1070176665
