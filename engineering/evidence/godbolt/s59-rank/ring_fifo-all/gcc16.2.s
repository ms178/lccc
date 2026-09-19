main:
  movl $20000, %edx
  movl $1, %eax
  xorl %ecx, %ecx
.L2:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  addl %eax, %ecx
  subl $1, %edx
  jne .L2
  xorl %eax, %eax
  cmpl $1920339856, %ecx
  setne %al
  addl %eax, %eax
  ret
