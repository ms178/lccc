main:
  movl $1103515245, %edx
  pushq %rbp
  vpcmpeqd %ymm14, %ymm14, %ymm14
  movl $data, %eax
  vmovd %edx, %xmm12
  movl $12345, %edx
  vpxor %xmm13, %xmm13, %xmm13
  vmovd %edx, %xmm11
  movl $8, %edx
  vpbroadcastd %xmm12, %ymm12
  vmovd %edx, %xmm15
  movl $16, %edx
  movq %rsp, %rbp
  andq $-32, %rsp
  vmovd %edx, %xmm6
  subq $8, %rsp
  movl $24, %edx
  vpbroadcastd %xmm6, %ymm6
  vpbroadcastd %xmm11, %ymm11
  vpbroadcastd %xmm15, %ymm15
  vmovdqa %ymm6, -24(%rsp)
  vmovd %edx, %xmm6
  movl $1491936009, %edx
  vpbroadcastd %xmm6, %ymm6
  vmovdqa .LC0(%rip), %ymm10
  vpsrlw $8, %ymm14, %ymm14
  vmovdqa %ymm6, -56(%rsp)
  vmovd %edx, %xmm6
  movl $538976288, %edx
  vmovd %edx, %xmm7
  movl $32, %edx
  vpbroadcastd %xmm6, %ymm6
  vpbroadcastd %xmm7, %ymm7
  vmovdqa %ymm7, -88(%rsp)
  vmovd %edx, %xmm7
  vpbroadcastd %xmm7, %ymm7
  vmovdqa %ymm7, -120(%rsp)
.L2:
  vpmulld %ymm12, %ymm10, %ymm0
  vpaddd %ymm15, %ymm10, %ymm1
  vpaddd -24(%rsp), %ymm10, %ymm3
  addq $32, %rax
  vpmulld %ymm12, %ymm1, %ymm1
  vpaddd -56(%rsp), %ymm10, %ymm2
  vpaddd -120(%rsp), %ymm10, %ymm10
  vpmulld %ymm12, %ymm3, %ymm3
  vpmulld %ymm12, %ymm2, %ymm2
  vpaddd %ymm11, %ymm0, %ymm0
  vpsrld $16, %ymm0, %ymm0
  vpaddd %ymm11, %ymm1, %ymm1
  vpsrld $16, %ymm1, %ymm8
  vpmuludq %ymm6, %ymm0, %ymm4
  vpaddd %ymm11, %ymm3, %ymm3
  vpsrlq $32, %ymm0, %ymm1
  vpsrlq $32, %ymm8, %ymm9
  vpaddd %ymm11, %ymm2, %ymm2
  vpmuludq %ymm6, %ymm1, %ymm1
  vpmuludq %ymm6, %ymm9, %ymm9
  vpsrld $16, %ymm3, %ymm3
  vpsrld $16, %ymm2, %ymm2
  vpsrlq $32, %ymm3, %ymm7
  vpsrlq $32, %ymm2, %ymm5
  vpmuludq %ymm6, %ymm7, %ymm7
  vpmuludq %ymm6, %ymm5, %ymm5
  vpshufd $245, %ymm4, %ymm4
  vpblendd $85, %ymm4, %ymm1, %ymm1
  vpmuludq %ymm6, %ymm8, %ymm4
  vpshufd $245, %ymm4, %ymm4
  vpblendd $85, %ymm4, %ymm9, %ymm9
  vpmuludq %ymm6, %ymm3, %ymm4
  vpshufd $245, %ymm4, %ymm4
  vpblendd $85, %ymm4, %ymm7, %ymm7
  vpmuludq %ymm6, %ymm2, %ymm4
  vpshufd $245, %ymm4, %ymm4
  vpblendd $85, %ymm4, %ymm5, %ymm5
  vpsubd %ymm1, %ymm0, %ymm4
  vpsrld $1, %ymm4, %ymm4
  vpaddd %ymm1, %ymm4, %ymm4
  vpsrld $6, %ymm4, %ymm4
  vpslld $1, %ymm4, %ymm1
  vpaddd %ymm4, %ymm1, %ymm1
  vpslld $5, %ymm1, %ymm1
  vpsubd %ymm4, %ymm1, %ymm1
  vpsubd %ymm1, %ymm0, %ymm0
  vpsubd %ymm9, %ymm8, %ymm1
  vpsrld $1, %ymm1, %ymm1
  vpblendw $170, %ymm13, %ymm0, %ymm0
  vpaddd %ymm9, %ymm1, %ymm1
  vpsrld $6, %ymm1, %ymm1
  vpslld $1, %ymm1, %ymm4
  vpaddd %ymm1, %ymm4, %ymm4
  vpslld $5, %ymm4, %ymm4
  vpsubd %ymm1, %ymm4, %ymm1
  vpsubd %ymm7, %ymm3, %ymm4
  vpsrld $1, %ymm4, %ymm4
  vpsubd %ymm1, %ymm8, %ymm1
  vpaddd %ymm7, %ymm4, %ymm4
  vpblendw $170, %ymm13, %ymm1, %ymm1
  vpsrld $6, %ymm4, %ymm4
  vpackusdw %ymm1, %ymm0, %ymm0
  vpslld $1, %ymm4, %ymm1
  vpermq $216, %ymm0, %ymm0
  vpaddd %ymm4, %ymm1, %ymm1
  vpand %ymm0, %ymm14, %ymm0
  vpslld $5, %ymm1, %ymm1
  vpsubd %ymm4, %ymm1, %ymm1
  vpsubd %ymm1, %ymm3, %ymm1
  vpsubd %ymm5, %ymm2, %ymm3
  vpsrld $1, %ymm3, %ymm3
  vpblendw $170, %ymm13, %ymm1, %ymm1
  vpaddd %ymm5, %ymm3, %ymm3
  vpsrld $6, %ymm3, %ymm3
  vpslld $1, %ymm3, %ymm4
  vpaddd %ymm3, %ymm4, %ymm4
  vpslld $5, %ymm4, %ymm4
  vpsubd %ymm3, %ymm4, %ymm3
  vpsubd %ymm3, %ymm2, %ymm2
  vpblendw $170, %ymm13, %ymm2, %ymm2
  vpackusdw %ymm2, %ymm1, %ymm1
  vpermq $216, %ymm1, %ymm1
  vpand %ymm1, %ymm14, %ymm1
  vpackuswb %ymm1, %ymm0, %ymm0
  vpermq $216, %ymm0, %ymm0
  vpaddb -88(%rsp), %ymm0, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $data+65536, %rax
  jne .L2
  xorl %eax, %eax
  xorl %esi, %esi
.L4:
  movzbl data(%rsi), %ecx
  leal -65(%rcx), %edi
  movl %ecx, %edx
  leal 32(%rcx), %r8d
  cmpl $25, %edi
  cmovbe %r8d, %ecx
  leal 32(%rdx), %r8d
  cmovbe %r8d, %edx
  addq $1, %rsi
  movb %dl, folded-1(%rsi)
  movl %eax, %edx
  sall $5, %edx
  addl %edx, %eax
  addl %ecx, %eax
  cmpq $65536, %rsi
  jne .L4
  cmpb $32, folded(%rip)
  jne .L5
  cmpb $55, folded+1(%rip)
  je .L6
.L5:
  movl $1, %eax
.L1:
  vzeroupper
  leave
  ret
.L6:
  cmpb $110, folded+2(%rip)
  jne .L5
  cmpl $-1723667183, %eax
  setne %al
  movzbl %al, %eax
  addl %eax, %eax
  jmp .L1
.LC0:
