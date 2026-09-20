rot:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movl (%rdi), %r8d
  movl 4(%rdi), %r11d
  movl 8(%rdi), %r15d
  movl 12(%rdi), %ecx
  movl 16(%rdi), %ebp
  movl 20(%rdi), %ebx
  movl 24(%rdi), %r9d
  movl 28(%rdi), %r10d
  testl %edx, %edx
  jle .LBB0_1
  movl %edx, %edi
  cmpl $4, %edx
  movq %rdi, -16(%rsp)
  jae .LBB0_6
  jmp .LBB0_9
.LBB0_1:
  movl %r10d, %r14d
  movl %r11d, %eax
  movl %r8d, %edx
  jmp .LBB0_5
.LBB0_6:
  shrl $2, %edi
  shlq $4, %rdi
  movq %rdi, -8(%rsp)
  xorl %r14d, %r14d
.LBB0_7:
  movl %r9d, %r13d
  movl %ebx, %r12d
  movl %ebp, %edx
  movl %r10d, %r9d
  leal (%rdx,%r12), %ebx
  leal (%rbx,%r13), %ebp
  movl (%rsi,%r14), %edi
  addl %ebp, %ecx
  addl %r9d, %ecx
  movl %ecx, %r10d
  leal (%r15,%r11), %ecx
  addl %r8d, %ecx
  addl %ebp, %ecx
  movl 4(%rsi,%r14), %eax
  addl %edi, %r10d
  addl %r9d, %ecx
  addl %edi, %ecx
  leal (%r10,%rdx), %ebp
  leal (%r12,%rax), %r9d
  addl %r13d, %r9d
  addl %ebp, %r9d
  addl %r15d, %r9d
  addl %r8d, %ebx
  addl %ecx, %ebx
  addl %r11d, %ebx
  addl %r10d, %ebx
  addl %r13d, %eax
  movl %eax, %r15d
  addl %ebx, %r15d
  movl 8(%rsi,%r14), %eax
  leal (%r9,%r10), %r13d
  leal (%rdx,%rax), %ebx
  addl %r12d, %ebx
  addl %r11d, %ebx
  addl %r13d, %ebx
  addl %ecx, %ebp
  addl %r15d, %ebp
  addl %r8d, %ebp
  addl %r9d, %ebp
  addl %r12d, %eax
  movl %eax, %r11d
  addl %ebp, %r11d
  movl 12(%rsi,%r14), %eax
  leal (%rax,%r13), %r12d
  addl %r15d, %r13d
  addl %ecx, %r13d
  addl %ebx, %r13d
  addl %r11d, %eax
  addl %r13d, %eax
  movl %r8d, %ebp
  addl %ebx, %r12d
  addl %edx, %r12d
  movl %eax, %r8d
  addl %edx, %r8d
  addl %r12d, %ebp
  addq $16, %r14
  cmpq %r14, -8(%rsp)
  jne .LBB0_7
  leal (%rcx,%r15), %edx
  addl %r11d, %edx
  addl %r12d, %edx
  movq -16(%rsp), %rdi
.LBB0_9:
  movl %edi, %r12d
  andl $-4, %r12d
  movl %r10d, %r14d
  movl %r11d, %eax
  cmpq %rdi, %r12
  jae .LBB0_5
.LBB0_10:
  movl %r9d, %r14d
  movl %ebx, %r9d
  movl %ebp, %ebx
  movl %ecx, %ebp
  movl %r8d, %eax
  movl %r15d, %ecx
  movl %r11d, %r15d
  movl (%rsi,%r12,4), %edi
  leal (%rbx,%r9), %r8d
  addl %r14d, %r8d
  leal (%r8,%r10), %r13d
  addl %edi, %r13d
  addl %r13d, %ebp
  leal (%rcx,%r15), %edx
  addl %eax, %edx
  addl %edx, %r8d
  addl %r10d, %r8d
  addl %edi, %r8d
  incq %r12
  movl %r14d, %r10d
  movl %eax, %r11d
  cmpq %r12, -16(%rsp)
  jne .LBB0_10
  addl %r13d, %edx
.LBB0_5:
  xorl %ebx, %ebp
  xorl %r9d, %ebp
  xorl %eax, %ecx
  xorl %edx, %r15d
  xorl %ecx, %r15d
  xorl %ebp, %r15d
  xorl %r14d, %r15d
  movl %r15d, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq
