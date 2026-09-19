main:
..B1.1: # Preds ..B1.0
  pushq %rbp #33.33
  movq %rsp, %rbp #33.33
  andq $-128, %rsp #33.33
  pushq %r14 #33.33
  pushq %r15 #33.33
  subq $112, %rsp #33.33
  movq %rsi, %r15 #33.33
  movl %edi, %r14d #33.33
  movl $3, %edi #33.33
  xorl %esi, %esi #33.33
  call __intel_new_feature_proc_init #33.33
..B1.15: # Preds ..B1.1
  stmxcsr (%rsp) #33.33
  orl $32832, (%rsp) #33.33
  ldmxcsr (%rsp) #33.33
  cmpl $1, %r14d #34.30
  jle ..B1.4 # Prob 50% #34.30
..B1.2: # Preds ..B1.15
  movq 8(%r15), %rdi #483.16
  call atol #483.16
..B1.3: # Preds ..B1.2
  movl %eax, %r14d #483.16
  jmp ..B1.5 # Prob 100% #483.16
..B1.4: # Preds ..B1.15
  movl $5000, %r14d #34.50
..B1.5: # Preds ..B1.3 ..B1.4
  movdqu .L_2il0floatpacket.0(%rip), %xmm5 #36.9
  xorl %eax, %eax #35.5
  movdqu .L_2il0floatpacket.1(%rip), %xmm4 #36.9
  movups .L_2il0floatpacket.2(%rip), %xmm3 #36.40
  movdqu .L_2il0floatpacket.3(%rip), %xmm2 #36.34
  movups .L_2il0floatpacket.4(%rip), %xmm1 #37.39
  movdqu .L_2il0floatpacket.5(%rip), %xmm0 #37.34
..B1.6: # Preds ..B1.6 ..B1.5
  movdqa %xmm2, %xmm6 #36.34
  movdqa %xmm0, %xmm8 #37.34
  pand %xmm4, %xmm6 #36.34
  pand %xmm4, %xmm8 #37.34
  paddd %xmm5, %xmm4 #36.9
  movdqa %xmm2, %xmm10 #36.34
  movdqa %xmm0, %xmm12 #37.34
  pand %xmm4, %xmm10 #36.34
  pand %xmm4, %xmm12 #37.34
  paddd %xmm5, %xmm4 #36.9
  cvtdq2ps %xmm6, %xmm7 #36.34
  cvtdq2ps %xmm8, %xmm9 #37.34
  cvtdq2ps %xmm10, %xmm11 #36.34
  cvtdq2ps %xmm12, %xmm13 #37.34
  mulps %xmm3, %xmm7 #36.40
  mulps %xmm1, %xmm9 #37.39
  mulps %xmm3, %xmm11 #36.40
  mulps %xmm1, %xmm13 #37.39
  movups %xmm7, input_a(,%rax,4) #36.9
  movups %xmm9, input_b(,%rax,4) #37.9
  movups %xmm11, 16+input_a(,%rax,4) #36.9
  movups %xmm13, 16+input_b(,%rax,4) #37.9
  addq $8, %rax #35.5
  cmpq $65536, %rax #35.5
  jb ..B1.6 # Prob 99% #35.5
..B1.7: # Preds ..B1.6
  xorl %r15d, %r15d #39.16
  testl %r14d, %r14d #39.25
  jle ..B1.11 # Prob 10% #39.25
..B1.9: # Preds ..B1.7 ..B1.17
  movl $input_a, %edi #40.16
  movl $65536, %esi #40.16
  call sum_f32 #40.16
..B1.18: # Preds ..B1.9
  movl $input_a, %edi #41.17
  movl $input_b, %esi #41.17
  movl $65536, %edx #41.17
  movss %xmm0, sink(%rip) #40.9
  call dot_f32 #41.17
..B1.17: # Preds ..B1.18
  movaps %xmm0, %xmm1 #41.17
  incl %r15d #39.40
  movss sink(%rip), %xmm0 #41.9
  addss %xmm0, %xmm1 #41.9
  movss %xmm1, sink(%rip) #41.9
  cmpl %r14d, %r15d #39.25
  jl ..B1.9 # Prob 82% #39.25
..B1.11: # Preds ..B1.17 ..B1.7
  movss sink(%rip), %xmm0 #43.30
  movl $.L_2__STRING.0, %edi #43.5
  cvtss2sd %xmm0, %xmm0 #43.5
  movl $1, %eax #43.5
  call printf #43.5
..B1.12: # Preds ..B1.11
  movss sink(%rip), %xmm0 #44.12
  cmpneqss .L_2il0floatpacket.6(%rip), %xmm0 #44.20
  movd %xmm0, %eax #44.20
  negl %eax #44.20
  addq $112, %rsp #44.20
  popq %r15 #44.20
  popq %r14 #44.20
  movq %rbp, %rsp #44.20
  popq %rbp #44.20
  ret #44.20
dot_f32:
..B2.1: # Preds ..B2.0
  movq %rsi, %r8 #26.55
  andq $15, %rsi #28.5
  pxor %xmm1, %xmm1 #27.15
  pxor %xmm0, %xmm0 #27.15
  testl %esi, %esi #28.5
  je ..B2.6 # Prob 50% #28.5
..B2.2: # Preds ..B2.1
  testl $3, %esi #28.5
  jne ..B2.24 # Prob 10% #28.5
..B2.3: # Preds ..B2.2
  negl %esi #28.5
  xorl %edx, %edx #28.5
  addl $16, %esi #28.5
  shrl $2, %esi #28.5
  movl %esi, %eax #28.5
..B2.4: # Preds ..B2.4 ..B2.3
  movss (%rdi,%rdx,4), %xmm2 #29.16
  mulss (%r8,%rdx,4), %xmm2 #29.23
  incq %rdx #28.5
  addss %xmm2, %xmm1 #29.9
  cmpq %rax, %rdx #28.5
  jb ..B2.4 # Prob 82% #28.5
  jmp ..B2.7 # Prob 100% #28.5
..B2.6: # Preds ..B2.1
  xorl %eax, %eax #29.9
..B2.7: # Preds ..B2.4 ..B2.6
  negl %esi #28.5
  lea (%rdi,%rax,4), %r9 #29.16
  andl $15, %esi #28.5
  pxor %xmm4, %xmm4 #27.15
  negl %esi #28.5
  movaps %xmm4, %xmm3 #27.15
  movaps %xmm3, %xmm2 #27.15
  lea 65536(%rsi), %ecx #28.5
  movl %ecx, %edx #28.5
  testq $15, %r9 #28.5
  je ..B2.12 # Prob 60% #28.5
..B2.9: # Preds ..B2.7 ..B2.9
  movups (%rdi,%rax,4), %xmm5 #29.16
  movups 16(%rdi,%rax,4), %xmm6 #29.16
  movups 32(%rdi,%rax,4), %xmm7 #29.16
  movups 48(%rdi,%rax,4), %xmm8 #29.16
  mulps (%r8,%rax,4), %xmm5 #29.23
  mulps 16(%r8,%rax,4), %xmm6 #29.23
  mulps 32(%r8,%rax,4), %xmm7 #29.23
  addps %xmm5, %xmm0 #29.9
  mulps 48(%r8,%rax,4), %xmm8 #29.23
  addps %xmm6, %xmm4 #29.9
  addps %xmm7, %xmm3 #29.9
  addps %xmm8, %xmm2 #29.9
  addq $16, %rax #28.5
  cmpq %rdx, %rax #28.5
  jb ..B2.9 # Prob 82% #28.5
  jmp ..B2.14 # Prob 100% #28.5
..B2.12: # Preds ..B2.7 ..B2.12
  movups (%rdi,%rax,4), %xmm5 #29.16
  movups 16(%rdi,%rax,4), %xmm6 #29.16
  movups 32(%rdi,%rax,4), %xmm7 #29.16
  movups 48(%rdi,%rax,4), %xmm8 #29.16
  mulps (%r8,%rax,4), %xmm5 #29.23
  mulps 16(%r8,%rax,4), %xmm6 #29.23
  mulps 32(%r8,%rax,4), %xmm7 #29.23
  addps %xmm5, %xmm0 #29.9
  mulps 48(%r8,%rax,4), %xmm8 #29.23
  addps %xmm6, %xmm4 #29.9
  addps %xmm7, %xmm3 #29.9
  addps %xmm8, %xmm2 #29.9
  addq $16, %rax #28.5
  cmpq %rdx, %rax #28.5
  jb ..B2.12 # Prob 82% #28.5
