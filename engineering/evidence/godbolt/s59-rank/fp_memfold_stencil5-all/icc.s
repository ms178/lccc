main:
..B1.1: # Preds ..B1.0
  pushq %rbp #22.33
  movq %rsp, %rbp #22.33
  andq $-128, %rsp #22.33
  pushq %r12 #22.33
  pushq %r13 #22.33
  subq $112, %rsp #22.33
  movq %rsi, %r13 #22.33
  movl %edi, %r12d #22.33
  movl $3, %edi #22.33
  xorl %esi, %esi #22.33
  call __intel_new_feature_proc_init #22.33
..B1.22: # Preds ..B1.1
  stmxcsr (%rsp) #22.33
  orl $32832, (%rsp) #22.33
  ldmxcsr (%rsp) #22.33
  cmpl $1, %r12d #23.30
  jle ..B1.4 # Prob 50% #23.30
..B1.2: # Preds ..B1.22
  movq 8(%r13), %rdi #483.16
  call atol #483.16
..B1.3: # Preds ..B1.2
  movl %eax, %r12d #483.16
  jmp ..B1.5 # Prob 100% #483.16
..B1.4: # Preds ..B1.22
  movl $1000, %r12d #23.50
..B1.5: # Preds ..B1.3 ..B1.4
  movdqu .L_2il0floatpacket.0(%rip), %xmm3 #25.9
  xorl %eax, %eax #24.5
  movdqu .L_2il0floatpacket.1(%rip), %xmm2 #25.9
  movups .L_2il0floatpacket.2(%rip), %xmm1 #25.38
  movdqu .L_2il0floatpacket.3(%rip), %xmm0 #25.32
..B1.6: # Preds ..B1.6 ..B1.5
  movdqa %xmm0, %xmm4 #25.32
  movdqa %xmm0, %xmm6 #25.32
  pand %xmm2, %xmm4 #25.32
  paddd %xmm3, %xmm2 #25.9
  pand %xmm2, %xmm6 #25.32
  paddd %xmm3, %xmm2 #25.9
  movdqa %xmm0, %xmm8 #25.32
  movdqa %xmm0, %xmm10 #25.32
  pand %xmm2, %xmm8 #25.32
  paddd %xmm3, %xmm2 #25.9
  pand %xmm2, %xmm10 #25.32
  paddd %xmm3, %xmm2 #25.9
  cvtdq2ps %xmm4, %xmm5 #25.32
  cvtdq2ps %xmm6, %xmm7 #25.32
  cvtdq2ps %xmm8, %xmm9 #25.32
  cvtdq2ps %xmm10, %xmm11 #25.32
  mulps %xmm1, %xmm5 #25.38
  mulps %xmm1, %xmm7 #25.38
  mulps %xmm1, %xmm9 #25.38
  mulps %xmm1, %xmm11 #25.38
  movups %xmm5, input(,%rax,4) #25.9
  movups %xmm7, 16+input(,%rax,4) #25.9
  movups %xmm9, 32+input(,%rax,4) #25.9
  movups %xmm11, 48+input(,%rax,4) #25.9
  addq $16, %rax #24.5
  cmpq $65536, %rax #24.5
  jb ..B1.6 # Prob 99% #24.5
..B1.7: # Preds ..B1.6
  xorl %r13d, %r13d #26.16
  testl %r12d, %r12d #26.25
  jle ..B1.12 # Prob 10% #26.25
..B1.9: # Preds ..B1.7 ..B1.10
  movl $output, %edi #27.9
  movl $input, %esi #27.9
  movl $65536, %edx #27.9
  call stencil5 #27.9
..B1.10: # Preds ..B1.9
  incl %r13d #26.40
  cmpl %r12d, %r13d #26.25
  jl ..B1.9 # Prob 82% #26.25
..B1.12: # Preds ..B1.10 ..B1.7
  xorl %eax, %eax #30.5
  pxor %xmm0, %xmm0 #29.21
  movaps %xmm0, %xmm7 #29.21
  movaps %xmm7, %xmm6 #29.21
  movaps %xmm6, %xmm5 #29.21
  movaps %xmm5, %xmm4 #29.21
  movaps %xmm4, %xmm3 #29.21
  movaps %xmm3, %xmm2 #29.21
  movaps %xmm2, %xmm1 #29.21
