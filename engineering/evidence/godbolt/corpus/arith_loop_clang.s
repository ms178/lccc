.LCPI0_0:
  .long 24
  .long 25
  .long 26
  .long 27
  .long 28
  .long 29
  .long 30
  .long 31
.LCPI0_1:
  .long 16
  .long 17
  .long 18
  .long 19
  .long 20
  .long 21
  .long 22
  .long 23
.LCPI0_2:
  .long 25
  .long 26
  .long 27
  .long 28
  .long 29
  .long 30
  .long 31
  .long 32
.LCPI0_3:
  .long 17
  .long 18
  .long 19
  .long 20
  .long 21
  .long 22
  .long 23
  .long 24
.LCPI0_4:
  .long 8
  .long 9
  .long 10
  .long 11
  .long 12
  .long 13
  .long 14
  .long 15
.LCPI0_5:
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
  .zero 4
arith_loop:
  testl %edi, %edi
  jle .LBB0_1
  pushq %rbx
  vmovdqa .LCPI0_0(%rip), %ymm1
  vmovdqa .LCPI0_1(%rip), %ymm2
  vmovdqa .LCPI0_2(%rip), %ymm3
  vmovdqa .LCPI0_3(%rip), %ymm4
  vmovdqa .LCPI0_4(%rip), %ymm0
  movl $1, %ecx
  movl $2, %r9d
  movl $3, %r10d
  movl $4, %r8d
  movl $5, %esi
  movl $6, %edx
  movl $7, %eax
  vmovdqa .LCPI0_5(%rip), %ymm5
.LBB0_5:
  movl %r9d, %r11d
  movl %r10d, %ebx
  movl %r8d, %r9d
  movl %esi, %r10d
  movl %edx, %r8d
  movl %eax, %esi
  vpextrd $1, %xmm0, %eax
  vmovd %xmm0, %edx
  imull %edx, %eax
  imull %esi, %edx
  addl %esi, %eax
  imull %r8d, %esi
  addl %r8d, %edx
  imull %r10d, %r8d
  addl %r10d, %esi
  imull %r9d, %r10d
  addl %r9d, %r8d
  imull %ebx, %r9d
  addl %ebx, %r10d
  imull %r11d, %ebx
  addl %ebx, %ecx
  vpbroadcastd %xmm4, %xmm6
  vpbroadcastd %xmm2, %ymm7
  vpunpckldq %xmm6, %xmm7, %xmm6
  vinserti128 $1, %xmm6, %ymm0, %ymm6
  vpermq $249, %ymm0, %ymm8
  vpblendd $192, %ymm6, %ymm8, %ymm6
  vpermd %ymm0, %ymm5, %ymm8
  vpblendd $128, %ymm7, %ymm8, %ymm7
  vpmulld %ymm7, %ymm6, %ymm6
  addl %r11d, %r9d
  vpaddd %ymm0, %ymm6, %ymm0
  vperm2i128 $33, %ymm3, %ymm4, %ymm6
  vpalignr $4, %ymm4, %ymm6, %ymm6
  vpermd %ymm3, %ymm5, %ymm7
  vmovd %ecx, %xmm8
  vpbroadcastd %xmm8, %ymm8
  vpblendd $128, %ymm8, %ymm7, %ymm7
  vpmulld %ymm7, %ymm3, %ymm9
  vpmulld %ymm6, %ymm4, %ymm10
  vpaddd %ymm1, %ymm9, %ymm1
  vpblendd $3, %ymm3, %ymm4, %ymm9
  vpermq $57, %ymm9, %ymm9
  vpermq $249, %ymm3, %ymm11
  vpblendd $64, %ymm8, %ymm11, %ymm8
  vmovd %r9d, %xmm11
  vpbroadcastd %xmm11, %ymm11
  vpblendd $128, %ymm11, %ymm8, %ymm8
  vpmulld %ymm8, %ymm7, %ymm7
  vpmulld %ymm9, %ymm6, %ymm6
  vpaddd %ymm2, %ymm10, %ymm2
  vpaddd %ymm7, %ymm3, %ymm3
  vpaddd %ymm6, %ymm4, %ymm4
  decl %edi
  jne .LBB0_5
  vextracti128 $1, %ymm3, %xmm3
  vpextrd $3, %xmm3, %edi
  vpxor %ymm1, %ymm2, %ymm1
  vextracti128 $1, %ymm1, %xmm2
  vpxor %xmm2, %xmm1, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpxor %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpxor %xmm2, %xmm1, %xmm1
  vmovd %xmm1, %r11d
  vextracti128 $1, %ymm0, %xmm1
  vpxor %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpxor %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpxor %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %ebx
  xorl %edi, %ebx
  xorl %r11d, %ebx
  xorl %edx, %eax
  xorl %esi, %eax
  xorl %r8d, %eax
  xorl %r10d, %eax
  xorl %ecx, %eax
  xorl %r9d, %eax
  xorl %ebx, %eax
  popq %rbx
  vzeroupper
  retq
.LBB0_1:
  movl $32, %eax
  retq

main:
  pushq %rax
  movl $10000000, %edi
  callq arith_loop
  movl %eax, 4(%rsp)
  movl 4(%rsp), %esi
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "arith_loop result: %d\n"

