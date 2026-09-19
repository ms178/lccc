main:
..B1.1: # Preds ..B1.0
  pushq %rbp #24.16
  movq %rsp, %rbp #24.16
  andq $-128, %rsp #24.16
  subq $128, %rsp #24.16
  movl $3, %edi #24.16
  xorl %esi, %esi #24.16
  call __intel_new_feature_proc_init #24.16
..B1.17: # Preds ..B1.1
  stmxcsr (%rsp) #24.16
  movl $sieve, %edi #12.5
  movl $1, %esi #12.5
  orl $32832, (%rsp) #24.16
  movl $10000001, %edx #12.5
  ldmxcsr (%rsp) #24.16
  call _intel_fast_memset #12.5
..B1.2: # Preds ..B1.17
  xorl %eax, %eax #13.5
  movl $2, %esi #14.16
  movb %al, sieve(%rip) #13.5
  movl $2, %ecx #14.16
  movb %al, 1+sieve(%rip) #13.16
..B1.3: # Preds ..B1.8 ..B1.2
  cmpb $0, sieve(%rcx) #15.13
  je ..B1.8 # Prob 50% #15.13
..B1.4: # Preds ..B1.3
  movq %rcx, %rdx #16.28
  imulq %rcx, %rdx #16.28
  cmpq $10000000, %rdx #16.36
  jg ..B1.8 # Prob 10% #16.36
..B1.6: # Preds ..B1.4 ..B1.6
  movb $0, sieve(%rdx) #17.17
  addq %rcx, %rdx #16.39
  cmpq $10000000, %rdx #16.36
  jle ..B1.6 # Prob 99% #16.36
..B1.8: # Preds ..B1.6 ..B1.3 ..B1.4
  incl %esi #14.33
  incq %rcx #14.33
  movl %esi, %edx #14.25
  imull %esi, %edx #14.25
  cmpl $10000000, %edx #14.30
  jle ..B1.3 # Prob 82% #14.30
..B1.9: # Preds ..B1.8
  movl %eax, %r8d #19.5
  movl $sieve+2, %edi #20.13
  pxor %xmm1, %xmm1 #18.15
  pcmpeqd %xmm0, %xmm0 #20.13
  movdqa %xmm1, %xmm2 #18.15
  movl $sieve+6, %esi #20.13
  movl $sieve+10, %ecx #20.13
  movl $sieve+14, %edx #20.13
..B1.10: # Preds ..B1.10 ..B1.9
  movd (%rdi), %xmm3 #20.13
  addl $16, %r8d #19.5
  punpcklbw %xmm3, %xmm3 #20.13
  addq $16, %rdi #19.5
  movd (%rsi), %xmm4 #20.13
  addq $16, %rsi #19.5
  punpcklwd %xmm3, %xmm3 #20.13
  punpcklbw %xmm4, %xmm4 #20.13
  psrad $24, %xmm3 #20.13
  movd (%rcx), %xmm5 #20.13
  pcmpeqd %xmm1, %xmm3 #20.13
  punpcklwd %xmm4, %xmm4 #20.13
  pxor %xmm0, %xmm3 #20.13
  punpcklbw %xmm5, %xmm5 #20.13
  psrad $24, %xmm4 #20.13
  movd (%rdx), %xmm6 #20.13
  pcmpeqd %xmm1, %xmm4 #20.13
  punpcklwd %xmm5, %xmm5 #20.13
  psubd %xmm3, %xmm2 #20.23
  punpcklbw %xmm6, %xmm6 #20.13
  psrad $24, %xmm5 #20.13
  punpcklwd %xmm6, %xmm6 #20.13
  pxor %xmm0, %xmm4 #20.13
  pcmpeqd %xmm1, %xmm5 #20.13
  psrad $24, %xmm6 #20.13
  pcmpeqd %xmm1, %xmm6 #20.13
  psubd %xmm4, %xmm2 #20.23
  pxor %xmm0, %xmm5 #20.13
  pxor %xmm0, %xmm6 #20.13
  psubd %xmm5, %xmm2 #20.23
  addq $16, %rcx #19.5
  addq $16, %rdx #19.5
  psubd %xmm6, %xmm2 #20.23
  cmpl $9999984, %r8d #19.5
  jb ..B1.10 # Prob 99% #19.5
