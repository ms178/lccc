main:
  pushq %rax
  movabsq $50000005000000, %rax
  movq %rax, (%rsp)
  movq (%rsp), %rsi
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "sum(10000000) = %ld\n"

