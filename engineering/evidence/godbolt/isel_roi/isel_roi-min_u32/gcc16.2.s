min_u32:
  cmpl %edi, %esi
  movl %edi, %eax
  cmovbe %esi, %eax
  ret
