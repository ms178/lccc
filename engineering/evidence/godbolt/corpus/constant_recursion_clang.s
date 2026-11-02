main:
  pushq %rax
  movl $3, %edi
  movl $11, %esi
  callq ackermann
  movl %eax, 4(%rsp)
  movl 4(%rsp), %esi
  leaq .L.str(%rip), %rdi
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
  .asciz "constant ackermann: %d\n"

