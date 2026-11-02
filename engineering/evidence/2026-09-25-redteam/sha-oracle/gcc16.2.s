sha256_transform:
  pushq %rbp
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  movq %rdi, %r12
  pushq %rbx
  subq $176, %rsp
  leaq -88(%rsp), %r13
  leaq 104(%rsp), %rdx
  movq %r13, %rax
  vmovdqu (%rsi), %ymm0
  vmovdqu %ymm0, -88(%rsp)
  vmovdqu 32(%rsi), %ymm0
  vmovdqu %ymm0, -56(%rsp)
  vmovq -32(%rsp), %xmm0
.L2:
  vpsrld $19, %xmm0, %xmm3
  vpslld $13, %xmm0, %xmm1
  vmovq 4(%rax), %xmm2
  addq $8, %rax
  vpsrld $17, %xmm0, %xmm4
  vpor %xmm3, %xmm1, %xmm1
  vpslld $15, %xmm0, %xmm3
  vpsrld $10, %xmm0, %xmm0
  vpor %xmm4, %xmm3, %xmm3
  vpsrld $7, %xmm2, %xmm4
  vpxor %xmm3, %xmm1, %xmm1
  vpsrld $18, %xmm2, %xmm3
  vpxor %xmm0, %xmm1, %xmm1
  vpslld $14, %xmm2, %xmm0
  vpor %xmm3, %xmm0, %xmm0
  vpslld $25, %xmm2, %xmm3
  vpor %xmm4, %xmm3, %xmm3
  vpsrld $3, %xmm2, %xmm2
  vpxor %xmm3, %xmm0, %xmm0
  vpxor %xmm2, %xmm0, %xmm0
  vmovq -8(%rax), %xmm2
  vpaddd %xmm0, %xmm1, %xmm1
  vmovq 28(%rax), %xmm0
  vpaddd %xmm2, %xmm0, %xmm0
  vpaddd %xmm0, %xmm1, %xmm0
  vmovq %xmm0, 56(%rax)
  cmpq %rax, %rdx
  jne .L2
  movl (%r12), %r15d
  movl 28(%r12), %eax
  xorl %edi, %edi
  movl 4(%r12), %r9d
  movl 8(%r12), %r8d
  movl 12(%r12), %ebx
  movl 16(%r12), %ecx
  movl %eax, -116(%rsp)
  movl %eax, %edx
  movl 20(%r12), %r11d
  movl 24(%r12), %r10d
  movl %r9d, -92(%rsp)
  movl %r15d, %esi
  movl %r8d, -96(%rsp)
  movl %ebx, -100(%rsp)
  movl %ecx, -104(%rsp)
  movl %r11d, -108(%rsp)
  movl %r10d, -112(%rsp)
  movl %r15d, -120(%rsp)
  jmp .L3
.L4:
  movl %r11d, %r10d
  movl %r9d, %r8d
  movl %ecx, %r11d
  movl %esi, %r9d
  movl %r14d, %ecx
  movl %eax, %esi
.L3:
  rorx $11, %ecx, %r14d
  movl %ecx, %r15d
  rorx $6, %ecx, %eax
  xorl %r14d, %eax
  andl %r11d, %r15d
  rorx $25, %ecx, %r14d
  xorl %r14d, %eax
  movl 0(%r13,%rdi), %r14d
  addl K(%rdi), %r14d
  addq $4, %rdi
  addl %r14d, %eax
  andn %r10d, %ecx, %r14d
  xorl %r15d, %r14d
  movl %r9d, %r15d
  addl %r14d, %eax
  rorx $13, %esi, %r14d
  andl %r8d, %r15d
  addl %edx, %eax
  rorx $2, %esi, %edx
  xorl %r14d, %edx
  rorx $22, %esi, %r14d
  xorl %r14d, %edx
  movl %r9d, %r14d
  xorl %r8d, %r14d
  andl %esi, %r14d
  xorl %r15d, %r14d
  addl %r14d, %edx
  leal (%rax,%rbx), %r14d
  movl %r8d, %ebx
  addl %edx, %eax
  movl %r10d, %edx
  cmpq $256, %rdi
  jne .L4
  movl -120(%rsp), %r15d
  addl %eax, %r15d
  movl -92(%rsp), %eax
  movl %r15d, (%r12)
  addl %esi, %eax
  movl %eax, 4(%r12)
  movl -96(%rsp), %eax
  addl %r9d, %eax
  movl %eax, 8(%r12)
  movl -100(%rsp), %eax
  addl %r8d, %eax
  movl %eax, 12(%r12)
  movl -104(%rsp), %eax
  addl %r14d, %eax
  movl %eax, 16(%r12)
  movl -108(%rsp), %eax
  addl %ecx, %eax
  movl %eax, 20(%r12)
  movl -112(%rsp), %eax
  addl %r11d, %eax
  movl %eax, 24(%r12)
  movl -116(%rsp), %eax
  addl %r10d, %eax
  movl %eax, 28(%r12)
  vzeroupper
  addq $176, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  ret
.LC1:
  .string "%016llx\n"
