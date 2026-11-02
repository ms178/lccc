main:
  pushq %rax
  movl $777, %ecx
  xorl %eax, %eax
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  movl %ecx, %esi
  shrl $16, %esi
  andl $15, %esi
  movzwl %cx, %edi
  leaq .LJTI0_0(%rip), %rdx
  movslq (%rdx,%rsi,4), %r8
  addq %rdx, %r8
  xorl %esi, %esi
  jmpq *%r8
.LBB0_1:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  movl %ecx, %r8d
  shrl $16, %r8d
  andl $15, %r8d
  movzwl %cx, %edi
  movslq (%rdx,%r8,4), %r8
  addq %rdx, %r8
  jmpq *%r8
.LBB0_16:
  addl %eax, %edi
  incl %edi
  jmp .LBB0_17
.LBB0_14:
  orl %eax, %edi
  decl %edi
  jmp .LBB0_17
.LBB0_7:
  andl %eax, %edi
  jmp .LBB0_17
.LBB0_15:
  andl %eax, %edi
  addl $2, %edi
  jmp .LBB0_17
.LBB0_12:
  movl %eax, %r8d
  subl %edi, %r8d
  leal (%r8,%r8,4), %edi
  jmp .LBB0_17
.LBB0_5:
  xorl %eax, %edi
  jmp .LBB0_17
.LBB0_6:
  orl %eax, %edi
  jmp .LBB0_17
.LBB0_2:
  addl %eax, %edi
  jmp .LBB0_17
.LBB0_10:
  movl %eax, %r8d
  notl %r8d
  addl %r8d, %edi
  jmp .LBB0_17
.LBB0_3:
  movl %eax, %r9d
  subl %edi, %r9d
  movq %rsi, %r8
  movl %r9d, %edi
  jmp .LBB0_18
.LBB0_4:
  imull %eax, %edi
  jmp .LBB0_17
.LBB0_8:
  movl %ecx, %edi
  andb $7, %dil
  shlxl %edi, %eax, %edi
  jmp .LBB0_17
.LBB0_9:
  movl %ecx, %edi
  andb $7, %dil
  shrxl %edi, %eax, %edi
  jmp .LBB0_17
.LBB0_13:
  xorl %eax, %edi
  incl %edi
  jmp .LBB0_17
.LBB0_11:
  addl %eax, %edi
  leal (%rdi,%rdi,2), %edi
.LBB0_17:
  movq %rsi, %r8
.LBB0_18:
  movslq %edi, %rsi
  addq %r8, %rsi
  incl %eax
  cmpl $50000000, %eax
  jne .LBB0_1
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq
.LJTI0_0:
  .long .LBB0_2-.LJTI0_0
  .long .LBB0_3-.LJTI0_0
  .long .LBB0_4-.LJTI0_0
  .long .LBB0_5-.LJTI0_0
  .long .LBB0_6-.LJTI0_0
  .long .LBB0_7-.LJTI0_0
  .long .LBB0_8-.LJTI0_0
  .long .LBB0_9-.LJTI0_0
  .long .LBB0_10-.LJTI0_0
  .long .LBB0_16-.LJTI0_0
  .long .LBB0_11-.LJTI0_0
  .long .LBB0_12-.LJTI0_0
  .long .LBB0_13-.LJTI0_0
  .long .LBB0_14-.LJTI0_0
  .long .LBB0_15-.LJTI0_0
  .long .LBB0_16-.LJTI0_0

.L.str:
  .asciz "switch_dispatch sum: %ld\n"

