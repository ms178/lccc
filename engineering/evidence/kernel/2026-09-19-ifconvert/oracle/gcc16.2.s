branch_index_store:
  testq %rdi, %rdi
  je .L16
  xorl %eax, %eax
.L19:
  xorl %edx, %edx
  movl %eax, %ecx
  rep bsfq %rdi, %rdx
  cmpl %edx, %eax
  je .L18
  movl %edx, %edx
  movl branch_slots(,%rdx,4), %ecx
.L18:
  leaq -1(%rdi), %rdx
  movl %ecx, branch_slots(,%rax,4)
  addq $1, %rax
  andq %rdx, %rdi
  jne .L19
.L16:
  ret
.LC14:
  .string "fail=%ld\n"
