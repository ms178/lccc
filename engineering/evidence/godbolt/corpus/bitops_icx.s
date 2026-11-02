main:
  pushq %r14
  pushq %rbx
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $-559038737, %eax
  movl $50000000, %edi
  xorl %r8d, %r8d
  xorl %ecx, %ecx
  xorl %edx, %edx
  xorl %esi, %esi
  jmp .LBB0_1
.LBB0_3:
  movl %eax, %r9d
  shll $16, %r9d
  xorl %r10d, %r10d
  cmpl $65536, %eax
  setb %r10b
  cmovael %eax, %r9d
  shll $4, %r10d
  leal 8(%r10), %r11d
  movl %r9d, %ebx
  shll $8, %ebx
  cmpl $16777216, %r9d
  cmovael %r9d, %ebx
  cmovael %r10d, %r11d
  leal 4(%r11), %r10d
  movl %ebx, %r14d
  shll $4, %r14d
  cmpl $268435456, %ebx
  cmovael %ebx, %r14d
  cmovael %r11d, %r10d
  leal 2(%r10), %r11d
  leal (,%r14,4), %r9d
  cmpl $1073741824, %r14d
  cmovael %r14d, %r9d
  cmovael %r10d, %r11d
  notl %r9d
  shrl $31, %r9d
  addl %r11d, %r9d
.LBB0_4:
  movl %eax, %r10d
  shrl %r10d
  andl $1431655765, %r10d
  movl %eax, %r11d
  subl %r10d, %r11d
  movl %r11d, %r10d
  andl $858993459, %r10d
  shrl $2, %r11d
  andl $858993459, %r11d
  addl %r10d, %r11d
  movl %r11d, %r10d
  shrl $4, %r10d
  addl %r11d, %r10d
  andl $252645135, %r10d
  imull $16843009, %r10d, %r10d
  shrl $24, %r10d
  addq %r10, %rsi
  movl %r9d, %r9d
  addq %r9, %rdx
  movl %eax, %r9d
  bswapl %r9d
  movl %r9d, %r10d
  andl $252645135, %r10d
  shll $4, %r10d
  shrl $4, %r9d
  andl $252645135, %r9d
  orl %r10d, %r9d
  movl %r9d, %r10d
  andl $858993459, %r10d
  shrl $2, %r9d
  andl $858993459, %r9d
  leal (%r9,%r10,4), %r9d
  movl %r9d, %r10d
  andl $85, %r10d
  shrl %r9d
  andl $85, %r9d
  leal (%r9,%r10,2), %r9d
  addq %r9, %rcx
  movzwl %ax, %r9d
  decl %r9d
  movl %r9d, %r10d
  shrl %r10d
  orl %r9d, %r10d
  movl %r10d, %r9d
  shrl $2, %r9d
  orl %r10d, %r9d
  movl %r9d, %r10d
  shrl $4, %r10d
  orl %r9d, %r10d
  movl %r10d, %r9d
  shrl $8, %r9d
  orl %r10d, %r9d
  movl %r9d, %r10d
  shrl $16, %r10d
  orl %r9d, %r10d
  incl %r10d
  addq %r10, %r8
  decl %edi
  je .LBB0_5
.LBB0_1:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  jne .LBB0_3
  movl $32, %r9d
  jmp .LBB0_4
.LBB0_5:
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  retq

.L.str:
  .asciz "pop=%ld clz=%ld rev=%ld pow2=%ld\n"