..B2.14: # Preds ..B2.12 ..B2.9
  addps %xmm4, %xmm0 #27.15
  addps %xmm2, %xmm3 #27.15
  lea 65537(%rsi), %eax #28.5
  addps %xmm3, %xmm0 #27.15
  cmpl $65536, %eax #28.5
  ja ..B2.23 # Prob 50% #28.5
..B2.15: # Preds ..B2.14
  movl %ecx, %eax #28.5
  negl %eax #28.5
  addl $65536, %eax #28.5
  cmpl $4, %eax #28.5
  jb ..B2.25 # Prob 10% #28.5
..B2.16: # Preds ..B2.15
  movl %eax, %edx #28.5
  xorl %r9d, %r9d #28.5
  andl $-4, %edx #28.5
..B2.17: # Preds ..B2.17 ..B2.16
  lea 65536(%r9,%rsi), %r10d #29.9
  addl $4, %r9d #28.5
  movslq %r10d, %r10 #29.9
  movups (%rdi,%r10,4), %xmm2 #29.16
  mulps (%r8,%r10,4), %xmm2 #29.23
  addps %xmm2, %xmm0 #29.9
  cmpl %edx, %r9d #28.5
  jb ..B2.17 # Prob 82% #28.5
..B2.19: # Preds ..B2.17 ..B2.25 ..B2.24
  cmpl %eax, %edx #28.5
  jae ..B2.23 # Prob 0% #28.5
..B2.21: # Preds ..B2.19 ..B2.21
  lea (%rcx,%rdx), %esi #29.9
  incl %edx #28.5
  movslq %esi, %rsi #29.9
  movss (%rdi,%rsi,4), %xmm2 #29.16
  mulss (%r8,%rsi,4), %xmm2 #29.23
  addss %xmm2, %xmm1 #29.9
  cmpl %eax, %edx #28.5
  jb ..B2.21 # Prob 82% #28.5
..B2.23: # Preds ..B2.21 ..B2.14 ..B2.19
  movaps %xmm0, %xmm2 #27.15
  movhlps %xmm0, %xmm2 #27.15
  addps %xmm2, %xmm0 #27.15
  movaps %xmm0, %xmm3 #27.15
  shufps $245, %xmm0, %xmm3 #27.15
  addss %xmm3, %xmm0 #27.15
  addss %xmm1, %xmm0 #27.15
  ret #30.12
..B2.24: # Preds ..B2.2
  xorl %ecx, %ecx #28.5
  movl $65536, %eax #28.5
  xorl %edx, %edx #28.5
  jmp ..B2.19 # Prob 100% #28.5
..B2.25: # Preds ..B2.15
  xorl %edx, %edx #28.5
  jmp ..B2.19 # Prob 100% #28.5
sum_f32:
..B3.1: # Preds ..B3.0
  movq %rdi, %rax #19.5
  andq $15, %rax #19.5
  pxor %xmm1, %xmm1 #18.15
  pxor %xmm0, %xmm0 #18.15
  testl %eax, %eax #19.5
  je ..B3.6 # Prob 50% #19.5
..B3.2: # Preds ..B3.1
  testb $3, %al #19.5
  jne ..B3.19 # Prob 10% #19.5
..B3.3: # Preds ..B3.2
  negl %eax #19.5
  xorl %ecx, %ecx #19.5
  addl $16, %eax #19.5
  shrl $2, %eax #19.5
  movl %eax, %edx #19.5
..B3.4: # Preds ..B3.4 ..B3.3
  addss (%rdi,%rcx,4), %xmm1 #20.9
  incq %rcx #19.5
  cmpq %rdx, %rcx #19.5
  jb ..B3.4 # Prob 82% #19.5
  jmp ..B3.7 # Prob 100% #19.5
