zinit:
  pushq %rbx
  subq $4096, %rsp
  movq %rsp, %rbx
  movl $4096, %edx
  movq %rbx, %rdi
  xorl %esi, %esi
  callq memset@PLT
  movq %rbx, %rdi
  callq use@PLT
  addq $4096, %rsp
  popq %rbx
  retq

