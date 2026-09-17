.LC0:
main:
  pushq %r15
  xorl %r8d, %r8d
  movl $42, %edx
  xorl %edi, %edi
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbp
  pushq %rbx
  subq $248, %rsp
.L3:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  movl %edx, %eax
  shrl $2, %eax
  imulq $381774871, %rax, %rax
  shrq $34, %rax
  imull $180, %eax, %ecx
  movl %edx, %eax
  subl %ecx, %eax
  leaq strings(%r8), %rcx
  leal 10(%rax), %r10d
  leaq strings+10(%r8,%rax), %r9
.L2:
  imull $1664525, %edx, %edx
  addq $1, %rcx
  leal 1013904223(%rdx), %eax
  movq %rax, %rdx
  imulq $1321528399, %rax, %rax
  movl %edx, %esi
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %esi
  leal 97(%rsi), %eax
  movb %al, -1(%rcx)
  cmpq %r9, %rcx
  jne .L2
  movl %edi, %eax
  movl %r10d, %ecx
  addl $1, %edi
  addq $200, %r8
  imulq $200, %rax, %rax
  movb $0, strings(%rcx,%rax)
  cmpl $100000, %edi
  jne .L3
  movl $50, %r14d
  xorl %r13d, %r13d
  movl $strings+20000000, %r12d
.L4:
  movl $strings, %ebx
.L5:
  movq %rbx, %rdi
  addq $200, %rbx
  call strlen
  leaq 0(%r13,%rax), %rbp
  movq %rbp, %r13
  cmpq %r12, %rbx
  jne .L5
  subl $1, %r14d
  jne .L4
  movl $strings+19999800, %ebx
  movl $strings, %r13d
  xorl %r14d, %r14d
.L7:
  movq %r13, %rdi
  addq $200, %r13
  movq %r13, %rsi
  call strcmp
  xorl %edx, %edx
  testl %eax, %eax
  setg %dl
  shrl $31, %eax
  subl %eax, %edx
  movslq %edx, %rdx
  addq %rdx, %r14
  cmpq %rbx, %r13
  jne .L7
  movl $6513249, 28(%rsp)
  movl $strings, %edi
  xorl %r15d, %r15d
.L15:
  movq %rdi, %rsi
  cmpb $0, (%rdi)
  je .L19
.L8:
  movzbl (%rsi), %edx
  movl $97, %ecx
  xorl %eax, %eax
  testb %dl, %dl
  je .L13
.L14:
  cmpb %cl, %dl
  jne .L13
  movzbl 1(%rsi,%rax), %edx
  addq $1, %rax
  movzbl 28(%rsp,%rax), %ecx
  testb %dl, %dl
  je .L10
  testb %cl, %cl
  jne .L14
.L11:
  addq $1, %r15
.L19:
  addq $200, %rdi
  cmpq %r12, %rdi
  jne .L15
  movl $50, 12(%rsp)
  xorl %r13d, %r13d
.L16:
  movl $strings, %ebx
.L17:
  movq %rbx, %rdi
  call strlen
  movq %rbx, %rsi
  leaq 32(%rsp), %rdi
  addq $200, %rbx
  leal 1(%rax), %edx
  call memcpy
  movsbq 32(%rsp), %rax
  addq %rax, %r13
  cmpq %r12, %rbx
  jne .L17
  subl $1, 12(%rsp)
  jne .L16
  movq %r13, %r8
  movq %r15, %rcx
  movq %r14, %rdx
  movq %rbp, %rsi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  addq $248, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %rbp
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  ret
.L10:
  testb %cl, %cl
  je .L11
.L13:
  addq $1, %rsi
  cmpb $0, (%rsi)
  jne .L8
  jmp .L19
