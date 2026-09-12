sha256_transform:
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  movq %rdi, %r12
  pushq %rbp
  pushq %rbx
  subq $176, %rsp
  movdqu (%rsi), %xmm0
  leaq -24(%rsp), %rax
  movaps %xmm0, -88(%rsp)
  movdqu 16(%rsi), %xmm0
  movaps %xmm0, -72(%rsp)
  movdqu 32(%rsi), %xmm0
  movaps %xmm0, -56(%rsp)
  movdqu 48(%rsi), %xmm0
  movaps %xmm0, -40(%rsp)
  movq -32(%rsp), %xmm0
.L2:
  movdqa %xmm0, %xmm1
  movdqa %xmm0, %xmm3
  movdqa %xmm0, %xmm4
  addq $8, %rax
  psrld $19, %xmm3
  pslld $13, %xmm1
  movq -68(%rax), %xmm2
  leaq 168(%rsp), %rbx
  por %xmm3, %xmm1
  psrld $17, %xmm4
  movdqa %xmm0, %xmm3
  pslld $15, %xmm3
  psrld $10, %xmm0
  por %xmm4, %xmm3
  movdqa %xmm2, %xmm4
  pxor %xmm3, %xmm1
  psrld $7, %xmm4
  movdqa %xmm2, %xmm3
  pxor %xmm0, %xmm1
  psrld $18, %xmm3
  movdqa %xmm2, %xmm0
  pslld $14, %xmm0
  por %xmm3, %xmm0
  movdqa %xmm2, %xmm3
  pslld $25, %xmm3
  psrld $3, %xmm2
  por %xmm4, %xmm3
  pxor %xmm3, %xmm0
  pxor %xmm2, %xmm0
  movq -72(%rax), %xmm2
  paddd %xmm0, %xmm1
  movq -36(%rax), %xmm0
  paddd %xmm2, %xmm0
  paddd %xmm1, %xmm0
  movq %xmm0, -8(%rax)
  cmpq %rax, %rbx
  jne .L2
  movl (%r12), %r13d
  movl 28(%r12), %eax
  xorl %edi, %edi
  leaq -88(%rsp), %rbp
  movl 4(%r12), %r15d
  movl 8(%r12), %r8d
  movl 12(%r12), %ebx
  movl 16(%r12), %ecx
  movl %eax, -96(%rsp)
  movl %eax, %edx
  movl 20(%r12), %r11d
  movl 24(%r12), %r10d
  movl %r15d, %r9d
  movl %r13d, %esi
  movl %r8d, -116(%rsp)
  movl %ebx, -112(%rsp)
  movl %ecx, -108(%rsp)
  movl %r11d, -104(%rsp)
  movl %r10d, -100(%rsp)
  movl %r13d, -92(%rsp)
  jmp .L3
.L4:
  movl %r11d, %r10d
  movl %r9d, %r8d
  movl %ecx, %r11d
  movl %esi, %r9d
  movl %r14d, %ecx
  movl %eax, %esi
.L3:
  movl %ecx, %eax
  movl %ecx, %r13d
  movl %ecx, %r14d
  rorl $11, %r13d
  rorl $6, %eax
  andl %r11d, %r14d
  xorl %r13d, %eax
  movl %ecx, %r13d
  roll $7, %r13d
  xorl %r13d, %eax
  movl 0(%rbp,%rdi), %r13d
  addl K(%rdi), %r13d
  addq $4, %rdi
  addl %r13d, %eax
  movl %ecx, %r13d
  notl %r13d
  andl %r10d, %r13d
  xorl %r14d, %r13d
  movl %r9d, %r14d
  addl %r13d, %eax
  movl %esi, %r13d
  andl %r8d, %r14d
  addl %edx, %eax
  movl %esi, %edx
  rorl $13, %r13d
  rorl $2, %edx
  xorl %r13d, %edx
  movl %esi, %r13d
  roll $10, %r13d
  xorl %r13d, %edx
  movl %r9d, %r13d
  xorl %r8d, %r13d
  andl %esi, %r13d
  xorl %r14d, %r13d
  leal (%rax,%rbx), %r14d
  movl %r8d, %ebx
  addl %r13d, %edx
  addl %edx, %eax
  movl %r10d, %edx
  cmpq $256, %rdi
  jne .L4
  movl -92(%rsp), %r13d
  addl %esi, %r15d
  movl %r15d, 4(%r12)
  addl %eax, %r13d
  movl -116(%rsp), %eax
  movl %r13d, (%r12)
  addl %r9d, %eax
  movl %eax, 8(%r12)
  movl -112(%rsp), %eax
  addl %r8d, %eax
  movl %eax, 12(%r12)
  movl -108(%rsp), %eax
  addl %r14d, %eax
  movl %eax, 16(%r12)
  movl -104(%rsp), %eax
  addl %ecx, %eax
  movl %eax, 20(%r12)
  movl -100(%rsp), %eax
  addl %r11d, %eax
  movl %eax, 24(%r12)
  movl -96(%rsp), %eax
  addl %r10d, %eax
  movl %eax, 28(%r12)
  addq $176, %rsp
  popq %rbx
  popq %rbp
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  ret
.LC2:
  .string "%016llx\n"
