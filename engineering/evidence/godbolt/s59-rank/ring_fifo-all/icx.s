main:
  vstmxcsr -4(%rsp)
  orl $32832, -4(%rsp)
  vldmxcsr -4(%rsp)
  xorl %eax, %eax
  movl $1, %esi
  movl $5000, %edi
  xorl %ecx, %ecx
  xorl %edx, %edx
  jmp .LBB0_1
.LBB0_17:
  decl %edi
  je .LBB0_18
.LBB0_1:
  movl %eax, %r8d
  subl %ecx, %r8d
  notl %r8d
  testl $1023, %r8d
  je .LBB0_3
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  movl %eax, %r8d
  andl $1023, %r8d
  movl %esi, ring(,%r8,4)
  incl %eax
.LBB0_3:
  cmpl %ecx, %eax
  je .LBB0_5
  movl %ecx, %r8d
  andl $1023, %r8d
  addl ring(,%r8,4), %edx
  incl %ecx
.LBB0_5:
  movl %eax, %r8d
  subl %ecx, %r8d
  notl %r8d
  testl $1023, %r8d
  je .LBB0_7
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  movl %eax, %r8d
  andl $1023, %r8d
  movl %esi, ring(,%r8,4)
  incl %eax
.LBB0_7:
  cmpl %ecx, %eax
  je .LBB0_9
  movl %ecx, %r8d
  andl $1023, %r8d
  addl ring(,%r8,4), %edx
  incl %ecx
.LBB0_9:
  movl %eax, %r8d
  subl %ecx, %r8d
  notl %r8d
  testl $1023, %r8d
  je .LBB0_11
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  movl %eax, %r8d
  andl $1023, %r8d
  movl %esi, ring(,%r8,4)
  incl %eax
.LBB0_11:
  cmpl %ecx, %eax
  je .LBB0_13
  movzwl %cx, %r8d
  andl $1023, %r8d
  addl ring(,%r8,4), %edx
  incl %ecx
.LBB0_13:
  movl %eax, %r8d
  subl %ecx, %r8d
  notl %r8d
  testl $1023, %r8d
  je .LBB0_15
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  movl %eax, %r8d
  andl $1023, %r8d
  movl %esi, ring(,%r8,4)
  incl %eax
.LBB0_15:
  cmpl %ecx, %eax
  je .LBB0_17
  movzwl %cx, %r8d
  andl $1023, %r8d
  addl ring(,%r8,4), %edx
  incl %ecx
  jmp .LBB0_17
.LBB0_18:
  xorl $20000, %eax
  xorl $20000, %ecx
  xorl %esi, %esi
  cmpl $1920339856, %edx
  setne %sil
  addl %esi, %esi
  orl %eax, %ecx
  movl $1, %eax
  cmovel %esi, %eax
  retq

