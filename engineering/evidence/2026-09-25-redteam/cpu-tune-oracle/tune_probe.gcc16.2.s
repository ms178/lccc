popcnt_sum:
  testq %rsi, %rsi
  je .L4
  leaq (%rdi,%rsi,8), %rcx
  xorl %eax, %eax
.L3:
  popcntq (%rdi), %rdx
  addq $8, %rdi
  addq %rdx, %rax
  cmpq %rdi, %rcx
  jne .L3
  ret
.L4:
  xorl %eax, %eax
  ret
ctz_of:
  tzcntq %rdi, %rax
  ret
clz_of:
  lzcntq %rdi, %rax
  ret
shl_var:
  shlx %rsi, %rdi, %rax
  ret
shr_var:
  shrx %rsi, %rdi, %rax
  ret
sar_var:
  sarx %rsi, %rdi, %rax
  ret
rot_hash:
  testq %rsi, %rsi
  je .L15
  leaq (%rdi,%rsi,4), %rcx
  xorl %eax, %eax
.L14:
  addq $4, %rdi
  shlx %edx, %eax, %eax
  xorl -4(%rdi), %eax
  cmpq %rcx, %rdi
  jne .L14
  ret
.L15:
  xorl %eax, %eax
  ret
copy_200:
  vmovdqu (%rsi), %ymm0
  vmovdqu %ymm0, (%rdi)
  vmovdqu 32(%rsi), %ymm0
  vmovdqu %ymm0, 32(%rdi)
  vmovdqu 64(%rsi), %ymm0
  vmovdqu %ymm0, 64(%rdi)
  vmovdqu 96(%rsi), %ymm0
  vmovdqu %ymm0, 96(%rdi)
  vmovdqu 128(%rsi), %ymm0
  vmovdqu %ymm0, 128(%rdi)
  vmovdqu 160(%rsi), %ymm0
  vmovdqu %ymm0, 160(%rdi)
  movq 192(%rsi), %rax
  movq %rax, 192(%rdi)
  vzeroupper
  ret
copy_2112:
  subq $8, %rsp
  movl $2112, %edx
  call memcpy
  addq $8, %rsp
  ret
copy_4096:
  subq $8, %rsp
  movl $4096, %edx
  call memcpy
  addq $8, %rsp
  ret
copy_8192:
  subq $8, %rsp
  movl $8192, %edx
  call memcpy
  addq $8, %rsp
  ret