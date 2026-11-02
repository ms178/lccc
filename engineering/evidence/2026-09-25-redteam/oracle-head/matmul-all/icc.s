main:
..B1.1: # Preds ..B1.0
  pushq %rbp #17.16
  movq %rsp, %rbp #17.16
  andq $-128, %rsp #17.16
  subq $128, %rsp #17.16
  movl $3, %edi #17.16
  xorl %esi, %esi #17.16
  call __intel_new_feature_proc_init #17.16
..B1.15: # Preds ..B1.1
  stmxcsr 8(%rsp) #17.16
  xorl %edx, %edx #18.5
  xorl %eax, %eax #18.5
  orl $32832, 8(%rsp) #17.16
  ldmxcsr 8(%rsp) #17.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm2 #20.36
  movdqu .L_2il0floatpacket.1(%rip), %xmm1 #20.13
  movups .L_2il0floatpacket.2(%rip), %xmm3 #20.41
  movdqu .L_2il0floatpacket.3(%rip), %xmm4 #21.36
  movdqu .L_2il0floatpacket.4(%rip), %xmm6 #21.40
..B1.2: # Preds ..B1.4 ..B1.15
  movd %edx, %xmm9 #20.36
  lea 1(%rdx), %ecx #20.36
  movdqa %xmm9, %xmm5 #20.36
  lea 2(%rdx), %esi #20.36
  lea 3(%rdx), %edi #20.36
  movd %ecx, %xmm0 #20.36
  xorl %ecx, %ecx #19.9
  movd %esi, %xmm8 #20.36
  movd %edi, %xmm7 #20.36
  punpckldq %xmm0, %xmm5 #20.36
  movdqa %xmm1, %xmm0 #20.13
  punpckldq %xmm7, %xmm8 #20.36
  punpcklqdq %xmm8, %xmm5 #20.36
  pshufd $0, %xmm9, %xmm8 #18.28
  movdqa %xmm8, %xmm7 #21.36
  psrlq $32, %xmm7 #21.36
..B1.3: # Preds ..B1.3 ..B1.2
  movdqa %xmm0, %xmm12 #21.36
  movdqa %xmm8, %xmm13 #21.36
  psrlq $32, %xmm12 #21.36
  movdqa %xmm5, %xmm9 #20.36
  pmuludq %xmm0, %xmm13 #21.36
  paddd %xmm2, %xmm0 #20.13
  cvtdq2pd %xmm5, %xmm10 #20.36
  pmuludq %xmm7, %xmm12 #21.36
  mulpd %xmm3, %xmm10 #20.41
  pand %xmm4, %xmm13 #21.36
  psllq $32, %xmm12 #21.36
  por %xmm12, %xmm13 #21.36
  paddd %xmm6, %xmm13 #21.40
  punpckhqdq %xmm5, %xmm9 #20.36
  paddd %xmm2, %xmm5 #20.36
  cvtdq2pd %xmm13, %xmm14 #21.40
  cvtdq2pd %xmm9, %xmm11 #20.36
  mulpd %xmm3, %xmm14 #21.45
  mulpd %xmm3, %xmm11 #20.41
  punpckhqdq %xmm13, %xmm13 #21.40
  cvtdq2pd %xmm13, %xmm15 #21.40
  mulpd %xmm3, %xmm15 #21.45
  movups %xmm10, A(%rax,%rcx,8) #20.13
  movups %xmm11, 16+A(%rax,%rcx,8) #20.13
  movups %xmm14, B(%rax,%rcx,8) #21.13
  movups %xmm15, 16+B(%rax,%rcx,8) #21.13
  addq $4, %rcx #19.9
  cmpq $256, %rcx #19.9
  jb ..B1.3 # Prob 99% #19.9
..B1.4: # Preds ..B1.3
  incl %edx #18.5
  addq $2048, %rax #18.5
  cmpl $256, %edx #18.5
  jb ..B1.2 # Prob 99% #18.5
..B1.5: # Preds ..B1.4
  xorl %edx, %edx #11.5
  xorl %eax, %eax #11.5
..B1.6: # Preds ..B1.10 ..B1.5
  xorl %edi, %edi #12.9
  xorl %esi, %esi #12.9
  movq %rax, %rcx #14.28
..B1.7: # Preds ..B1.9 ..B1.6
  movsd A(%rcx), %xmm0 #14.28
  xorl %r8d, %r8d #13.13
  unpcklpd %xmm0, %xmm0 #14.28
