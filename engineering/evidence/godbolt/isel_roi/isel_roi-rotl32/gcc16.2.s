rotl32:
  movl %edi, %eax
  movl %esi, %ecx
  roll %cl, %eax
  ret
