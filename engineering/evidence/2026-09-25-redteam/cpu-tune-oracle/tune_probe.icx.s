.LCPI0_2:
  .byte 15
.LCPI0_3:
  .byte 0
  .byte 1
  .byte 1
  .byte 2
  .byte 1
  .byte 2
  .byte 2
  .byte 3
  .byte 1
  .byte 2
  .byte 2
  .byte 3
  .byte 2
  .byte 3
  .byte 3
  .byte 4
popcnt_sum:
  testq %rsi, %rsi
  je .LBB0_1
  movq %rsi, %rcx
  andq $-4, %rcx
  je .LBB0_4
  leaq -1(%rcx), %rax
  vpxor %xmm0, %xmm0, %xmm0
  xorl %edx, %edx
  vpbroadcastb .LCPI0_2(%rip), %ymm2
  vbroadcasti128 .LCPI0_3(%rip), %ymm3
  vpxor %xmm1, %xmm1, %xmm1
.LBB0_6:
  vmovdqu (%rdi,%rdx,8), %ymm4
  vpand %ymm2, %ymm4, %ymm5
  vpshufb %ymm5, %ymm3, %ymm5
  vpsrlw $4, %ymm4, %ymm4
  vpand %ymm2, %ymm4, %ymm4
  vpshufb %ymm4, %ymm3, %ymm4
  vpaddb %ymm5, %ymm4, %ymm4
  vpsadbw %ymm0, %ymm4, %ymm4
  vpaddq %ymm1, %ymm4, %ymm1
  addq $4, %rdx
  cmpq %rax, %rdx
  jbe .LBB0_6
  vextracti128 $1, %ymm1, %xmm0
  vpaddq %xmm0, %xmm1, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rax
  cmpq %rsi, %rcx
  jne .LBB0_8
.LBB0_2:
  vzeroupper
  retq
.LBB0_1:
  xorl %eax, %eax
  retq
.LBB0_4:
  xorl %ecx, %ecx
  xorl %eax, %eax
.LBB0_8:
  popcntq (%rdi,%rcx,8), %rdx
  addq %rdx, %rax
  incq %rcx
  cmpq %rcx, %rsi
  jne .LBB0_8
  jmp .LBB0_2

ctz_of:
  tzcntq %rdi, %rax
  retq

clz_of:
  lzcntq %rdi, %rax
  retq

shl_var:
  shlxq %rsi, %rdi, %rax
  retq

shr_var:
  shrxq %rsi, %rdi, %rax
  retq

sar_var:
  sarxq %rsi, %rdi, %rax
  retq

rot_hash:
  xorl %eax, %eax
  testq %rsi, %rsi
  je .LBB6_6
  cmpq $8, %rsi
  jb .LBB6_4
  movq %rsi, %rcx
  shrq $3, %rcx
  leaq 28(%rdi), %r8
  xorl %eax, %eax
.LBB6_3:
  shlxl %edx, %eax, %eax
  xorl -28(%r8), %eax
  shlxl %edx, %eax, %eax
  xorl -24(%r8), %eax
  shlxl %edx, %eax, %eax
  xorl -20(%r8), %eax
  shlxl %edx, %eax, %eax
  xorl -16(%r8), %eax
  shlxl %edx, %eax, %eax
  xorl -12(%r8), %eax
  shlxl %edx, %eax, %eax
  xorl -8(%r8), %eax
  shlxl %edx, %eax, %eax
  xorl -4(%r8), %eax
  shlxl %edx, %eax, %eax
  xorl (%r8), %eax
  addq $32, %r8
  decq %rcx
  jne .LBB6_3
.LBB6_4:
  movq %rsi, %rcx
  andq $-8, %rcx
  cmpq %rsi, %rcx
  jae .LBB6_6
.LBB6_5:
  shlxl %edx, %eax, %eax
  xorl (%rdi,%rcx,4), %eax
  incq %rcx
  cmpq %rcx, %rsi
  jne .LBB6_5
.LBB6_6:
  retq

copy_200:
  movq 192(%rsi), %rax
  movq %rax, 192(%rdi)
  vmovups 160(%rsi), %ymm0
  vmovups %ymm0, 160(%rdi)
  vmovups 128(%rsi), %ymm0
  vmovups %ymm0, 128(%rdi)
  vmovups (%rsi), %ymm0
  vmovups 32(%rsi), %ymm1
  vmovups 64(%rsi), %ymm2
  vmovups 96(%rsi), %ymm3
  vmovups %ymm3, 96(%rdi)
  vmovups %ymm2, 64(%rdi)
  vmovups %ymm1, 32(%rdi)
  vmovups %ymm0, (%rdi)
  vzeroupper
  retq

copy_2112:
  movl $2112, %edx
  jmp _intel_fast_memcpy@PLT

copy_4096:
  movl $4096, %edx
  jmp _intel_fast_memcpy@PLT

copy_8192:
  movl $8192, %edx
  jmp _intel_fast_memcpy@PLT
