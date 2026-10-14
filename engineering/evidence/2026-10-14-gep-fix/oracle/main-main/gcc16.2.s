main:
  pushq %rbp
  movl $8, %edx
  vpxor %xmm9, %xmm9, %xmm9
  vpcmpeqd %ymm4, %ymm4, %ymm4
  vmovd %edx, %xmm8
  movl $16, %edx
  movl $guard, %eax
  vmovd %edx, %xmm7
  movl $24, %edx
  vpbroadcastd %xmm8, %ymm8
  vmovd %edx, %xmm6
  movq %rsp, %rbp
  movl $32, %edx
  andq $-32, %rsp
  subq $32, %rsp
  vmovd %edx, %xmm5
  vmovdqa %ymm9, %ymm3
  vmovdqa .LC0(%rip), %ymm2
  vpbroadcastd %xmm7, %ymm7
  vpbroadcastd %xmm6, %ymm6
  vpbroadcastd %xmm5, %ymm5
  vpsrlw $8, %ymm4, %ymm4
.L2:
  vpaddd %ymm8, %ymm2, %ymm1
  vpblendw $170, %ymm3, %ymm2, %ymm0
  vpaddd %ymm6, %ymm2, %ymm10
  addq $32, %rax
  vpblendw $170, %ymm3, %ymm1, %ymm1
  vpblendw $170, %ymm3, %ymm10, %ymm10
  vpackusdw %ymm1, %ymm0, %ymm0
  vpaddd %ymm7, %ymm2, %ymm1
  vpaddd %ymm5, %ymm2, %ymm2
  vpblendw $170, %ymm3, %ymm1, %ymm1
  vpermq $216, %ymm0, %ymm0
  vpackusdw %ymm10, %ymm1, %ymm1
  vpand %ymm0, %ymm4, %ymm0
  vpermq $216, %ymm1, %ymm1
  vpand %ymm1, %ymm4, %ymm1
  vpackuswb %ymm1, %ymm0, %ymm0
  vpermq $216, %ymm0, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $guard+256, %rax
  jne .L2
  movl $524296, %edx
  vmovdqa %ymm9, (%rsp)
  movl $tree, %eax
  vmovd %edx, %xmm0
  vpbroadcastd %xmm0, %ymm0
.L4:
  vmovdqa %ymm0, (%rax)
  addq $96, %rax
  vmovdqa %ymm0, -64(%rax)
  vmovdqa %ymm0, -32(%rax)
  cmpq $tree+288, %rax
  jne .L4
  movw $144, 16(%rsp)
  movq %rsp, %rdi
  movl $tree, %edx
  movl $guard, %esi
  vzeroupper
  call check
  leave
  ret
.LC0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
