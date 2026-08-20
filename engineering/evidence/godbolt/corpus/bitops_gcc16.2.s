.LC0:
  .string "pop=%ld clz=%ld rev=%ld pow2=%ld\n"
main:
  pushq %rbx
  movl $50000000, %ecx
  movl $1107023190, %r10d
  xorl %r8d, %r8d
  movl $18, %r11d
  xorl %r9d, %r9d
  xorl %edi, %edi
  xorl %esi, %esi
  movl $-559038737, %edx
  jmp .L9
.L15:
  imull $1664525, %edx, %eax
  xorl %r11d, %r11d
  addl $1013904223, %eax
  leal (%rax,%rax), %r10d
  popcntl %eax, %r11d
  shrl %eax
  andl $1431655765, %eax
  andl $-1431655766, %r10d
  orl %eax, %r10d
  movl %r10d, %eax
  sall $2, %r10d
  shrl $2, %eax
  andl $-858993460, %r10d
  andl $858993459, %eax
  orl %r10d, %eax
  movl %eax, %r10d
  sall $4, %eax
  shrl $4, %r10d
  andl $-252645136, %eax
  andl $252645135, %r10d
  orl %eax, %r10d
  bswap %r10d
.L9:
  imull $1664525, %edx, %edx
  addq %r11, %rsi
  movl $32, %eax
  addl $1013904223, %edx
  je .L2
  movl %edx, %eax
  xorl %r11d, %r11d
  cmpl $65535, %edx
  ja .L3
  sall $16, %eax
  movl $16, %r11d
.L3:
  cmpl $16777215, %eax
  leal 8(%r11), %ebx
  cmovbe %ebx, %r11d
  movl %eax, %ebx
  sall $8, %ebx
  cmpl $16777215, %eax
  cmovbe %ebx, %eax
  leal 4(%r11), %ebx
  cmpl $268435455, %eax
  cmovbe %ebx, %r11d
  movl %eax, %ebx
  sall $4, %ebx
  cmpl $268435455, %eax
  cmovbe %ebx, %eax
  leal 2(%r11), %ebx
  cmpl $1073741823, %eax
  cmovbe %ebx, %r11d
  leal 0(,%rax,4), %ebx
  cmovbe %ebx, %eax
  movl %r11d, %ebx
  addl $1, %r11d
  testl %eax, %eax
  movq %rbx, %rax
  cmovns %r11, %rax
.L2:
  addq %rax, %rdi
  movzwl %dx, %eax
  movzbl %r10b, %r10d
  subl $1, %eax
  addq %r10, %r9
  movl %eax, %r10d
  shrl %r10d
  orl %r10d, %eax
  movl %eax, %r10d
  shrl $2, %r10d
  orl %r10d, %eax
  movl %eax, %r10d
  shrl $4, %r10d
  orl %r10d, %eax
  movl %eax, %r10d
  shrl $8, %r10d
  orl %r10d, %eax
  movl %eax, %r10d
  shrl $16, %r10d
  orl %r10d, %eax
  addl $1, %eax
  addq %rax, %r8
  subl $1, %ecx
  jne .L15
  movq %rdi, %rdx
  movq %r9, %rcx
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  popq %rbx
  ret
