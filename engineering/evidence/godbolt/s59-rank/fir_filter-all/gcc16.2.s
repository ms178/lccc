.LC8:
main:
  movl $8, %ecx
  pushq %rbp
  vpcmpeqd %ymm4, %ymm4, %ymm4
  movl $a.2, %edx
  vmovd %ecx, %xmm8
  movl $851981, %ecx
  vpxor %xmm3, %xmm3, %xmm3
  movq %rdx, %rax
  vmovd %ecx, %xmm7
  movl $16, %ecx
  vpbroadcastd %xmm8, %ymm8
  vmovdqa .LC0(%rip), %ymm2
  vmovd %ecx, %xmm6
  movq %rsp, %rbp
  vpbroadcastd %xmm7, %ymm7
  andq $-32, %rsp
  vpsrlw $6, %ymm4, %ymm9
  vpsllw $9, %ymm4, %ymm4
  vpbroadcastd %xmm6, %ymm6
.L2:
  vpaddd %ymm8, %ymm2, %ymm0
  vpblendw $170, %ymm3, %ymm2, %ymm1
  addq $32, %rax
  vpblendw $170, %ymm3, %ymm0, %ymm0
  vpaddd %ymm6, %ymm2, %ymm2
  vpackusdw %ymm0, %ymm1, %ymm1
  vpermq $216, %ymm1, %ymm1
  vpsllw $8, %ymm1, %ymm0
  vpaddw %ymm1, %ymm0, %ymm0
  vpaddw %ymm7, %ymm0, %ymm0
  vpand %ymm9, %ymm0, %ymm0
  vpaddw %ymm4, %ymm0, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $a.2+512, %rax
  jne .L2
  vmovdqa .LC6(%rip), %xmm0
  movl $d.0, %ecx
  movl $d.0+1024, %r8d
  movq %rcx, %rsi
  vmovdqa %xmm0, a.2+512(%rip)
  vmovdqa .LC7(%rip), %xmm0
  vmovdqa %xmm0, c.1(%rip)
.L3:
  xorl %eax, %eax
  vpxor %xmm4, %xmm4, %xmm4
  vpxor %xmm3, %xmm3, %xmm3
.L4:
  vpbroadcastw c.1(%rax), %ymm1
  vmovdqu (%rdx,%rax), %ymm2
  addq $2, %rax
  vpmullw %ymm2, %ymm1, %ymm0
  vpmulhw %ymm2, %ymm1, %ymm1
  vpunpcklwd %ymm1, %ymm0, %ymm2
  vpunpckhwd %ymm1, %ymm0, %ymm0
  vperm2i128 $32, %ymm0, %ymm2, %ymm1
  vperm2i128 $49, %ymm0, %ymm2, %ymm2
  vpaddd %ymm1, %ymm3, %ymm3
  vpaddd %ymm2, %ymm4, %ymm4
  cmpq $16, %rax
  jne .L4
  vmovdqa %ymm3, (%rsi)
  addq $64, %rsi
  addq $32, %rdx
  vmovdqa %ymm4, -32(%rsi)
  cmpq %rsi, %r8
  jne .L3
  movabsq $1099511628211, %rdi
  movl $146959, %esi
.L5:
  movl (%rcx), %eax
  addq $4, %rcx
  movzwl %ax, %edx
  shrl $16, %eax
  xorq %rsi, %rdx
  imulq %rdi, %rdx
  xorq %rdx, %rax
  imulq %rdi, %rax
  movq %rax, %rsi
  cmpq %rcx, %r8
  jne .L5
  xorl %eax, %eax
  movl $.LC8, %edi
  vzeroupper
  call printf
  xorl %eax, %eax
  leave
  ret
.LC0:
.LC6:
.LC7:
