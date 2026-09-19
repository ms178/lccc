conditional_increment:
  movl %edi, %eax
  cmpl $1, %esi
  sbbl $-1, %eax
  retq

narrow_high_constant:
  xorl %eax, %eax
  cmpw $-2, %di
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
  pushq %r12
  pushq %rbx
  subq $16, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $1, %ebp
  xorl %r14d, %r14d
  xorl %r15d, %r15d
  xorl %ebx, %ebx
.LBB3_1:
  movl %r15d, %r12d
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
  movl %eax, %r15d
  addl %r12d, %r15d
  incl %r14d
  cmpl $50000000, %r14d
  jne .LBB3_1
  movl $65534, %edi
  callq narrow_high_constant
  xorl %eax, %r15d
  movl $.L.str, %edi
  movl %ebx, %esi
  movl %r15d, %edx
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $16, %rsp
  popq %rbx
  popq %r12
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

