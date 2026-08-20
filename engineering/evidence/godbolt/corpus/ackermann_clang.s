main:
  pushq %rax
  movl $3, %edi
  movl $11, %esi
  callq ackermann
  leaq .L.str(%rip), %rdi
  movl %eax, %esi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

ackermann:
  movl %esi, %eax
  testl %edi, %edi
  je .LBB1_6
  pushq %rbx
  movl %edi, %ebx
  jmp .LBB1_2
.LBB1_3:
  movl $1, %eax
  decl %ebx
  je .LBB1_5
.LBB1_2:
  testl %eax, %eax
  je .LBB1_3
  decl %eax
  movl %ebx, %edi
  movl %eax, %esi
  callq ackermann
  decl %ebx
  jne .LBB1_2
.LBB1_5:
  popq %rbx
.LBB1_6:
  incl %eax
  retq

.L.str:
  .asciz "ackermann(3,11) = %d\n"

