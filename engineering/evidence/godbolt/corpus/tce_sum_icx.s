main:
  subq $24, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movabsq $50000005000000, %rax
  movq %rax, 16(%rsp)
  movq 16(%rsp), %rsi
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  retq

.L.str:
  .asciz "sum(10000000) = %ld\n"

