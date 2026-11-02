.LCPI0_2:
  .zero 4,15
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
  cmpq $4, %rsi
  jae .LBB0_5
  xorl %ecx, %ecx
  xorl %eax, %eax
  jmp .LBB0_4
.LBB0_1:
  xorl %eax, %eax
  retq
.LBB0_5:
  cmpq $16, %rsi
  jae .LBB0_7
  xorl %ecx, %ecx
  xorl %eax, %eax
  jmp .LBB0_11
.LBB0_7:
  movq %rsi, %rcx
  andq $-16, %rcx
  vpxor %xmm0, %xmm0, %xmm0
  xorl %eax, %eax
  vpbroadcastd .LCPI0_2(%rip), %ymm1
  vbroadcasti128 .LCPI0_3(%rip), %ymm2
  vpxor %xmm3, %xmm3, %xmm3
  vpxor %xmm4, %xmm4, %xmm4
  vpxor %xmm5, %xmm5, %xmm5
  vpxor %xmm6, %xmm6, %xmm6
.LBB0_8:
  vmovdqu (%rdi,%rax,8), %ymm7
  vmovdqu 32(%rdi,%rax,8), %ymm8
  vmovdqu 64(%rdi,%rax,8), %ymm9
  vmovdqu 96(%rdi,%rax,8), %ymm10
  vpand %ymm1, %ymm7, %ymm11
  vpshufb %ymm11, %ymm2, %ymm11
  vpsrlw $4, %ymm7, %ymm7
  vpand %ymm1, %ymm7, %ymm7
  vpshufb %ymm7, %ymm2, %ymm7
  vpaddb %ymm7, %ymm11, %ymm7
  vpsadbw %ymm0, %ymm7, %ymm7
  vpaddq %ymm3, %ymm7, %ymm3
  vpand %ymm1, %ymm8, %ymm7
  vpshufb %ymm7, %ymm2, %ymm7
  vpsrlw $4, %ymm8, %ymm8
  vpand %ymm1, %ymm8, %ymm8
  vpshufb %ymm8, %ymm2, %ymm8
  vpaddb %ymm7, %ymm8, %ymm7
  vpsadbw %ymm0, %ymm7, %ymm7
  vpaddq %ymm4, %ymm7, %ymm4
  vpand %ymm1, %ymm9, %ymm7
  vpshufb %ymm7, %ymm2, %ymm7
  vpsrlw $4, %ymm9, %ymm8
  vpand %ymm1, %ymm8, %ymm8
  vpshufb %ymm8, %ymm2, %ymm8
  vpaddb %ymm7, %ymm8, %ymm7
  vpsadbw %ymm0, %ymm7, %ymm7
  vpaddq %ymm5, %ymm7, %ymm5
  vpand %ymm1, %ymm10, %ymm7
  vpshufb %ymm7, %ymm2, %ymm7
  vpsrlw $4, %ymm10, %ymm8
  vpand %ymm1, %ymm8, %ymm8
  vpshufb %ymm8, %ymm2, %ymm8
  vpaddb %ymm7, %ymm8, %ymm7
  vpsadbw %ymm0, %ymm7, %ymm7
  vpaddq %ymm6, %ymm7, %ymm6
  addq $16, %rax
  cmpq %rax, %rcx
  jne .LBB0_8
  vpaddq %ymm3, %ymm4, %ymm0
  vpaddq %ymm0, %ymm5, %ymm0
  vpaddq %ymm0, %ymm6, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rax
  cmpq %rcx, %rsi
  je .LBB0_15
  testb $12, %sil
  je .LBB0_4
.LBB0_11:
  movq %rcx, %rdx
  movq %rsi, %rcx
  andq $-4, %rcx
  vmovq %rax, %xmm0
  vpbroadcastd .LCPI0_2(%rip), %ymm1
  vbroadcasti128 .LCPI0_3(%rip), %ymm2
  vpxor %xmm3, %xmm3, %xmm3
.LBB0_12:
  vmovdqu (%rdi,%rdx,8), %ymm4
  vpand %ymm1, %ymm4, %ymm5
  vpshufb %ymm5, %ymm2, %ymm5
  vpsrlw $4, %ymm4, %ymm4
  vpand %ymm1, %ymm4, %ymm4
  vpshufb %ymm4, %ymm2, %ymm4
  vpaddb %ymm5, %ymm4, %ymm4
  vpsadbw %ymm3, %ymm4, %ymm4
  vpaddq %ymm0, %ymm4, %ymm0
  addq $4, %rdx
  cmpq %rdx, %rcx
  jne .LBB0_12
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rax
  jmp .LBB0_14
.LBB0_4:
  popcntq (%rdi,%rcx,8), %rdx
  addq %rdx, %rax
  incq %rcx
.LBB0_14:
  cmpq %rcx, %rsi
  jne .LBB0_4
.LBB0_15:
  vzeroupper
  retq

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
  testq %rsi, %rsi
  je .LBB6_1
  movl %esi, %ecx
  andl $7, %ecx
  cmpq $8, %rsi
  jae .LBB6_8
  xorl %r8d, %r8d
  xorl %eax, %eax
  jmp .LBB6_5
.LBB6_1:
  xorl %eax, %eax
  retq
.LBB6_8:
  andq $-8, %rsi
  xorl %r8d, %r8d
  xorl %eax, %eax
.LBB6_9:
  shlxl %edx, %eax, %eax
  xorl (%rdi,%r8,4), %eax
  shlxl %edx, %eax, %eax
  xorl 4(%rdi,%r8,4), %eax
  shlxl %edx, %eax, %eax
  xorl 8(%rdi,%r8,4), %eax
  shlxl %edx, %eax, %eax
  xorl 12(%rdi,%r8,4), %eax
  shlxl %edx, %eax, %eax
  xorl 16(%rdi,%r8,4), %eax
  shlxl %edx, %eax, %eax
  xorl 20(%rdi,%r8,4), %eax
  shlxl %edx, %eax, %eax
  xorl 24(%rdi,%r8,4), %eax
  shlxl %edx, %eax, %eax
  xorl 28(%rdi,%r8,4), %eax
  addq $8, %r8
  cmpq %r8, %rsi
  jne .LBB6_9
  testq %rcx, %rcx
  je .LBB6_7
.LBB6_5:
  leaq (%rdi,%r8,4), %rsi
  xorl %edi, %edi
.LBB6_6:
  shlxl %edx, %eax, %eax
  xorl (%rsi,%rdi,4), %eax
  incq %rdi
  cmpq %rdi, %rcx
  jne .LBB6_6
.LBB6_7:
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
  jmp memcpy@PLT

copy_4096:
  movl $4096, %edx
  jmp memcpy@PLT

copy_8192:
  movl $8192, %edx
  jmp memcpy@PLT
