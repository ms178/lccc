branch_index_store:
  testq %rdi, %rdi
  je .LBB4_6
  leaq branch_slots(%rip), %rax
  xorl %ecx, %ecx
  movq %rax, %rdx
  jmp .LBB4_2
.LBB4_3:
  movl %ecx, %esi
.LBB4_5:
  movl %esi, (%rdx)
  incq %rcx
  addq $4, %rdx
  blsrq %rdi, %rdi
  je .LBB4_6
.LBB4_2:
  xorl %esi, %esi
  tzcntq %rdi, %rsi
  cmpq %rsi, %rcx
  je .LBB4_3
  movl (%rax,%rsi,4), %esi
  jmp .LBB4_5
.LBB4_6:
  retq
