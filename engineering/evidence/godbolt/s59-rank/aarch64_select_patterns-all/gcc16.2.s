conditional_increment:
  cmpl $1, %esi
  movl %edi, %eax
  sbbl $-1, %eax
  ret
narrow_high_constant:
  xorl %eax, %eax
  cmpw $-2, %di
  sete %al
  ret
select_pressure:
  testl %edi, %edi
  cmovne %esi, %edx
  cmovne %ecx, %esi
  addl %edx, %esi
  leal (%rsi,%r9), %eax
  ret
.LC0:
main:
  subq $8, %rsp
  xorl %r11d, %r11d
  xorl %r9d, %r9d
  xorl %ecx, %ecx
  movl $1, %r10d
.L12:
  imull $1664525, %r10d, %r10d
  movl %ecx, %edi
  movl %r11d, %edx
  addl $1, %r11d
  addl $1013904223, %r10d
  movl %r10d, %esi
  movl %r10d, %r8d
  andl $1, %esi
  shrl $3, %r8d
  call conditional_increment
  movl %r10d, %edi
  movl %r10d, %esi
  movl %eax, %ecx
  andl $8, %edi
  call select_pressure
  addl %eax, %r9d
  cmpl $50000000, %r11d
  jne .L12
  movl $65534, %edi
  movl %ecx, %esi
  call narrow_high_constant
  movl $.LC0, %edi
  xorl %r9d, %eax
  movl %eax, %edx
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
