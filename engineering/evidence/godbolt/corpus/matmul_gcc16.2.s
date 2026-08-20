matmul:
  xorl %edi, %edi
.L2:
  leaq A(%rdi), %rsi
  movl $B, %ecx
  leaq C(%rdi), %rdx
.L6:
  vbroadcastsd (%rsi), %ymm1
  xorl %eax, %eax
.L3:
  vmovapd (%rcx,%rax), %ymm0
  vfmadd213pd (%rdx,%rax), %ymm1, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  addq $32, %rax
  cmpq $2048, %rax
  jne .L3
  addq $2048, %rcx
  addq $8, %rsi
  cmpq $B+524288, %rcx
  jne .L6
  addq $2048, %rdi
  cmpq $524288, %rdi
  jne .L2
  vzeroupper
  ret
.LC5:
  .string "matmul C[128][128] = %.4f\n"
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  movl $8, %eax
  xorl %esi, %esi
  pushq -8(%r10)
  vpcmpeqd %ymm6, %ymm6, %ymm6
  vmovd %eax, %xmm5
  movl $A, %ecx
  pushq %rbp
  movl $B, %edx
  vpsrld $31, %ymm6, %ymm6
  vpbroadcastd %xmm5, %ymm5
  movq %rsp, %rbp
  pushq %r10
  subq $40, %rsp
  vmovdqa .LC0(%rip), %ymm7
  vbroadcastsd .LC3(%rip), %ymm3
.L11:
  vmovd %esi, %xmm4
  xorl %eax, %eax
  vmovdqa %ymm7, %ymm2
  vpbroadcastd %xmm4, %ymm4
.L12:
  vpaddd %ymm2, %ymm4, %ymm1
  vpmulld %ymm2, %ymm4, %ymm0
  vpaddd %ymm5, %ymm2, %ymm2
  vcvtdq2pd %xmm1, %ymm8
  vmulpd %ymm3, %ymm8, %ymm8
  vextracti128 $0x1, %ymm1, %xmm1
  vcvtdq2pd %xmm1, %ymm1
  vmulpd %ymm3, %ymm1, %ymm1
  vpaddd %ymm6, %ymm0, %ymm0
  vmovapd %ymm8, (%rcx,%rax)
  vmovapd %ymm1, 32(%rcx,%rax)
  vcvtdq2pd %xmm0, %ymm1
  vextracti128 $0x1, %ymm0, %xmm0
  vmulpd %ymm3, %ymm1, %ymm1
  vcvtdq2pd %xmm0, %ymm0
  vmulpd %ymm3, %ymm0, %ymm0
  vmovapd %ymm1, (%rdx,%rax)
  vmovapd %ymm0, 32(%rdx,%rax)
  addq $64, %rax
  cmpq $2048, %rax
  jne .L12
  addl $1, %esi
  addq $2048, %rcx
  addq $2048, %rdx
  cmpl $256, %esi
  jne .L11
  vzeroupper
  call matmul
  vmovsd C+263168(%rip), %xmm0
  movl $.LC5, %edi
  movl $1, %eax
  vmovsd %xmm0, -24(%rbp)
  vmovsd -24(%rbp), %xmm0
  call printf
  addq $40, %rsp
  xorl %eax, %eax
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
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
.LC3:
  .long 0
  .long 1064304640
