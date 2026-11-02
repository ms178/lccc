conditional_increment:
  movl %edi, %eax
  cmpl $1, %esi
  sbbl $-1, %eax
  retq

narrow_high_constant:
  xorl %eax, %eax
  cmpl $65534, %edi
  sete %al
  retq

select_pressure:
  testl %edi, %edi
  cmovnel %esi, %edx
  cmovel %esi, %ecx
  leal (%rcx,%rdx), %eax
  addl %r9d, %eax
  retq

main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %rbx
  pushq %rax
  movl $1, %ebp
  xorl %r14d, %r14d
  xorl %eax, %eax
  xorl %ebx, %ebx
.LBB3_1:
  movl %eax, %r15d
  imull $1664525, %ebp, %ebp
  addl $1013904223, %ebp
  movl %ebp, %esi
  andl $1, %esi
  movl %ebx, %edi
  callq conditional_increment
  movl %eax, %ebx
  movl %ebp, %edi
  andl $8, %edi
  movl %ebp, %esi
  movl %r14d, %edx
  movl %eax, %ecx
  movl %r15d, %r9d
  callq select_pressure
  addl %r15d, %eax
  incl %r14d
  cmpl $50000000, %r14d
  jne .LBB3_1
  movl $65534, %edi
  movl %eax, %ebp
  callq narrow_high_constant
  xorl %eax, %ebp
  leaq .L.str(%rip), %rdi
  movl %ebx, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

