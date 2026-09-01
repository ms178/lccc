clz32:
..B3.1: # Preds ..B3.0
  movl $-1, %edx #14.35
  bsr %edi, %eax #14.35
  cmove %edx, %eax #14.35
  negl %eax #14.35
  addl $31, %eax #14.35
  ret #14.35
