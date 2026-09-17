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
  leaq -24(%rsp), %rax
  vmovdqu (%rsi), %ymm0
  vmovdqu %ymm0, -88(%rsp)
  vmovdqu 32(%rsi), %ymm0
  vmovdqu %ymm0, -56(%rsp)
  vmovq -32(%rsp), %xmm0
.L2:
  vpsrld $19, %xmm0, %xmm3
  vpslld $13, %xmm0, %xmm1
  vmovq -60(%rax), %xmm2
  leaq 168(%rsp), %rbx
  vpsrld $17, %xmm0, %xmm4
  vpor %xmm3, %xmm1, %xmm1
  addq $8, %rax
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
  vmovq -72(%rax), %xmm2
  vpaddd %xmm0, %xmm1, %xmm1
  vmovq -36(%rax), %xmm0
  vpaddd %xmm2, %xmm0, %xmm0
  vpaddd %xmm0, %xmm1, %xmm0
  vmovq %xmm0, -8(%rax)
  cmpq %rax, %rbx
  jne .L2
  movl (%r12), %r15d
  movl 28(%r12), %eax
  xorl %edi, %edi
  leaq -88(%rsp), %r13
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
main:
  pushq %rbp
  vpxor %xmm0, %xmm0, %xmm0
  movq %rsp, %rbp
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  andq $-32, %rsp
  subq $96, %rsp
  vmovdqu %ymm0, 36(%rsp)
  leaq 32(%rsp), %rsi
  movq %rsp, %rdi
  vmovdqu %ymm0, 60(%rsp)
  vmovdqa .LC0(%rip), %ymm0
  movl $1633837952, 32(%rsp)
  movl $24, 92(%rsp)
  vmovdqa %ymm0, (%rsp)
  vzeroupper
  call sha256_transform
  cmpl $-1166534977, (%rsp)
  jne .L9
  cmpl $-1895706646, 4(%rsp)
  je .L16
.L9:
  leaq -32(%rbp), %rsp
  movl $2, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %rbp
  ret
.L16:
  cmpl $-234875475, 28(%rsp)
  jne .L9
  xorl %r13d, %r13d
  xorl %r12d, %r12d
.L10:
  movl %r13d, %esi
  xorl %ebx, %ebx
  movl $1541459225, %ecx
  movl $528734635, %r11d
  xorl $1779033703, %esi
  movl $-1694144372, %r10d
  movl $-1521486534, %edx
  movl $1359893119, %r9d
  movl $1013904242, %r8d
  movl $-1150833019, %edi
.L12:
  movl %esi, %eax
  movl %esi, (%rsp)
  xorl %ebx, %eax
  movl %edi, 4(%rsp)
  movl %eax, 32(%rsp)
  leal 1013904223(%rbx), %eax
  xorl %edi, %eax
  movl %r8d, 8(%rsp)
  movl %eax, 36(%rsp)
  leal 2027808446(%rbx), %eax
  xorl %r8d, %eax
  movl %edx, 12(%rsp)
  movl %eax, 40(%rsp)
  leal -1253254627(%rbx), %eax
  xorl %edx, %eax
  movl %r9d, 16(%rsp)
  movl %eax, 44(%rsp)
  leal -239350404(%rbx), %eax
  xorl %r9d, %eax
  movl %r10d, 20(%rsp)
  movl %eax, 48(%rsp)
  leal 774553819(%rbx), %eax
  xorl %r10d, %eax
  movl %r11d, 24(%rsp)
  movl %eax, 52(%rsp)
  leal 1788458042(%rbx), %eax
  xorl %r11d, %eax
  movl %ecx, 28(%rsp)
  movl %eax, 56(%rsp)
  leal -1492605031(%rbx), %eax
  xorl %ecx, %eax
  movl %eax, 60(%rsp)
  leal -478700808(%rbx), %eax
  xorl %esi, %eax
  leaq 32(%rsp), %rsi
  movl %eax, 64(%rsp)
  leal 535203415(%rbx), %eax
  xorl %edi, %eax
  movq %rsp, %rdi
  movl %eax, 68(%rsp)
  leal 1549107638(%rbx), %eax
  xorl %r8d, %eax
  movl %eax, 72(%rsp)
  leal -1731955435(%rbx), %eax
  xorl %edx, %eax
  movl %eax, 76(%rsp)
  leal -718051212(%rbx), %eax
  xorl %r9d, %eax
  movl %eax, 80(%rsp)
  leal 295853011(%rbx), %eax
  xorl %r10d, %eax
  movl %eax, 84(%rsp)
  leal 1309757234(%rbx), %eax
  xorl %r11d, %eax
  movl %eax, 88(%rsp)
  leal -1971305839(%rbx), %eax
  addl $1664525, %ebx
  xorl %ecx, %eax
  movl %eax, 92(%rsp)
  call sha256_transform
  movl (%rsp), %esi
  movl 12(%rsp), %r14d
  movl 28(%rsp), %ecx
  movl 4(%rsp), %edi
  movq %rsi, %rax
  movq %r14, %rdx
  salq $16, %r14
  movl 8(%rsp), %r8d
  salq $32, %rax
  movl 16(%rsp), %r9d
  movl 20(%rsp), %r10d
  xorq %r14, %rax
  movl %ecx, %r14d
  movl 24(%rsp), %r11d
  xorq %r14, %rax
  addq %rax, %r12
  cmpl $-870711296, %ebx
  jne .L12
  addl $1, %r13d
  cmpl $8, %r13d
  jne .L10
  movq %r12, %rsi
  movl $.LC1, %edi
  xorl %eax, %eax
  call printf
  leaq -32(%rbp), %rsp
  xorl %eax, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %rbp
  ret
K:
.LC0:
