rot:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movl (%rdi), %ecx
  movl 4(%rdi), %r8d
  movl 8(%rdi), %eax
  movl 12(%rdi), %r9d
  movl 16(%rdi), %ebx
  movl 20(%rdi), %r11d
  movl 24(%rdi), %r10d
  movl 28(%rdi), %edi
  testl %edx, %edx
  jle .LBB0_5
  cmpl $1, %edx
  jne .LBB0_6
  xorl %edx, %edx
  jmp .LBB0_4
.LBB0_6:
  movl %edx, %r14d
  movl %r14d, %r15d
  andl $2147483646, %r15d
  xorl %edx, %edx
  movl %edi, %ebp
  movl %r10d, %r12d
.LBB0_7:
  movl %ebx, %r10d
  movl %r11d, %edi
  movl %r9d, %r11d
  movl %r8d, %r9d
  leal (%rdi,%r10), %r13d
  addl %r12d, %ebp
  addl %r13d, %ebp
  addl (%rsi,%rdx,4), %ebp
  leal (%r9,%rax), %r8d
  movl %eax, %ebx
  movl %ecx, %eax
  addl %ebp, %r11d
  addl %ecx, %r8d
  addl %ebp, %r8d
  addl %r12d, %r13d
  addl %r11d, %r13d
  addl 4(%rsi,%rdx,4), %r13d
  addl %r13d, %ebx
  leal (%rax,%r9), %ecx
  addl %r8d, %ecx
  addl %r13d, %ecx
  addq $2, %rdx
  movl %edi, %ebp
  movl %r10d, %r12d
  cmpq %rdx, %r15
  jne .LBB0_7
  testb $1, %r14b
  je .LBB0_5
.LBB0_4:
  movl %r10d, %ebp
  movl %r11d, %r10d
  movl %ebx, %r11d
  leal (%r10,%r11), %r14d
  movl %r9d, %ebx
  movl %eax, %r9d
  addl %ebp, %edi
  addl %r14d, %edi
  addl (%rsi,%rdx,4), %edi
  addl %edi, %ebx
  leal (%r8,%r9), %edx
  addl %ecx, %edx
  addl %edi, %edx
  movl %r8d, %eax
  movl %ebp, %edi
  movl %ecx, %r8d
  movl %edx, %ecx
.LBB0_5:
  xorl %edi, %r10d
  xorl %r8d, %r10d
  xorl %r11d, %ebx
  xorl %r9d, %eax
  xorl %ebx, %eax
  xorl %r10d, %eax
  xorl %ecx, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq
