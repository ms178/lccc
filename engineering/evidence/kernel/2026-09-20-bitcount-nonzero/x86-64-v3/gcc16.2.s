branch_index_store:
  testq %rdi, %rdi
  je .L26
  xorl %eax, %eax
.L21:
  xorl %ecx, %ecx
  movl %eax, %edx
  tzcntq %rdi, %rcx
  cmpl %ecx, %eax
  je .L20
  movl branch_slots(,%rcx,4), %edx
.L20:
  movl %edx, branch_slots(,%rax,4)
  addq $1, %rax
  blsr %rdi, %rdi
  jne .L21
.L26:
  ret
