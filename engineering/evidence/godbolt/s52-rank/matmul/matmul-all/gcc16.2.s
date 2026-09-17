matmul:
  xorl %r8d, %r8d
.L2:
  leaq A(%r8), %rdi
  movl $B, %ecx
  leaq C(%r8), %rdx
.L6:
  vbroadcastsd (%rdi), %ymm2
  vbroadcastsd 8(%rdi), %ymm1
  leaq 2048(%rcx), %rsi
  xorl %eax, %eax
.L3:
  vmovapd (%rcx,%rax), %ymm0
  vfmadd213pd (%rdx,%rax), %ymm2, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  vfmadd231pd (%rsi,%rax), %ymm1, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  addq $32, %rax
  cmpq $2048, %rax
  jne .L3
  addq $4096, %rcx
  addq $16, %rdi
  cmpq $B+524288, %rcx
  jne .L6
  addq $2048, %r8
  cmpq $524288, %r8
  jne .L2
  vzeroupper
  ret
.LC5:
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  movl $8, %eax
  xorl %esi, %esi
  pushq -8(%r10)
  vpcmpeqd %ymm6, %ymm6, %ymm6
  vmovd %eax, %xmm5
  movl $B, %ecx
  pushq %rbp
  movl $A, %edx
  vpsrld $31, %ymm6, %ymm6
  vpbroadcastd %xmm5, %ymm5
  movq %rsp, %rbp
  pushq %r10
  subq $40, %rsp
  vmovdqa .LC0(%rip), %ymm7
  vbroadcastsd .LC3(%rip), %ymm2
.L11:
  vmovd %esi, %xmm4
  xorl %eax, %eax
  vmovdqa %ymm7, %ymm3
  vpbroadcastd %xmm4, %ymm4
.L12:
  vpaddd %ymm3, %ymm4, %ymm1
  vpmulld %ymm3, %ymm4, %ymm0
  vpaddd %ymm5, %ymm3, %ymm3
  vcvtdq2pd %xmm1, %ymm8
  vmulpd %ymm2, %ymm8, %ymm8
  vextracti128 $0x1, %ymm1, %xmm1
  vcvtdq2pd %xmm1, %ymm1
  vmulpd %ymm2, %ymm1, %ymm1
  vpaddd %ymm6, %ymm0, %ymm0
  vmovapd %ymm8, (%rdx,%rax)
  vmovapd %ymm1, 32(%rdx,%rax)
  vcvtdq2pd %xmm0, %ymm1
  vextracti128 $0x1, %ymm0, %xmm0
  vmulpd %ymm2, %ymm1, %ymm1
  vcvtdq2pd %xmm0, %ymm0
  vmulpd %ymm2, %ymm0, %ymm0
  vmovapd %ymm1, (%rcx,%rax)
  vmovapd %ymm0, 32(%rcx,%rax)
  addq $64, %rax
  cmpq $2048, %rax
  jne .L12
  addl $1, %esi
  addq $2048, %rdx
  addq $2048, %rcx
  cmpl $256, %esi
  jne .L11
  xorl %r8d, %r8d
.L13:
  leaq A(%r8), %rdi
  movl $B, %ecx
  leaq C(%r8), %rdx
.L17:
  vbroadcastsd (%rdi), %ymm2
  vbroadcastsd 8(%rdi), %ymm1
  leaq 2048(%rcx), %rsi
  xorl %eax, %eax
.L14:
  vmovapd (%rcx,%rax), %ymm0
  vfmadd213pd (%rdx,%rax), %ymm2, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  vfmadd231pd (%rsi,%rax), %ymm1, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  addq $32, %rax
  cmpq $2048, %rax
  jne .L14
  addq $4096, %rcx
  addq $16, %rdi
  cmpq $B+524288, %rcx
  jne .L17
  addq $2048, %r8
  cmpq $524288, %r8
  jne .L13
  vmovsd C+263168(%rip), %xmm0
  movl $.LC5, %edi
  movl $1, %eax
  vmovsd %xmm0, -24(%rbp)
  vmovsd -24(%rbp), %xmm0
  vzeroupper
  call printf
  addq $40, %rsp
  xorl %eax, %eax
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
  ret
.LC0:
.LC3:
