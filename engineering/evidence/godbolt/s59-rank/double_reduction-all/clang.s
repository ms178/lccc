main:
  movl $1, %esi
  xorl %edi, %edi
  leaq a(%rip), %rax
  leaq b(%rip), %rcx
  leaq c(%rip), %rdx
.LBB0_1:
  imull $1664525, %esi, %r8d
  addl $1013904223, %r8d
  shrl $16, %r8d
  andl $15, %r8d
  addl $-7, %r8d
  movl %r8d, (%rdi,%rax)
  imull $389569705, %esi, %r8d
  addl $1196435762, %r8d
  shrl $16, %r8d
  andl $15, %r8d
  addl $-7, %r8d
  movl %r8d, (%rdi,%rcx)
  imull $-1354167659, %esi, %esi
  addl $-775096599, %esi
  movl %esi, %r8d
  shrl $16, %r8d
  andl $15, %r8d
  addl $-7, %r8d
  movl %r8d, (%rdi,%rdx)
  addq $4, %rdi
  cmpq $4194304, %rdi
  jne .LBB0_1
  xorl %esi, %esi
  xorl %edi, %edi
.LBB0_3:
  vpxor %xmm0, %xmm0, %xmm0
  xorl %r8d, %r8d
  vpxor %xmm1, %xmm1, %xmm1
  vpxor %xmm2, %xmm2, %xmm2
  vpxor %xmm3, %xmm3, %xmm3
  vpxor %xmm4, %xmm4, %xmm4
  vpxor %xmm5, %xmm5, %xmm5
  vpxor %xmm6, %xmm6, %xmm6
  vpxor %xmm7, %xmm7, %xmm7
.LBB0_4:
  vmovdqu (%rax,%r8,4), %ymm8
  vmovdqu 32(%rax,%r8,4), %ymm9
  vmovdqu 64(%rax,%r8,4), %ymm10
  vpmulld (%rcx,%r8,4), %ymm8, %ymm11
  vmovdqu 96(%rax,%r8,4), %ymm12
  vpmulld 32(%rcx,%r8,4), %ymm9, %ymm13
  vpaddd %ymm4, %ymm11, %ymm4
  vpmulld 64(%rcx,%r8,4), %ymm10, %ymm11
  vpaddd %ymm5, %ymm13, %ymm5
  vpmulld 96(%rcx,%r8,4), %ymm12, %ymm13
  vpaddd %ymm6, %ymm11, %ymm6
  vpmulld (%rdx,%r8,4), %ymm8, %ymm8
  vpaddd %ymm7, %ymm13, %ymm7
  vpmulld 32(%rdx,%r8,4), %ymm9, %ymm9
  vpaddd %ymm0, %ymm8, %ymm0
  vpmulld 64(%rdx,%r8,4), %ymm10, %ymm8
  vpaddd %ymm1, %ymm9, %ymm1
  vpmulld 96(%rdx,%r8,4), %ymm12, %ymm9
  vpaddd %ymm2, %ymm8, %ymm2
  vpaddd %ymm3, %ymm9, %ymm3
  addq $32, %r8
  cmpq $1048576, %r8
  jne .LBB0_4
  vpaddd %ymm0, %ymm1, %ymm0
  vpaddd %ymm0, %ymm2, %ymm0
  vpaddd %ymm0, %ymm3, %ymm0
  vpaddd %ymm4, %ymm5, %ymm1
  vpaddd %ymm1, %ymm6, %ymm1
  vpaddd %ymm1, %ymm7, %ymm1
  vpxor %xmm2, %xmm2, %xmm2
  xorl %r8d, %r8d
  vpxor %xmm3, %xmm3, %xmm3
  vpxor %xmm4, %xmm4, %xmm4
  vpxor %xmm5, %xmm5, %xmm5
  vpxor %xmm6, %xmm6, %xmm6
  vpxor %xmm7, %xmm7, %xmm7
  vpxor %xmm8, %xmm8, %xmm8
  vpxor %xmm9, %xmm9, %xmm9
.LBB0_6:
  vpaddd (%rax,%r8,4), %ymm6, %ymm6
  vpaddd 32(%rax,%r8,4), %ymm7, %ymm7
  vpaddd 64(%rax,%r8,4), %ymm8, %ymm8
  vpaddd 96(%rax,%r8,4), %ymm9, %ymm9
  vpaddd (%rcx,%r8,4), %ymm2, %ymm2
  vpaddd 32(%rcx,%r8,4), %ymm3, %ymm3
  vpaddd 64(%rcx,%r8,4), %ymm4, %ymm4
  vpaddd 96(%rcx,%r8,4), %ymm5, %ymm5
  vpaddd 128(%rax,%r8,4), %ymm6, %ymm6
  vpaddd 160(%rax,%r8,4), %ymm7, %ymm7
  vpaddd 192(%rax,%r8,4), %ymm8, %ymm8
  vpaddd 224(%rax,%r8,4), %ymm9, %ymm9
  vpaddd 128(%rcx,%r8,4), %ymm2, %ymm2
  vpaddd 160(%rcx,%r8,4), %ymm3, %ymm3
  vpaddd 192(%rcx,%r8,4), %ymm4, %ymm4
  vpaddd 224(%rcx,%r8,4), %ymm5, %ymm5
  addq $64, %r8
  cmpq $1048576, %r8
  jne .LBB0_6
  vpaddd %ymm6, %ymm7, %ymm6
  vpaddd %ymm8, %ymm9, %ymm7
  vpaddd %ymm6, %ymm7, %ymm6
  vpaddd %ymm2, %ymm3, %ymm2
  vpaddd %ymm4, %ymm5, %ymm3
  vpaddd %ymm2, %ymm3, %ymm2
  vpaddd %ymm6, %ymm2, %ymm2
  vpaddd %ymm1, %ymm0, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %r8d
  movslq %r8d, %r8
  movq %rsi, %r9
  addq %r8, %r9
  vextracti128 $1, %ymm2, %xmm0
  vpaddd %xmm0, %xmm2, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %esi
  movslq %esi, %rsi
  addq %r9, %rsi
  incl %edi
  cmpl $128, %edi
  jne .LBB0_3
  pushq %rax
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

