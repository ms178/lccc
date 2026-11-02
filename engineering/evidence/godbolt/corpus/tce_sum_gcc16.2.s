.LC0:
  .string "sum(10000000) = %ld\n"
main:
  subq $24, %rsp
  movl $.LC0, %edi
  movabsq $50000005000000, %rax
  movq %rax, 8(%rsp)
  movq 8(%rsp), %rsi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $24, %rsp
  ret
