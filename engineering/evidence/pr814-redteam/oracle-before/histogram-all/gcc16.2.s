main:
  movl $-1640531535, %ecx
  vpcmpeqd %ymm4, %ymm4, %ymm4
  vpxor %xmm3, %xmm3, %xmm3
  movl $bytes, %edx
  vmovd %ecx, %xmm2
  movl $8, %ecx
  movq %rdx, %rax
  vmovdqa .LC0(%rip), %ymm1
  vmovd %ecx, %xmm8
  movl $16, %ecx
  vpbroadcastd %xmm2, %ymm2
  vmovd %ecx, %xmm7
  movl $24, %ecx
  vpbroadcastd %xmm8, %ymm8
  vmovd %ecx, %xmm6
  movl $32, %ecx
  vpbroadcastd %xmm7, %ymm7
  vmovd %ecx, %xmm5
  vpsrlw $8, %ymm4, %ymm4
  vpbroadcastd %xmm6, %ymm6
  vpbroadcastd %xmm5, %ymm5
.L2:
  vpaddd %ymm8, %ymm1, %ymm0
  vpmulld %ymm2, %ymm1, %ymm9
  vpaddd %ymm6, %ymm1, %ymm10
  addq $32, %rax
  vpmulld %ymm2, %ymm0, %ymm0
  vpmulld %ymm2, %ymm10, %ymm10
  vpsrld $24, %ymm9, %ymm9
  vpsrld $24, %ymm0, %ymm0
  vpsrld $24, %ymm10, %ymm10
  vpblendw $170, %ymm3, %ymm9, %ymm9
  vpblendw $170, %ymm3, %ymm0, %ymm0
  vpblendw $170, %ymm3, %ymm10, %ymm10
  vpackusdw %ymm0, %ymm9, %ymm9
  vpaddd %ymm7, %ymm1, %ymm0
  vpaddd %ymm5, %ymm1, %ymm1
  vpmulld %ymm2, %ymm0, %ymm0
  vpermq $216, %ymm9, %ymm9
  vpand %ymm9, %ymm4, %ymm9
  vpsrld $24, %ymm0, %ymm0
  vpblendw $170, %ymm3, %ymm0, %ymm0
  vpackusdw %ymm10, %ymm0, %ymm0
  vpermq $216, %ymm0, %ymm0
  vpand %ymm0, %ymm4, %ymm0
  vpackuswb %ymm0, %ymm9, %ymm0
  vpermq $216, %ymm0, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $bytes+262144, %rax
  jne .L2
.L3:
  movzbl (%rdx), %eax
  addq $1, %rdx
  addq $1, bins(,%rax,8)
  cmpq $bytes+262144, %rdx
  jne .L3
  movl $bins, %edx
  movl $bins+2048, %edi
  xorl %eax, %eax
  xorl %esi, %esi
.L4:
  imulq $131, %rax, %rax
  movq (%rdx), %rcx
  addq $8, %rdx
  addq %rcx, %rsi
  addq %rcx, %rax
  cmpq %rdx, %rdi
  jne .L4
  movl $1, %edx
  cmpq $262144, %rsi
  jne .L1
  movabsq $-7388273516099151790, %rdx
  cmpq %rdx, %rax
  setne %dl
  movzbl %dl, %edx
  addl %edx, %edx
.L1:
  movl %edx, %eax
  vzeroupper
  ret
.LC0:
