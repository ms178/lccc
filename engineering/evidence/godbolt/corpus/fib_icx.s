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
  subq $24, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $40, %edi
  callq fib
  movq %rax, 16(%rsp)
  movq 16(%rsp), %rsi
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  retq

.L.str:
  .asciz "fib(40) = %ld\n"