..B1.13: # Preds ..B1.13 ..B1.12
  cvtps2pd output(,%rax,4), %xmm8 #31.21
  cvtps2pd 8+output(,%rax,4), %xmm9 #31.21
  cvtps2pd 16+output(,%rax,4), %xmm10 #31.21
  cvtps2pd 24+output(,%rax,4), %xmm11 #31.21
  addpd %xmm8, %xmm0 #31.9
  addpd %xmm9, %xmm7 #31.9
  cvtps2pd 32+output(,%rax,4), %xmm12 #31.21
  addpd %xmm10, %xmm6 #31.9
  addpd %xmm11, %xmm5 #31.9
  cvtps2pd 40+output(,%rax,4), %xmm13 #31.21
  cvtps2pd 48+output(,%rax,4), %xmm14 #31.21
  addpd %xmm12, %xmm4 #31.9
  cvtps2pd 56+output(,%rax,4), %xmm15 #31.21
  addpd %xmm13, %xmm3 #31.9
  addpd %xmm14, %xmm2 #31.9
  addpd %xmm15, %xmm1 #31.9
  addq $16, %rax #30.5
  cmpq $65536, %rax #30.5
  jb ..B1.13 # Prob 99% #30.5
..B1.14: # Preds ..B1.13
  addpd %xmm7, %xmm0 #29.21
  addpd %xmm5, %xmm6 #29.21
  addpd %xmm3, %xmm4 #29.21
  addpd %xmm1, %xmm2 #29.21
  addpd %xmm6, %xmm0 #29.21
  addpd %xmm2, %xmm4 #29.21
  addpd %xmm4, %xmm0 #29.21
  movaps %xmm0, %xmm1 #29.21
  movl $.L_2__STRING.0, %edi #32.5
  unpckhpd %xmm0, %xmm1 #29.21
  movl $1, %eax #32.5
  addsd %xmm1, %xmm0 #29.21
  movsd %xmm0, (%rsp) #32.5[spill]
  call printf #32.5
..B1.15: # Preds ..B1.14
  movsd (%rsp), %xmm0 #[spill]
  comisd .L_2il0floatpacket.4(%rip), %xmm0 #33.23
  jbe ..B1.18 # Prob 50% #33.23
..B1.16: # Preds ..B1.15
  movsd .L_2il0floatpacket.5(%rip), %xmm1 #33.46
  comisd %xmm0, %xmm1 #33.46
  jbe ..B1.18 # Prob 50% #33.46
..B1.17: # Preds ..B1.16
  xorl %eax, %eax #33.23
  addq $112, %rsp #33.23
  popq %r13 #33.23
  popq %r12 #33.23
  movq %rbp, %rsp #33.23
  popq %rbp #33.23
  ret #33.23
..B1.18: # Preds ..B1.15 ..B1.16
  movl $1, %eax #33.23
..B1.19: # Preds ..B1.18
  addq $112, %rsp #33.23
  popq %r13 #33.23
  popq %r12 #33.23
  movq %rbp, %rsp #33.23
  popq %rbp #33.23
  ret #33.23
stencil5:
..B2.1: # Preds ..B2.0
  movq %rsi, %r8 #17.56
  lea 16(%r8), %r9 #19.66
  andq $15, %r9 #18.5
  testl %r9d, %r9d #18.5
  je ..B2.6 # Prob 50% #18.5
..B2.2: # Preds ..B2.1
  testl $3, %r9d #18.5
  jne ..B2.24 # Prob 10% #18.5
..B2.3: # Preds ..B2.2
  negl %r9d #18.5
  xorl %edx, %edx #18.5
  addl $16, %r9d #18.5
  shrl $2, %r9d #18.5
  movl %r9d, %eax #18.5
..B2.4: # Preds ..B2.4 ..B2.3
  movss (%r8,%rdx,4), %xmm0 #19.18
  addss 4(%r8,%rdx,4), %xmm0 #19.31
  addss 8(%r8,%rdx,4), %xmm0 #19.44
  addss 12(%r8,%rdx,4), %xmm0 #19.53
  addss 16(%r8,%rdx,4), %xmm0 #19.66
  movss %xmm0, 8(%rdi,%rdx,4) #19.9
  incq %rdx #18.5
  cmpq %rax, %rdx #18.5
  jb ..B2.4 # Prob 82% #18.5
  jmp ..B2.7 # Prob 100% #18.5
