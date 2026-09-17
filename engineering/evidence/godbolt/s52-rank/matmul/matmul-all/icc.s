main:
..B1.1: # Preds ..B1.0
  pushq %rbp #17.16
  movq %rsp, %rbp #17.16
  andq $-128, %rsp #17.16
  subq $256, %rsp #17.16
  movl $3, %edi #17.16
  xorl %esi, %esi #17.16
  call __intel_new_feature_proc_init #17.16
..B1.28: # Preds ..B1.1
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
..B1.2: # Preds ..B1.4 ..B1.28
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
  movq %r12, 8(%rsp) #11.5[spill]
  xorl %edx, %edx #11.5
  movq %r13, 16(%rsp) #11.5[spill]
  movl $128, %eax #11.5
  movq %r14, 24(%rsp) #11.5[spill]
  movq %r15, 32(%rsp) #11.5[spill]
  movq %rbx, 40(%rsp) #11.5[spill]
..B1.6: # Preds ..B1.22 ..B1.5
  movl %edx, %ebx #11.5
  xorl %ecx, %ecx #11.5
  shll $7, %ebx #11.5
  negl %ebx #11.16
  addl $256, %ebx #11.16
  cmpl $128, %ebx #11.5
  movl %edx, %r11d #14.28
  cmovae %eax, %ebx #11.5
  shlq $18, %r11 #14.28
  movq %rbx, 96(%rsp) #11.5[spill]
  movl %edx, 112(%rsp) #11.5[spill]
..B1.7: # Preds ..B1.21 ..B1.6
  movl %ecx, %ebx #12.9
  xorl %edx, %edx #11.5
  shll $7, %ebx #12.9
  negl %ebx #12.20
  addl $256, %ebx #12.20
  cmpl $128, %ebx #12.9
  movl %ecx, %r12d #14.28
  movq %r12, %r14 #14.28
  cmovae %eax, %ebx #12.9
  shlq $10, %r14 #14.28
  addq %r11, %r14 #14.28
  shlq $18, %r12 #14.38
  movq %rbx, 136(%rsp) #12.9[spill]
..B1.8: # Preds ..B1.20 ..B1.7
  movl %edx, %edi #13.13
  movq %rdx, %r10 #14.17
  shll $7, %edi #13.13
  xorl %r8d, %r8d #11.5
  movl %edi, %esi #13.24
  xorl %r9d, %r9d #11.5
  negl %esi #13.24
  addl $256, %esi #13.24
  cmpl $128, %esi #13.13
  movl %edi, 128(%rsp) #11.5[spill]
  cmovae %eax, %esi #13.13
  shlq $10, %r10 #14.17
  movl %esi, %ebx #13.13
  movq %rbx, 144(%rsp) #11.5[spill]
  movl %esi, 120(%rsp) #11.5[spill]
  movq %rdx, 152(%rsp) #11.5[spill]
  lea (%r12,%r10), %r13 #14.38
  movl %ecx, 160(%rsp) #11.5[spill]
  addq %r11, %r10 #14.17
  movq 96(%rsp), %r15 #11.5[spill]
..B1.9: # Preds ..B1.19 ..B1.8
  movq %r13, 48(%rsp) #12.9[spill]
  xorl %eax, %eax #12.9
  movq %r10, 64(%rsp) #12.9[spill]
  lea (%r14,%r9), %rdx #14.28
  movq %r11, 104(%rsp) #12.9[spill]
  lea (%r11,%r9), %rsi #14.17
  movq %r9, 56(%rsp) #12.9[spill]
  lea (%r10,%r9), %rcx #14.17
  movq %r8, 72(%rsp) #12.9[spill]
  movq %r13, %rdi #12.9
  movq %r12, 80(%rsp) #12.9[spill]
  movq %r12, %rbx #12.9
  movq %r14, 88(%rsp) #12.9[spill]
  movl 120(%rsp), %r10d #12.9[spill]
  movl 128(%rsp), %r13d #12.9[spill]
  movq 136(%rsp), %r11 #12.9[spill]
..B1.10: # Preds ..B1.18 ..B1.9
  movsd A(%rdx,%rax,8), %xmm1 #14.28
  cmpl $8, %r10d #13.13
  jb ..B1.25 # Prob 10% #13.13
..B1.11: # Preds ..B1.10
  movaps %xmm1, %xmm0 #14.28
  movl %r10d, %r14d #13.13
  unpcklpd %xmm0, %xmm0 #14.28
  xorl %r8d, %r8d #13.13
  movq 144(%rsp), %r9 #13.13[spill]
..B1.12: # Preds ..B1.12 ..B1.11
  movups B(%rdi,%r8,8), %xmm2 #14.38
  movups 16+B(%rdi,%r8,8), %xmm3 #14.38
  movups 32+B(%rdi,%r8,8), %xmm4 #14.38
  movups 48+B(%rdi,%r8,8), %xmm5 #14.38
  mulpd %xmm0, %xmm2 #14.38
  mulpd %xmm0, %xmm3 #14.38
  mulpd %xmm0, %xmm4 #14.38
  mulpd %xmm0, %xmm5 #14.38
  addpd C(%rcx,%r8,8), %xmm2 #14.17
  addpd 16+C(%rcx,%r8,8), %xmm3 #14.17
  addpd 32+C(%rcx,%r8,8), %xmm4 #14.17
  addpd 48+C(%rcx,%r8,8), %xmm5 #14.17
  movups %xmm2, C(%rcx,%r8,8) #14.17
  movups %xmm3, 16+C(%rcx,%r8,8) #14.17
  movups %xmm4, 32+C(%rcx,%r8,8) #14.17
  movups %xmm5, 48+C(%rcx,%r8,8) #14.17
  addq $8, %r8 #13.13
  cmpq %r9, %r8 #13.13
  jb ..B1.12 # Prob 99% #13.13
..B1.14: # Preds ..B1.12 ..B1.25
  xorl %r12d, %r12d #13.13
  lea 1(%r14), %r8d #13.13
  xorl %r9d, %r9d #13.13
  cmpl %r10d, %r8d #13.13
  ja ..B1.18 # Prob 0% #13.13
..B1.15: # Preds ..B1.14
  lea (%r14,%r13), %r8d #14.17
  negl %r14d #13.13
  movslq %r8d, %r8 #14.38
  addl %r10d, %r14d #13.13
  movslq %r14d, %r14 #13.13
  lea (%rsi,%r8,8), %r15 #14.17
  lea (%rbx,%r8,8), %r8 #14.38
..B1.16: # Preds ..B1.16 ..B1.15
  movsd B(%r9,%r8), %xmm0 #14.38
  incq %r12 #13.13
  mulsd %xmm1, %xmm0 #14.38
  addsd C(%r9,%r15), %xmm0 #14.17
  movsd %xmm0, C(%r9,%r15) #14.17
  addq $8, %r9 #13.13
  cmpq %r14, %r12 #13.13
  jb ..B1.16 # Prob 99% #13.13
..B1.18: # Preds ..B1.16 ..B1.14
  incq %rax #12.9
  addq $2048, %rdi #12.9
  addq $2048, %rbx #12.9
  cmpq %r11, %rax #12.9
  jb ..B1.10 # Prob 99% #12.9
..B1.19: # Preds ..B1.18
  movq 72(%rsp), %r8 #[spill]
  incq %r8 #11.5
  movq 56(%rsp), %r9 #[spill]
  movq 96(%rsp), %r15 #[spill]
  addq $2048, %r9 #11.5
  movq 48(%rsp), %r13 #[spill]
  movq 64(%rsp), %r10 #[spill]
  movq 80(%rsp), %r12 #[spill]
  movq 88(%rsp), %r14 #[spill]
  movq 104(%rsp), %r11 #[spill]
  cmpq %r15, %r8 #11.5
  jb ..B1.9 # Prob 99% #11.5
..B1.20: # Preds ..B1.19
  movq 152(%rsp), %rdx #[spill]
  movl $128, %eax #
  incq %rdx #11.5
  movl 160(%rsp), %ecx #[spill]
  cmpq $2, %rdx #11.5
  jb ..B1.8 # Prob 99% #11.5
..B1.21: # Preds ..B1.20
  incl %ecx #11.5
  cmpl $2, %ecx #11.5
  jb ..B1.7 # Prob 99% #11.5
..B1.22: # Preds ..B1.21
  movl 112(%rsp), %edx #[spill]
  incl %edx #11.5
  cmpl $2, %edx #11.5
  jb ..B1.6 # Prob 99% #11.5
..B1.23: # Preds ..B1.22
  movq 263168+C(%rip), %rax #24.30
  movl $.L_2__STRING.0, %edi #25.5
  movq %rax, (%rsp) #24.28
  movl $1, %eax #25.5
  movsd (%rsp), %xmm0 #25.43
  movq 8(%rsp), %r12 #[spill]
  movq 16(%rsp), %r13 #[spill]
  movq 24(%rsp), %r14 #[spill]
  movq 32(%rsp), %r15 #[spill]
  movq 40(%rsp), %rbx #[spill]
  call printf #25.5
..B1.24: # Preds ..B1.23
  xorl %eax, %eax #26.12
  movq %rbp, %rsp #26.12
  popq %rbp #26.12
  ret #26.12
..B1.25: # Preds ..B1.10
  xorl %r14d, %r14d #13.13
  jmp ..B1.14 # Prob 100% #13.13
matmul:
..B2.1: # Preds ..B2.0
  pushq %r12 #10.19
  pushq %r13 #10.19
  pushq %r14 #10.19
  pushq %r15 #10.19
  pushq %rbx #10.19
  pushq %rbp #10.19
  subq $136, %rsp #10.19
  xorl %edi, %edi #11.5
  xorl %edx, %edx #11.5
  movl $128, %eax #11.5
  xorl %esi, %esi #11.5
..B2.2: # Preds ..B2.18 ..B2.1
  movl %edi, %ebx #11.5
  movl %edx, %ecx #11.5
  shll $7, %ebx #11.5
  negl %ebx #11.16
  addl $256, %ebx #11.16
  cmpl $128, %ebx #11.5
  movl %edi, %r12d #14.28
  cmovae %eax, %ebx #11.5
  shlq $18, %r12 #14.28
  movq %rbx, 16(%rsp) #11.5[spill]
  movl %edi, (%rsp) #11.5[spill]
..B2.3: # Preds ..B2.17 ..B2.2
  movl %ecx, %ebx #12.9
  movq %rsi, %rdx #11.5
  shll $7, %ebx #12.9
  negl %ebx #12.20
  addl $256, %ebx #12.20
  cmpl $128, %ebx #12.9
  movl %ecx, %r13d #14.28
  movq %r13, %r10 #14.28
  cmovae %eax, %ebx #12.9
  shlq $10, %r10 #14.28
  addq %r12, %r10 #14.28
  shlq $18, %r13 #14.38
  movq %rbx, 88(%rsp) #12.9[spill]
..B2.4: # Preds ..B2.16 ..B2.3
  movl %edx, %r14d #13.13
  movq %rdx, %r11 #14.17
  shll $7, %r14d #13.13
  movq %rsi, %r8 #11.5
  movl %r14d, %ebp #13.24
  movq %r8, %r9 #11.5
  negl %ebp #13.24
  addl $256, %ebp #13.24
  cmpl $128, %ebp #13.13
  movl %r14d, 80(%rsp) #11.5[spill]
  cmovae %eax, %ebp #13.13
  shlq $10, %r11 #14.17
  movl %ebp, %ebx #13.13
  movq %rbx, 96(%rsp) #11.5[spill]
  movl %ebp, 72(%rsp) #11.5[spill]
  movq %rdx, 120(%rsp) #11.5[spill]
  lea (%r13,%r11), %rdi #14.38
  movl %ecx, 112(%rsp) #11.5[spill]
  addq %r12, %r11 #14.17
  movq 16(%rsp), %r15 #11.5[spill]
..B2.5: # Preds ..B2.15 ..B2.4
  movq %r9, 56(%rsp) #12.9[spill]
  lea (%r11,%r9), %r14 #14.17
  movq %r11, 48(%rsp) #12.9[spill]
  movq %rsi, %rcx #12.9
  movq %r10, 24(%rsp) #12.9[spill]
  lea (%r10,%r9), %rdx #14.28
  movq %r14, 104(%rsp) #12.9[spill]
  lea (%r12,%r9), %rax #14.17
  movq %rdi, 64(%rsp) #12.9[spill]
  movq %rdi, %rbp #12.9
  movq %r8, 40(%rsp) #12.9[spill]
  movq %r13, %rbx #12.9
  movq %r13, 32(%rsp) #12.9[spill]
  movq %r12, 8(%rsp) #12.9[spill]
  xorl %r12d, %r12d #12.9
  movl 72(%rsp), %r11d #12.9[spill]
  movl 80(%rsp), %r10d #12.9[spill]
  movq 88(%rsp), %r9 #12.9[spill]
..B2.6: # Preds ..B2.14 ..B2.5
  movsd A(%rdx,%rcx,8), %xmm1 #14.28
  cmpl $8, %r11d #13.13
  jb ..B2.20 # Prob 10% #13.13
..B2.7: # Preds ..B2.6
  movaps %xmm1, %xmm0 #14.28
  movl %r11d, %r14d #13.13
  unpcklpd %xmm0, %xmm0 #14.28
  movq %rsi, %rdi #13.13
  movq 104(%rsp), %r8 #13.13[spill]
  movq 96(%rsp), %r13 #13.13[spill]
