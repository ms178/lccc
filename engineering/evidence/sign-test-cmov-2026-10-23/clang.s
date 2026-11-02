nonnegative_difference:
  subl %esi, %edi
  xorl %eax, %eax
  testl %edi, %edi
  cmovgl %edi, %eax
  retq