..B1.11: # Preds ..B1.10
  xorb %cl, %cl #19.5
  movl $sieve+9999986, %edx #20.13
..B1.12: # Preds ..B1.12 ..B1.11
  movd (%rdx), %xmm3 #20.13
  addb $4, %cl #19.5
  punpcklbw %xmm3, %xmm3 #20.13
  addq $4, %rdx #19.5
  punpcklwd %xmm3, %xmm3 #20.13
  psrad $24, %xmm3 #20.13
  pcmpeqd %xmm1, %xmm3 #20.13
  pxor %xmm0, %xmm3 #20.13
  psubd %xmm3, %xmm2 #20.23
  cmpb $12, %cl #19.5
  jb ..B1.12 # Prob 99% #19.5
..B1.13: # Preds ..B1.12
  movdqa %xmm2, %xmm0 #18.15
  movl %eax, %edx #20.23
  psrldq $8, %xmm0 #18.15
  movl $.L_2__STRING.0, %edi #26.5
  paddd %xmm0, %xmm2 #18.15
  movl $10000000, %esi #26.5
  cmpb $0, 9999998+sieve(%rip) #20.23
  movdqa %xmm2, %xmm1 #18.15
  psrlq $32, %xmm1 #18.15
  setne %dl #20.23
  paddd %xmm1, %xmm2 #18.15
  cmpb 9999999+sieve(%rip), %al #20.23
  movd %xmm2, %ecx #18.15
  adcl $0, %edx #20.23
  cmpb 10000000+sieve(%rip), %al #18.15
  adcl $0, %edx #18.15
  xorl %eax, %eax #18.15
  addl %edx, %ecx #18.15
  movl %ecx, 4(%rsp) #25.25
  movl 4(%rsp), %edx #26.40
  call printf #26.5
..B1.14: # Preds ..B1.13
  xorl %eax, %eax #27.12
  movq %rbp, %rsp #27.12
  popq %rbp #27.12
  ret #27.12
count_primes:
..B2.1: # Preds ..B2.0
  pushq %rsi #11.24
  movl $sieve, %edi #12.5
  movl $1, %esi #12.5
  movl $10000001, %edx #12.5
  call _intel_fast_memset #12.5
..B2.2: # Preds ..B2.1
  xorl %edx, %edx #13.5
  movl $2, %esi #14.16
  movb %dl, sieve(%rip) #13.5
  movl $2, %ecx #14.16
  movb %dl, 1+sieve(%rip) #13.16
..B2.3: # Preds ..B2.8 ..B2.2
  cmpb $0, sieve(%rcx) #15.13
  je ..B2.8 # Prob 50% #15.13
..B2.4: # Preds ..B2.3
  movq %rcx, %rax #16.28
  imulq %rcx, %rax #16.28
  cmpq $10000000, %rax #16.36
  jg ..B2.8 # Prob 10% #16.36
..B2.6: # Preds ..B2.4 ..B2.6
  movb $0, sieve(%rax) #17.17
  addq %rcx, %rax #16.39
  cmpq $10000000, %rax #16.36
  jle ..B2.6 # Prob 99% #16.36
..B2.8: # Preds ..B2.6 ..B2.3 ..B2.4
  incl %esi #14.33
  incq %rcx #14.33
  movl %esi, %eax #14.25
  imull %esi, %eax #14.25
  cmpl $10000000, %eax #14.30
  jle ..B2.3 # Prob 82% #14.30
..B2.9: # Preds ..B2.8
  movl %edx, %r8d #19.5
  movl $sieve+2, %edi #20.13
  pxor %xmm1, %xmm1 #18.15
  pcmpeqd %xmm0, %xmm0 #20.13
  movdqa %xmm1, %xmm2 #18.15
  movl $sieve+6, %esi #20.13
  movl $sieve+10, %ecx #20.13
  movl $sieve+14, %eax #20.13
..B2.10: # Preds ..B2.10 ..B2.9
  movd (%rdi), %xmm3 #20.13
  addl $16, %r8d #19.5
  punpcklbw %xmm3, %xmm3 #20.13
  addq $16, %rdi #19.5
  movd (%rsi), %xmm4 #20.13
  addq $16, %rsi #19.5
  punpcklwd %xmm3, %xmm3 #20.13
  punpcklbw %xmm4, %xmm4 #20.13
  psrad $24, %xmm3 #20.13
  movd (%rcx), %xmm5 #20.13
  pcmpeqd %xmm1, %xmm3 #20.13
  punpcklwd %xmm4, %xmm4 #20.13
  pxor %xmm0, %xmm3 #20.13
  punpcklbw %xmm5, %xmm5 #20.13
  psrad $24, %xmm4 #20.13
  movd (%rax), %xmm6 #20.13
  pcmpeqd %xmm1, %xmm4 #20.13
  punpcklwd %xmm5, %xmm5 #20.13
  psubd %xmm3, %xmm2 #20.23
  punpcklbw %xmm6, %xmm6 #20.13
  psrad $24, %xmm5 #20.13
  punpcklwd %xmm6, %xmm6 #20.13
  pxor %xmm0, %xmm4 #20.13
  pcmpeqd %xmm1, %xmm5 #20.13
  psrad $24, %xmm6 #20.13
  pcmpeqd %xmm1, %xmm6 #20.13
  psubd %xmm4, %xmm2 #20.23
  pxor %xmm0, %xmm5 #20.13
  pxor %xmm0, %xmm6 #20.13
  psubd %xmm5, %xmm2 #20.23
  addq $16, %rcx #19.5
  addq $16, %rax #19.5
  psubd %xmm6, %xmm2 #20.23
  cmpl $9999984, %r8d #19.5
  jb ..B2.10 # Prob 99% #19.5
..B2.11: # Preds ..B2.10
  xorb %cl, %cl #19.5
  movl $sieve+9999986, %eax #20.13
..B2.12: # Preds ..B2.12 ..B2.11
  movd (%rax), %xmm3 #20.13
  addb $4, %cl #19.5
  punpcklbw %xmm3, %xmm3 #20.13
  addq $4, %rax #19.5
  punpcklwd %xmm3, %xmm3 #20.13
  psrad $24, %xmm3 #20.13
  pcmpeqd %xmm1, %xmm3 #20.13
  pxor %xmm0, %xmm3 #20.13
  psubd %xmm3, %xmm2 #20.23
  cmpb $12, %cl #19.5
  jb ..B2.12 # Prob 99% #19.5
..B2.13: # Preds ..B2.12
  movdqa %xmm2, %xmm0 #18.15
  movl %edx, %ecx #20.23
  psrldq $8, %xmm0 #18.15
  paddd %xmm0, %xmm2 #18.15
  cmpb $0, 9999998+sieve(%rip) #20.23
  movdqa %xmm2, %xmm1 #18.15
  psrlq $32, %xmm1 #18.15
  setne %cl #20.23
  paddd %xmm1, %xmm2 #18.15
  cmpb 9999999+sieve(%rip), %dl #20.23
  movd %xmm2, %eax #18.15
  adcl $0, %ecx #20.23
  cmpb 10000000+sieve(%rip), %dl #21.12
  adcl $0, %ecx #21.12
  addl %ecx, %eax #18.15
  popq %rcx #21.12
  ret #21.12
sieve:
.L_2__STRING.0:
