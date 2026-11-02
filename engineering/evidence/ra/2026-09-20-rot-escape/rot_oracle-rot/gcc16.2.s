rot:
  subq $24, %rsp
  movq %rsi, %rax
  movl (%rdi), %r10d
  movl 4(%rdi), %r9d
  movl 8(%rdi), %esi
  movl 12(%rdi), %r11d
  movq %rbx, (%rsp)
  movq %rdi, %rbx
  movl 16(%rdi), %r8d
  movl 24(%rbx), %ecx
  movq %r12, 16(%rsp)
  movl 20(%rdi), %edi
  movl 28(%rbx), %r12d
  testl %edx, %edx
  jle .L4
  movl %edx, %edx
  movq %rbp, 8(%rsp)
  leaq (%rax,%rdx,4), %rbp
  jmp .L3
.L5:
  movl %edi, %ecx
  movl %r9d, %esi
  movl %r8d, %edi
  movl %r10d, %r9d
  movl %ebx, %r8d
  movl %edx, %r10d
.L3:
  leal (%r8,%rdi), %edx
  addq $4, %rax
  addl %ecx, %edx
  addl -4(%rax), %edx
  addl %r12d, %edx
  leal (%r10,%r9), %r12d
  addl %esi, %r12d
  leal (%rdx,%r11), %ebx
  movl %esi, %r11d
  addl %r12d, %edx
  movl %ecx, %r12d
  cmpq %rax, %rbp
  jne .L5
  movq 8(%rsp), %rbp
.L2:
  movl %edx, %eax
  movq 16(%rsp), %r12
  xorl %r10d, %eax
  xorl %r9d, %eax
  xorl %esi, %eax
  xorl %ebx, %eax
  movq (%rsp), %rbx
  addq $24, %rsp
  xorl %r8d, %eax
  xorl %edi, %eax
  xorl %ecx, %eax
  ret
.L4:
  movl %r8d, %ebx
  movl %r10d, %edx
  movl %edi, %r8d
  movl %r9d, %r10d
  movl %ecx, %edi
  movl %esi, %r9d
  movl %r12d, %ecx
  movl %r11d, %esi
  jmp .L2
