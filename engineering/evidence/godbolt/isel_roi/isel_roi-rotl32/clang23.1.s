rotl32:
  movl %esi, %ecx
  movl %edi, %eax
  roll %cl, %eax
  testb $31, %cl
  cmovel %edi, %eax
  retq

