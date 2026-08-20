main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $3, %edi
  movl $11, %esi
  callq ackermann
  movl $.L.str, %edi
  movl %eax, %esi
  xorl %eax, %eax
  callq printf
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

