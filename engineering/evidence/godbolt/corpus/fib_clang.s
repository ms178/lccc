fib:
  pushq %r14
  pushq %rbx
  pushq %rax
  xorl %ebx, %ebx
  cmpl $2, %edi
  jl .LBB0_3
  movl %edi, %r14d
.LBB0_2:
  leal -1(%r14), %edi
  callq fib
  leal -2(%r14), %edi
  addq %rax, %rbx
  cmpl $4, %r14d
  movl %edi, %r14d
  jae .LBB0_2
.LBB0_3:
  movslq %edi, %rax
  addq %rbx, %rax
  addq $8, %rsp
  popq %rbx
  popq %r14
  retq

main:
  pushq %rax
  movl $40, %edi
  callq fib
  movq %rax, (%rsp)
  movq (%rsp), %rsi
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "fib(40) = %ld\n"

