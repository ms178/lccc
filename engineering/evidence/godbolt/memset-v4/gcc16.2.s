zinit:
  subq $4104, %rsp
  movl $4096, %edx
  movl $.LC0, %esi
  movq %rsp, %rdi
  call memcpy
  movq %rsp, %rdi
  call use
  addq $4104, %rsp
  ret