..B2.6: # Preds ..B2.1
  xorl %eax, %eax #18.5
..B2.7: # Preds ..B2.4 ..B2.6
  movl %r9d, %esi #18.5
  addl $2, %r9d #19.9
  negl %esi #18.5
  lea (%rdi,%r9,4), %r10 #19.9
  addl $12, %esi #18.5
  andl $15, %esi #18.5
  negl %esi #18.5
  lea 65532(%rsi), %ecx #18.5
  movl %ecx, %edx #18.5
  testq $15, %r10 #18.5
  je ..B2.12 # Prob 60% #18.5
..B2.9: # Preds ..B2.7 ..B2.9
  movups 4(%r8,%rax,4), %xmm0 #19.18
  movups (%r8,%rax,4), %xmm3 #19.18
  movups 8(%r8,%rax,4), %xmm1 #19.18
  movups 12(%r8,%rax,4), %xmm2 #19.18
  movups 20(%r8,%rax,4), %xmm4 #19.66
  movups 16(%r8,%rax,4), %xmm7 #19.66
  movups 24(%r8,%rax,4), %xmm5 #19.66
  movups 28(%r8,%rax,4), %xmm6 #19.66
  addps %xmm0, %xmm3 #19.31
  addps %xmm1, %xmm3 #19.44
  addps %xmm2, %xmm3 #19.53
  addps %xmm7, %xmm3 #19.66
  addps %xmm4, %xmm7 #19.31
  movups %xmm3, 8(%rdi,%rax,4) #19.9
  addps %xmm5, %xmm7 #19.44
  movups 36(%r8,%rax,4), %xmm8 #19.66
  movups 32(%r8,%rax,4), %xmm11 #19.66
  movups 40(%r8,%rax,4), %xmm9 #19.66
  movups 44(%r8,%rax,4), %xmm10 #19.66
  addps %xmm6, %xmm7 #19.53
  addps %xmm11, %xmm7 #19.66
  addps %xmm8, %xmm11 #19.31
  movups %xmm7, 24(%rdi,%rax,4) #19.9
  addps %xmm9, %xmm11 #19.44
  movups 52(%r8,%rax,4), %xmm12 #19.66
  movups 48(%r8,%rax,4), %xmm15 #19.66
  movups 56(%r8,%rax,4), %xmm13 #19.66
  movups 60(%r8,%rax,4), %xmm14 #19.66
  addps %xmm10, %xmm11 #19.53
  addps %xmm15, %xmm11 #19.66
  addps %xmm12, %xmm15 #19.31
  movups %xmm11, 40(%rdi,%rax,4) #19.9
  addps %xmm13, %xmm15 #19.44
  addps %xmm14, %xmm15 #19.53
  addps 64(%r8,%rax,4), %xmm15 #19.66
  movups %xmm15, 56(%rdi,%rax,4) #19.9
  addq $16, %rax #18.5
  cmpq %rdx, %rax #18.5
  jb ..B2.9 # Prob 82% #18.5
  jmp ..B2.14 # Prob 100% #18.5