..B2.8: # Preds ..B2.8 ..B2.7
  movups B(%rbp,%rdi,8), %xmm2 #14.38
  movups 16+B(%rbp,%rdi,8), %xmm3 #14.38
  movups 32+B(%rbp,%rdi,8), %xmm4 #14.38
  movups 48+B(%rbp,%rdi,8), %xmm5 #14.38
  mulpd %xmm0, %xmm2 #14.38
  mulpd %xmm0, %xmm3 #14.38
  mulpd %xmm0, %xmm4 #14.38
  mulpd %xmm0, %xmm5 #14.38
  addpd C(%r8,%rdi,8), %xmm2 #14.17
  addpd 16+C(%r8,%rdi,8), %xmm3 #14.17
  addpd 32+C(%r8,%rdi,8), %xmm4 #14.17
  addpd 48+C(%r8,%rdi,8), %xmm5 #14.17
  movups %xmm2, C(%r8,%rdi,8) #14.17
  movups %xmm3, 16+C(%r8,%rdi,8) #14.17
  movups %xmm4, 32+C(%r8,%rdi,8) #14.17
  movups %xmm5, 48+C(%r8,%rdi,8) #14.17
  addq $8, %rdi #13.13
  cmpq %r13, %rdi #13.13
  jb ..B2.8 # Prob 99% #13.13
..B2.10: # Preds ..B2.8 ..B2.20
  movq %rsi, %r13 #13.13
  lea 1(%r14), %edi #13.13
  movq %r13, %r8 #13.13
  cmpl %r11d, %edi #13.13
  ja ..B2.14 # Prob 0% #13.13
..B2.11: # Preds ..B2.10
  lea (%r14,%r10), %edi #14.17
  negl %r14d #13.13
  movslq %edi, %rdi #14.38
  addl %r11d, %r14d #13.13
  movslq %r14d, %r14 #13.13
  lea (%rax,%rdi,8), %r15 #14.17
  lea (%rbx,%rdi,8), %rdi #14.38
..B2.12: # Preds ..B2.12 ..B2.11
  movsd B(%r8,%rdi), %xmm0 #14.38
  incq %r13 #13.13
  mulsd %xmm1, %xmm0 #14.38
  addsd C(%r8,%r15), %xmm0 #14.17
  movsd %xmm0, C(%r8,%r15) #14.17
  addq $8, %r8 #13.13
  cmpq %r14, %r13 #13.13
  jb ..B2.12 # Prob 99% #13.13
..B2.14: # Preds ..B2.12 ..B2.10
  incq %rcx #12.9
  addq $2048, %rbp #12.9
  addq $2048, %rbx #12.9
  cmpq %r9, %rcx #12.9
  jb ..B2.6 # Prob 99% #12.9
..B2.15: # Preds ..B2.14
  movq 40(%rsp), %r8 #[spill]
  incq %r8 #11.5
  movq 56(%rsp), %r9 #[spill]
  movq 16(%rsp), %r15 #[spill]
  addq $2048, %r9 #11.5
  movq 64(%rsp), %rdi #[spill]
  movq 48(%rsp), %r11 #[spill]
  movq 32(%rsp), %r13 #[spill]
  movq 24(%rsp), %r10 #[spill]
  movq 8(%rsp), %r12 #[spill]
  cmpq %r15, %r8 #11.5
  jb ..B2.5 # Prob 99% #11.5
..B2.16: # Preds ..B2.15
  movq 120(%rsp), %rdx #[spill]
  movl $128, %eax #
  incq %rdx #11.5
  movl 112(%rsp), %ecx #[spill]
  cmpq $2, %rdx #11.5
  jb ..B2.4 # Prob 99% #11.5
..B2.17: # Preds ..B2.16
  incl %ecx #11.5
  cmpl $2, %ecx #11.5
  jb ..B2.3 # Prob 99% #11.5
..B2.18: # Preds ..B2.17
  movl (%rsp), %edi #[spill]
  xorl %edx, %edx #
  incl %edi #11.5
  cmpl $2, %edi #11.5
  jb ..B2.2 # Prob 99% #11.5
..B2.19: # Preds ..B2.18
  addq $136, %rsp #15.1
  popq %rbp #15.1
  popq %rbx #15.1
  popq %r15 #15.1
  popq %r14 #15.1
  popq %r13 #15.1
  popq %r12 #15.1
  ret #15.1
..B2.20: # Preds ..B2.6
  movl %r12d, %r14d #13.13
  jmp ..B2.10 # Prob 100% #13.13
A:
B:
C:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2__STRING.0:
