main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $1845045837, %edi
  movq $-15309, %rsi
  movl $-5, %eax
  movl $4, %ecx
  movl $5, %edx
  imull $1664525, %edi, %edi
  addl $1013904223, %edi
  movl %edi, %r9d
  shrl $16, %r9d
  andl $15, %r9d
  movzwl %di, %r8d
  jmpq *.LJTI0_0(,%r9,8)
.LBB0_1:
  addl %ecx, %r8d
  jmp .LBB0_18
.LBB0_2:
  movl %ecx, %r10d
  subl %r8d, %r10d
  movq %rsi, %r9
  movl %r10d, %r8d
  jmp .LBB0_19
.LBB0_3:
  imull %ecx, %r8d
  jmp .LBB0_18
.LBB0_4:
  xorl %ecx, %r8d
  jmp .LBB0_18
.LBB0_5:
  orl %ecx, %r8d
  jmp .LBB0_18
.LBB0_6:
  andl %ecx, %r8d
  jmp .LBB0_18
.LBB0_7:
  movl %edi, %r8d
  andb $7, %r8b
  shlxl %r8d, %ecx, %r8d
  jmp .LBB0_18
.LBB0_8:
  movl %edi, %r8d
  andb $7, %r8b
  shrxl %r8d, %ecx, %r8d
  jmp .LBB0_18
.LBB0_9:
  addl %eax, %r8d
  jmp .LBB0_18
.LBB0_10:
  addl %edx, %r8d
  movslq %r8d, %r8
  addq %r8, %rsi
  jmp .LBB0_20
.LBB0_11:
  addl %ecx, %r8d
  leal (%r8,%r8,2), %r8d
  jmp .LBB0_18
.LBB0_12:
  movl %ecx, %r9d
  subl %r8d, %r9d
  leal (%r9,%r9,4), %r8d
  jmp .LBB0_18
.LBB0_13:
  xorl %ecx, %r8d
  incl %r8d
  jmp .LBB0_18
.LBB0_14:
  orl %ecx, %r8d
  decl %r8d
  jmp .LBB0_18
.LBB0_15:
  andl %ecx, %r8d
  addq %r8, %rsi
  addq $2, %rsi
  cmpl $49999998, %ecx
  jbe .LBB0_16
.LBB0_21:
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq
.LBB0_16:
  decl %eax
  incl %ecx
  incl %edx
  imull $1664525, %edi, %edi
  addl $1013904223, %edi
  movl %edi, %r9d
  shrl $16, %r9d
  andl $15, %r9d
  movzwl %di, %r8d
  jmpq *.LJTI0_0(,%r9,8)
.LBB0_17:
  addl %ecx, %r8d
  incl %r8d
.LBB0_18:
  movq %rsi, %r9
.LBB0_19:
  movslq %r8d, %rsi
  addq %r9, %rsi
.LBB0_20:
  cmpl $-50000000, %eax
  jne .LBB0_16
  jmp .LBB0_21
.LJTI0_0:
  .quad .LBB0_1
  .quad .LBB0_2
  .quad .LBB0_3
  .quad .LBB0_4
  .quad .LBB0_5
  .quad .LBB0_6
  .quad .LBB0_7
  .quad .LBB0_8
  .quad .LBB0_9
  .quad .LBB0_10
  .quad .LBB0_11
  .quad .LBB0_12
  .quad .LBB0_13
  .quad .LBB0_14
  .quad .LBB0_15
  .quad .LBB0_17

.L.str:
  .asciz "switch_dispatch sum: %ld\n"