..B2.12: # Preds ..B2.7 ..B2.12
  movups 4(%r8,%rax,4), %xmm0 #19.18
  movups (%r8,%rax,4), %xmm3 #19.18
  movups 8(%r8,%rax,4), %xmm1 #19.18
  movups 12(%r8,%rax,4), %xmm2 #19.18
  movups 20(%r8,%rax,4), %xmm4 #19.66
  movups 16(%r8,%rax,4), %xmm7 #19.66
  movups 24(%r8,%rax,4), %xmm5 #19.66
  movups 28(%r8,%rax,4), %xmm6 #19.66
  addps %xmm0, %xmm3 #19.31
  addps %xmm1, %xmm3 #19.44
  addps %xmm2, %xmm3 #19.53
  addps %xmm7, %xmm3 #19.66
  addps %xmm4, %xmm7 #19.31
  movups %xmm3, 8(%rdi,%rax,4) #19.9
  addps %xmm5, %xmm7 #19.44
  movups 36(%r8,%rax,4), %xmm8 #19.66
  movups 32(%r8,%rax,4), %xmm11 #19.66
  movups 40(%r8,%rax,4), %xmm9 #19.66
  movups 44(%r8,%rax,4), %xmm10 #19.66
  addps %xmm6, %xmm7 #19.53
  addps %xmm11, %xmm7 #19.66
  addps %xmm8, %xmm11 #19.31
  movups %xmm7, 24(%rdi,%rax,4) #19.9
  addps %xmm9, %xmm11 #19.44
  movups 52(%r8,%rax,4), %xmm12 #19.66
  movups 48(%r8,%rax,4), %xmm15 #19.66
  movups 56(%r8,%rax,4), %xmm13 #19.66
  movups 60(%r8,%rax,4), %xmm14 #19.66
  addps %xmm10, %xmm11 #19.53
  addps %xmm15, %xmm11 #19.66
  addps %xmm12, %xmm15 #19.31
  movups %xmm11, 40(%rdi,%rax,4) #19.9
  addps %xmm13, %xmm15 #19.44
  addps %xmm14, %xmm15 #19.53
  addps 64(%r8,%rax,4), %xmm15 #19.66
  movups %xmm15, 56(%rdi,%rax,4) #19.9
  addq $16, %rax #18.5
  cmpq %rdx, %rax #18.5
  jb ..B2.12 # Prob 82% #18.5
..B2.14: # Preds ..B2.12 ..B2.9
  lea 65533(%rsi), %eax #18.5
  cmpl $65532, %eax #18.5
  ja ..B2.23 # Prob 50% #18.5
..B2.15: # Preds ..B2.14
  movl %ecx, %eax #18.5
  negl %eax #18.5
  addl $65532, %eax #18.5
  cmpl $4, %eax #18.5
  jb ..B2.25 # Prob 10% #18.5
..B2.16: # Preds ..B2.15
  movl %eax, %edx #18.5
  xorl %r9d, %r9d #18.5
  andl $-4, %edx #18.5
..B2.17: # Preds ..B2.17 ..B2.16
  lea 65532(%r9,%rsi), %r10d #19.26
  addl $4, %r9d #18.5
  movslq %r10d, %r10 #19.26
  movups 4(%r8,%r10,4), %xmm0 #19.18
  movups (%r8,%r10,4), %xmm3 #19.18
  movups 8(%r8,%r10,4), %xmm1 #19.18
  movups 12(%r8,%r10,4), %xmm2 #19.18
  addps %xmm0, %xmm3 #19.31
  addps %xmm1, %xmm3 #19.44
  addps %xmm2, %xmm3 #19.53
  addps 16(%r8,%r10,4), %xmm3 #19.66
  movups %xmm3, 8(%rdi,%r10,4) #19.9
  cmpl %edx, %r9d #18.5
  jb ..B2.17 # Prob 82% #18.5
..B2.19: # Preds ..B2.17 ..B2.25 ..B2.24
  cmpl %eax, %edx #18.5
  jae ..B2.23 # Prob 0% #18.5
..B2.21: # Preds ..B2.19 ..B2.21
  lea (%rcx,%rdx), %esi #19.26
  incl %edx #18.5
  movslq %esi, %rsi #19.74
  movss (%r8,%rsi,4), %xmm0 #19.18
  addss 4(%r8,%rsi,4), %xmm0 #19.31
  addss 8(%r8,%rsi,4), %xmm0 #19.44
  addss 12(%r8,%rsi,4), %xmm0 #19.53
  addss 16(%r8,%rsi,4), %xmm0 #19.66
  movss %xmm0, 8(%rdi,%rsi,4) #19.9
  cmpl %eax, %edx #18.5
  jb ..B2.21 # Prob 82% #18.5
..B2.23: # Preds ..B2.21 ..B2.14 ..B2.19
  ret #20.1
..B2.24: # Preds ..B2.2
  xorl %ecx, %ecx #18.5
  movl $65532, %eax #18.5
  xorl %edx, %edx #18.5
  jmp ..B2.19 # Prob 100% #18.5
..B2.25: # Preds ..B2.15
  xorl %edx, %edx #18.5
  jmp ..B2.19 # Prob 100% #18.5
input:
output:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
.L_2__STRING.0:
