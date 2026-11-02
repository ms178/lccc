nonnegative_difference:
  subl %esi, %edi
  movl $0, %eax
  cmovns %edi, %eax
  ret
