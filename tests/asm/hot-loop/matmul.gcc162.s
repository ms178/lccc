matmul:
  xorl %edi, %edi
.L2:
  leaq A(%rdi), %rsi
  movl $B, %ecx
  leaq C(%rdi), %rdx
.L6:
  vbroadcastsd (%rsi), %ymm1
  xorl %eax, %eax
.L3:
  vmovapd (%rcx,%rax), %ymm0
  vfmadd213pd (%rdx,%rax), %ymm1, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  addq $32, %rax
  cmpq $2048, %rax
  jne .L3
  addq $2048, %rcx
  addq $8, %rsi
  cmpq $B+524288, %rcx
  jne .L6
  addq $2048, %rdi
  cmpq $524288, %rdi
  jne .L2
  vzeroupper
  ret
.LC5:
  .string "matmul C[128][128] = %.4f\n"