..B3.6: # Preds ..B3.1
  xorl %edx, %edx #19.5
..B3.7: # Preds ..B3.4 ..B3.6
  negl %eax #19.5
  movaps %xmm0, %xmm8 #18.15
  andl $31, %eax #19.5
  movaps %xmm8, %xmm7 #18.15
  negl %eax #19.5
  movaps %xmm7, %xmm6 #18.15
  movaps %xmm6, %xmm5 #18.15
  movaps %xmm5, %xmm4 #18.15
  movaps %xmm4, %xmm3 #18.15
  movaps %xmm3, %xmm2 #18.15
  lea 65536(%rax), %ecx #19.5
  movl %ecx, %esi #19.5
..B3.8: # Preds ..B3.8 ..B3.7
  addps (%rdi,%rdx,4), %xmm0 #20.9
  addps 16(%rdi,%rdx,4), %xmm8 #20.9
  addps 32(%rdi,%rdx,4), %xmm7 #20.9
  addps 48(%rdi,%rdx,4), %xmm6 #20.9
  addps 64(%rdi,%rdx,4), %xmm5 #20.9
  addps 80(%rdi,%rdx,4), %xmm4 #20.9
  addps 96(%rdi,%rdx,4), %xmm3 #20.9
  addps 112(%rdi,%rdx,4), %xmm2 #20.9
  addq $32, %rdx #19.5
  cmpq %rsi, %rdx #19.5
  jb ..B3.8 # Prob 82% #19.5
..B3.9: # Preds ..B3.8
  addl $65537, %eax #19.5
  addps %xmm8, %xmm0 #18.15
  addps %xmm6, %xmm7 #18.15
  addps %xmm4, %xmm5 #18.15
  addps %xmm2, %xmm3 #18.15
  addps %xmm7, %xmm0 #18.15
  addps %xmm3, %xmm5 #18.15
  addps %xmm5, %xmm0 #18.15
  cmpl $65536, %eax #19.5
  ja ..B3.18 # Prob 50% #19.5
..B3.10: # Preds ..B3.9
  movl %ecx, %edx #19.5
  negl %edx #19.5
  addl $65536, %edx #19.5
  cmpl $4, %edx #19.5
  jb ..B3.20 # Prob 10% #19.5
..B3.11: # Preds ..B3.10
  movl %edx, %eax #19.5
  xorl %r8d, %r8d #19.5
  andl $-4, %eax #19.5
..B3.12: # Preds ..B3.12 ..B3.11
  addl $4, %r8d #19.5
  addps (%rdi,%rsi,4), %xmm0 #20.9
  addq $4, %rsi #19.5
  cmpl %eax, %r8d #19.5
  jb ..B3.12 # Prob 82% #19.5
..B3.14: # Preds ..B3.12 ..B3.20 ..B3.19
  addl %eax, %ecx #19.5
  cmpl %edx, %eax #19.5
  jae ..B3.18 # Prob 0% #19.5
..B3.16: # Preds ..B3.14 ..B3.16
  incl %eax #19.5
  addss (%rdi,%rcx,4), %xmm1 #20.9
  incq %rcx #19.5
  cmpl %edx, %eax #19.5
  jb ..B3.16 # Prob 82% #19.5
..B3.18: # Preds ..B3.16 ..B3.9 ..B3.14
  movaps %xmm0, %xmm2 #18.15
  movhlps %xmm0, %xmm2 #18.15
  addps %xmm2, %xmm0 #18.15
  movaps %xmm0, %xmm3 #18.15
  shufps $245, %xmm0, %xmm3 #18.15
  addss %xmm3, %xmm0 #18.15
  addss %xmm1, %xmm0 #18.15
  ret #21.12
..B3.19: # Preds ..B3.2
  xorl %ecx, %ecx #19.5
  movl $65536, %edx #19.5
  xorl %eax, %eax #19.5
  jmp ..B3.14 # Prob 100% #19.5
..B3.20: # Preds ..B3.10
  xorl %eax, %eax #19.5
  jmp ..B3.14 # Prob 100% #19.5
input_a:
input_b:
sink:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
.L_2il0floatpacket.6:
.L_2__STRING.0:
