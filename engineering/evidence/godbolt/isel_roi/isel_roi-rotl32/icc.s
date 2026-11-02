rotl32:
..B18.1: # Preds ..B18.0
  movl %esi, %r8d #33.5
  andl $31, %r8d #33.5
  movl %esi, %ecx #34.23
  movl %edi, %eax #34.23
  movl %edi, %edx #34.41
  shll %cl, %eax #34.23
  movl %r8d, %ecx #34.41
  negl %ecx #34.41
  shrl %cl, %edx #34.41
  orl %edx, %eax #34.41
  testl %r8d, %r8d #34.12
  cmove %edi, %eax #34.12
  ret #34.12
