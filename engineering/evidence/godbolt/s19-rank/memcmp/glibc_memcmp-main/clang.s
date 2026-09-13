main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $88, %rsp
  vmovaps .LCPI0_0(%rip), %ymm0
  vmovups %ymm0, 48(%rsp)
  vmovups %ymm0, 16(%rsp)
  leaq 48(%rsp), %rdi
  leaq 16(%rsp), %rsi
  movl $4, %edx
  vzeroupper
  callq glibc_memcmp_common_alignment
  testl %eax, %eax
  je .LBB0_2
  movl $2, %eax
  jmp .LBB0_8
.LBB0_2:
  movabsq $72623859790382857, %rax
  movq %rax, 24(%rsp)
  leaq 48(%rsp), %rdi
  leaq 16(%rsp), %rsi
  movl $4, %edx
  callq glibc_memcmp_common_alignment
  movl %eax, %ecx
  movl $2, %eax
  testl %ecx, %ecx
  jns .LBB0_8
  movabsq $-7046029254386353131, %rcx
  xorl %eax, %eax
  leaq glibc_left(%rip), %r14
  leaq glibc_right(%rip), %r15
.LBB0_4:
  movq %rcx, %rdx
  shlq $7, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shrq $9, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shlq $8, %rdx
  xorq %rcx, %rdx
  movq %rdx, (%r14,%rax,8)
  movq %rdx, (%r15,%rax,8)
  movq %rdx, %rcx
  shlq $7, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shrq $9, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shlq $8, %rcx
  xorq %rdx, %rcx
  movq %rcx, 8(%r14,%rax,8)
  movq %rcx, 8(%r15,%rax,8)
  addq $2, %rax
  cmpq $8192, %rax
  jne .LBB0_4
  movl $1, %r12d
  xorl %ebx, %ebx
  xorl %ebp, %ebp
  xorl %esi, %esi
.LBB0_6:
  movq %rsi, 8(%rsp)
  movl %ebx, %r13d
  andl $8191, %r13d
  shll $3, %r13d
  movq (%r13,%r14), %rax
  movq %rax, (%rsp)
  btcq %rbp, %rax
  movq %rax, (%r13,%r15)
  movl $8192, %edx
  movq %r14, %rdi
  movq %r15, %rsi
  callq glibc_memcmp_common_alignment
  movq 8(%rsp), %rsi
  addl $257, %eax
  imulq %r12, %rax
  addq %rax, %rsi
  movq (%rsp), %rax
  movq %rax, (%r13,%r15)
  addq $4051, %rbx
  addq $11, %rbp
  incq %r12
  cmpq $45056, %rbp
  jne .LBB0_6
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
.LBB0_8:
  addq $88, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

