branch_index_store:
  testq %rdi, %rdi
  je .LBB4_6
  xorl %eax, %eax
  jmp .LBB4_2
.LBB4_4:
  movl branch_slots(,%rcx,4), %ecx
.LBB4_5:
  movl %ecx, branch_slots(,%rax,4)
  incq %rax
  blsrq %rdi, %rdi
  je .LBB4_6
.LBB4_2:
  xorl %ecx, %ecx
  tzcntq %rdi, %rcx
  cmpq %rcx, %rax
  jne .LBB4_4
  movl %eax, %ecx
  jmp .LBB4_5
.LBB4_6:
  retq