..B1.8: # Preds ..B1.8 ..B1.7
  movups B(%rsi,%r8,8), %xmm1 #14.38
  movups 16+B(%rsi,%r8,8), %xmm2 #14.38
  movups 32+B(%rsi,%r8,8), %xmm3 #14.38
  movups 48+B(%rsi,%r8,8), %xmm4 #14.38
  mulpd %xmm0, %xmm1 #14.38
  mulpd %xmm0, %xmm2 #14.38
  mulpd %xmm0, %xmm3 #14.38
  mulpd %xmm0, %xmm4 #14.38
  addpd C(%rax,%r8,8), %xmm1 #14.17
  addpd 16+C(%rax,%r8,8), %xmm2 #14.17
  addpd 32+C(%rax,%r8,8), %xmm3 #14.17
  addpd 48+C(%rax,%r8,8), %xmm4 #14.17
  movups %xmm1, C(%rax,%r8,8) #14.17
  movups %xmm2, 16+C(%rax,%r8,8) #14.17
  movups %xmm3, 32+C(%rax,%r8,8) #14.17
  movups %xmm4, 48+C(%rax,%r8,8) #14.17
  addq $8, %r8 #13.13
  cmpq $256, %r8 #13.13
  jb ..B1.8 # Prob 99% #13.13
..B1.9: # Preds ..B1.8
  incl %edi #12.9
  addq $2048, %rsi #12.9
  addq $8, %rcx #12.9
  cmpl $256, %edi #12.9
  jb ..B1.7 # Prob 99% #12.9
..B1.10: # Preds ..B1.9
  incl %edx #11.5
  addq $2048, %rax #11.5
  cmpl $256, %edx #11.5
  jb ..B1.6 # Prob 99% #11.5
..B1.11: # Preds ..B1.10
  movq 263168+C(%rip), %rax #24.30
  movl $.L_2__STRING.0, %edi #25.5
  movq %rax, (%rsp) #24.28
  movl $1, %eax #25.5
  movsd (%rsp), %xmm0 #25.43
  call printf #25.5
..B1.12: # Preds ..B1.11
  xorl %eax, %eax #26.12
  movq %rbp, %rsp #26.12
  popq %rbp #26.12
  ret #26.12
matmul:
..B2.1: # Preds ..B2.0
  xorl %edx, %edx #11.5
  xorl %eax, %eax #11.5
..B2.2: # Preds ..B2.6 ..B2.1
  xorl %edi, %edi #12.9
  xorl %esi, %esi #12.9
  movq %rax, %rcx #14.28
..B2.3: # Preds ..B2.5 ..B2.2
  movsd A(%rcx), %xmm0 #14.28
  xorl %r8d, %r8d #13.13
  unpcklpd %xmm0, %xmm0 #14.28
..B2.4: # Preds ..B2.4 ..B2.3
  movups B(%rsi,%r8,8), %xmm1 #14.38
  movups 16+B(%rsi,%r8,8), %xmm2 #14.38
  movups 32+B(%rsi,%r8,8), %xmm3 #14.38
  movups 48+B(%rsi,%r8,8), %xmm4 #14.38
  mulpd %xmm0, %xmm1 #14.38
  mulpd %xmm0, %xmm2 #14.38
  mulpd %xmm0, %xmm3 #14.38
  mulpd %xmm0, %xmm4 #14.38
  addpd C(%rax,%r8,8), %xmm1 #14.17
  addpd 16+C(%rax,%r8,8), %xmm2 #14.17
  addpd 32+C(%rax,%r8,8), %xmm3 #14.17
  addpd 48+C(%rax,%r8,8), %xmm4 #14.17
  movups %xmm1, C(%rax,%r8,8) #14.17
  movups %xmm2, 16+C(%rax,%r8,8) #14.17
  movups %xmm3, 32+C(%rax,%r8,8) #14.17
  movups %xmm4, 48+C(%rax,%r8,8) #14.17
  addq $8, %r8 #13.13
  cmpq $256, %r8 #13.13
  jb ..B2.4 # Prob 99% #13.13
..B2.5: # Preds ..B2.4
  incl %edi #12.9
  addq $2048, %rsi #12.9
  addq $8, %rcx #12.9
  cmpl $256, %edi #12.9
  jb ..B2.3 # Prob 99% #12.9
..B2.6: # Preds ..B2.5
  incl %edx #11.5
  addq $2048, %rax #11.5
  cmpl $256, %edx #11.5
  jb ..B2.2 # Prob 99% #11.5
..B2.7: # Preds ..B2.6
  ret #15.1
A:
B:
C:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2__STRING.0:
