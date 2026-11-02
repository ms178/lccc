main:
  xorl %eax, %eax
  movl $1, %esi
  movl $20000, %edi
  leaq ring(%rip), %r8
  xorl %edx, %edx
  xorl %ecx, %ecx
  jmp .LBB0_1
.LBB0_9:
  addl $-2, %edi
  je .LBB0_10
.LBB0_1:
  movl %ecx, %r9d
  subl %eax, %r9d
  notl %r9d
  testl $1023, %r9d
  je .LBB0_3
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  movl %ecx, %r9d
  andl $1023, %r9d
  movl %esi, (%r8,%r9,4)
  incl %ecx
.LBB0_3:
  cmpl %eax, %ecx
  je .LBB0_5
  movl %eax, %r9d
  andl $1023, %r9d
  addl (%r8,%r9,4), %edx
  incl %eax
.LBB0_5:
  movl %ecx, %r9d
  subl %eax, %r9d
  notl %r9d
  testl $1023, %r9d
  jne .LBB0_6
  cmpl %eax, %ecx
  je .LBB0_9
  jmp .LBB0_8
.LBB0_6:
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  movl %ecx, %r9d
  andl $1023, %r9d
  movl %esi, (%r8,%r9,4)
  incl %ecx
  cmpl %eax, %ecx
  je .LBB0_9
.LBB0_8:
  movl %eax, %r9d
  andl $1023, %r9d
  addl (%r8,%r9,4), %edx
  incl %eax
  jmp .LBB0_9
.LBB0_10:
  xorl $20000, %ecx
  xorl $20000, %eax
  xorl %esi, %esi
  cmpl $1920339856, %edx
  setne %sil
  addl %esi, %esi
  orl %ecx, %eax
  movl $1, %eax
  cmovel %esi, %eax
  retq

