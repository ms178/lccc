min_u32:
  movl %esi, %eax
  cmpl %esi, %edi
  cmovbl %edi, %eax
  retq

