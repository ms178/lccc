main:
..B1.1: # Preds ..B1.0
  pushq %rbp #26.33
  movq %rsp, %rbp #26.33
  andq $-128, %rsp #26.33
  pushq %r15 #26.33
  pushq %rbx #26.33
  subq $112, %rsp #26.33
  movq %rsi, %r15 #26.33
  movl %edi, %ebx #26.33
  movl $3, %edi #26.33
  xorl %esi, %esi #26.33
  call __intel_new_feature_proc_init #26.33
..B1.21: # Preds ..B1.1
  stmxcsr (%rsp) #26.33
  orl $32832, (%rsp) #26.33
  ldmxcsr (%rsp) #26.33
  cmpl $1, %ebx #27.30
  jle ..B1.4 # Prob 50% #27.30
..B1.2: # Preds ..B1.21
  movq 8(%r15), %rdi #483.16
  call atol #483.16
..B1.3: # Preds ..B1.2
  movl %eax, %r15d #483.16
  jmp ..B1.5 # Prob 100% #483.16
..B1.4: # Preds ..B1.21
  movl $1, %r15d #27.50
..B1.5: # Preds ..B1.3 ..B1.4
  movq $0x200000001, %rdx #33.29
  movq $0x300000002, %rcx #34.29
  movdqu .L_2il0floatpacket.0(%rip), %xmm2 #33.29
  xorl %eax, %eax #32.5
  movq %rdx, %xmm1 #33.29
  movq %rcx, %xmm0 #34.29
..B1.6: # Preds ..B1.6 ..B1.5
  cvtdq2pd %xmm1, %xmm3 #33.29
  cvtdq2pd %xmm0, %xmm4 #34.29
  paddd %xmm2, %xmm1 #33.29
  paddd %xmm2, %xmm0 #34.29
  cvtdq2pd %xmm1, %xmm5 #33.29
  cvtdq2pd %xmm0, %xmm6 #34.29
  paddd %xmm2, %xmm1 #33.29
  paddd %xmm2, %xmm0 #34.29
  cvtdq2pd %xmm1, %xmm7 #33.29
  cvtdq2pd %xmm0, %xmm8 #34.29
  paddd %xmm2, %xmm1 #33.29
  paddd %xmm2, %xmm0 #34.29
  cvtdq2pd %xmm1, %xmm9 #33.29
  cvtdq2pd %xmm0, %xmm10 #34.29
  movups %xmm3, a(,%rax,8) #33.9
  paddd %xmm2, %xmm1 #33.29
  movups %xmm4, b(,%rax,8) #34.9
  paddd %xmm2, %xmm0 #34.29
  movups %xmm5, 16+a(,%rax,8) #33.9
  movups %xmm6, 16+b(,%rax,8) #34.9
  movups %xmm7, 32+a(,%rax,8) #33.9
  movups %xmm8, 32+b(,%rax,8) #34.9
  movups %xmm9, 48+a(,%rax,8) #33.9
  movups %xmm10, 48+b(,%rax,8) #34.9
  addq $8, %rax #32.5
  cmpq $64, %rax #32.5
  jb ..B1.6 # Prob 98% #32.5
..B1.7: # Preds ..B1.6
  movq $0x4050400000000000, %rdx #33.29
  movq $0x4050800000000000, %rcx #34.29
  movq %rdx, 512+a(%rip) #33.9
  pxor %xmm0, %xmm0 #37.19
  movq %rcx, 512+b(%rip) #34.9
  cmpl $1, %r15d #38.5
  je ..B1.14 # Prob 50% #38.5
..B1.8: # Preds ..B1.7
  movl $0, %ebx #38.5
  jl ..B1.17 # Prob 10% #38.25
..B1.9: # Preds ..B1.8
  movsd %xmm0, 16(%rsp) #[spill]
  movq %r13, (%rsp) #[spill]
  movq %r14, 8(%rsp) #[spill]
..B1.10: # Preds ..B1.12 ..B1.9
  xorl %r13d, %r13d #39.9
..B1.11: # Preds ..B1.23 ..B1.10
  movl $a, %edi #41.23
  movl bounds.295.0.4(,%r13,4), %r14d #40.21
  movl %r14d, %esi #41.23
  call sum_f64 #41.23
..B1.24: # Preds ..B1.11
  movl $a, %edi #42.23
  movl $b, %esi #42.23
  movl %r14d, %edx #42.23
  movsd %xmm0, 24(%rsp) #41.23[spill]
  call dot_f64 #42.23
..B1.23: # Preds ..B1.24
  movsd 24(%rsp), %xmm1 #41.13[spill]
  incl %r13d #39.9
  movsd 16(%rsp), %xmm2 #42.13[spill]
  addsd %xmm0, %xmm1 #41.13
  addsd %xmm1, %xmm2 #42.13
  movsd %xmm2, 16(%rsp) #42.13[spill]
  cmpl $18, %r13d #39.9
  jb ..B1.11 # Prob 82% #39.9
..B1.12: # Preds ..B1.23
  incl %ebx #38.5
  cmpl %r15d, %ebx #38.5
  jb ..B1.10 # Prob 80% #38.5
..B1.13: # Preds ..B1.12
  movaps %xmm2, %xmm0 #
  movq (%rsp), %r13 #[spill]
  movq 8(%rsp), %r14 #[spill]
  jmp ..B1.17 # Prob 100% #
..B1.14: # Preds ..B1.7
  movsd %xmm0, 16(%rsp) #39.9[spill]
  xorl %ebx, %ebx #39.9
..B1.15: # Preds ..B1.25 ..B1.14
  movl $a, %edi #41.23
  movl bounds.295.0.4(,%rbx,4), %r15d #40.21
  movl %r15d, %esi #41.23
  call sum_f64 #41.23
..B1.26: # Preds ..B1.15
  movl $a, %edi #42.23
  movl $b, %esi #42.23
  movl %r15d, %edx #42.23
  movsd %xmm0, (%rsp) #41.23[spill]
  call dot_f64 #42.23
..B1.25: # Preds ..B1.26
  movsd (%rsp), %xmm1 #41.13[spill]
  incl %ebx #39.9
  movsd 16(%rsp), %xmm2 #42.13[spill]
  addsd %xmm0, %xmm1 #41.13
  addsd %xmm1, %xmm2 #42.13
  movsd %xmm2, 16(%rsp) #42.13[spill]
  cmpl $18, %ebx #39.9
  jb ..B1.15 # Prob 82% #39.9
..B1.16: # Preds ..B1.25
  movaps %xmm2, %xmm0 #
..B1.17: # Preds ..B1.16 ..B1.13 ..B1.8
  movl $.L_2__STRING.0, %edi #45.5
  movl $1, %eax #45.5
  call printf #45.5
..B1.18: # Preds ..B1.17
  xorl %eax, %eax #46.12
  addq $112, %rsp #46.12
  popq %rbx #46.12
  popq %r15 #46.12
  movq %rbp, %rsp #46.12
  popq %rbp #46.12
  ret #46.12
bounds.295.0.4:
sum_f64:
..B2.1: # Preds ..B2.0
  pxor %xmm0, %xmm0 #12.16
  testl %esi, %esi #13.25
  jle ..B2.18 # Prob 50% #13.25
..B2.2: # Preds ..B2.1
  movslq %esi, %rax #13.5
  cmpq $8, %rax #13.5
  jl ..B2.19 # Prob 10% #13.5
..B2.3: # Preds ..B2.2
  movq %rdi, %r8 #13.5
  andq $15, %r8 #13.5
  testl %r8d, %r8d #13.5
  je ..B2.6 # Prob 50% #13.5
..B2.4: # Preds ..B2.3
  testl $7, %r8d #13.5
  jne ..B2.19 # Prob 10% #13.5
..B2.5: # Preds ..B2.4
  movl $1, %r8d #13.5
..B2.6: # Preds ..B2.5 ..B2.3
  movl %r8d, %ecx #13.5
  lea 8(%rcx), %rdx #13.5
  cmpq %rdx, %rax #13.5
  jl ..B2.19 # Prob 10% #13.5
..B2.7: # Preds ..B2.6
  movl %esi, %edx #13.5
  subl %r8d, %edx #13.5
  andl $7, %edx #13.5
  subl %edx, %esi #13.5
  xorl %edx, %edx #13.5
  movslq %esi, %rsi #13.5
  testl %r8d, %r8d #13.5
  jbe ..B2.11 # Prob 9% #13.5
..B2.9: # Preds ..B2.7 ..B2.9
  addsd (%rdi,%rdx,8), %xmm0 #14.9
  incq %rdx #13.5
  cmpq %rcx, %rdx #13.5
  jb ..B2.9 # Prob 82% #13.5
..B2.11: # Preds ..B2.9 ..B2.7
  pxor %xmm2, %xmm2 #12.16
  movaps %xmm2, %xmm3 #12.16
  movaps %xmm2, %xmm1 #12.16
  movsd %xmm0, %xmm3 #12.16
  movaps %xmm1, %xmm0 #12.16
..B2.12: # Preds ..B2.12 ..B2.11
  addpd (%rdi,%rcx,8), %xmm3 #14.9
  addpd 16(%rdi,%rcx,8), %xmm2 #14.9
  addpd 32(%rdi,%rcx,8), %xmm1 #14.9
  addpd 48(%rdi,%rcx,8), %xmm0 #14.9
  addq $8, %rcx #13.5
  cmpq %rsi, %rcx #13.5
  jb ..B2.12 # Prob 82% #13.5
..B2.13: # Preds ..B2.12
  addpd %xmm2, %xmm3 #12.16
  addpd %xmm0, %xmm1 #12.16
  addpd %xmm1, %xmm3 #12.16
  movaps %xmm3, %xmm0 #12.16
  unpckhpd %xmm3, %xmm0 #12.16
  addsd %xmm0, %xmm3 #12.16
  movaps %xmm3, %xmm0 #12.16
..B2.14: # Preds ..B2.13 ..B2.19
  cmpq %rax, %rsi #13.5
  jae ..B2.18 # Prob 9% #13.5
..B2.16: # Preds ..B2.14 ..B2.16
  addsd (%rdi,%rsi,8), %xmm0 #14.9
  incq %rsi #13.5
  cmpq %rax, %rsi #13.5
  jb ..B2.16 # Prob 82% #13.5
..B2.18: # Preds ..B2.16 ..B2.14 ..B2.1
  ret #15.12
..B2.19: # Preds ..B2.2 ..B2.4 ..B2.6
  xorl %esi, %esi #13.5
  jmp ..B2.14 # Prob 100% #13.5
dot_f64:
..B3.1: # Preds ..B3.0
  movl %edx, %ecx #19.82
  movq %rsi, %r8 #19.82
  pxor %xmm0, %xmm0 #20.16
  testl %ecx, %ecx #21.25
  jle ..B3.23 # Prob 50% #21.25
..B3.2: # Preds ..B3.1
  movslq %ecx, %rsi #21.5
  cmpq $8, %rsi #21.5
  jl ..B3.24 # Prob 10% #21.5
..B3.3: # Preds ..B3.2
  movq %r8, %rdx #21.5
  andq $15, %rdx #21.5
  testl %edx, %edx #21.5
  je ..B3.6 # Prob 50% #21.5
..B3.4: # Preds ..B3.3
  testb $7, %dl #21.5
  jne ..B3.24 # Prob 10% #21.5
..B3.5: # Preds ..B3.4
  movl $1, %edx #21.5
..B3.6: # Preds ..B3.5 ..B3.3
  movl %edx, %eax #21.5
  lea 8(%rax), %r9 #21.5
  cmpq %r9, %rsi #21.5
  jl ..B3.24 # Prob 10% #21.5
..B3.7: # Preds ..B3.6
  movl %ecx, %r9d #21.5
  subl %edx, %r9d #21.5
  andl $7, %r9d #21.5
  subl %r9d, %ecx #21.5
  xorl %r9d, %r9d #21.5
  movslq %ecx, %rcx #21.5
  testl %edx, %edx #21.5
  jbe ..B3.11 # Prob 9% #21.5
..B3.9: # Preds ..B3.7 ..B3.9
  movsd (%rdi,%r9,8), %xmm1 #22.16
  mulsd (%r8,%r9,8), %xmm1 #22.23
  incq %r9 #21.5
  addsd %xmm1, %xmm0 #22.9
  cmpq %rax, %r9 #21.5
  jb ..B3.9 # Prob 82% #21.5
..B3.11: # Preds ..B3.9 ..B3.7
  lea (%rdi,%rax,8), %rdx #22.16
  pxor %xmm2, %xmm2 #20.16
  pxor %xmm3, %xmm3 #20.16
  movaps %xmm2, %xmm1 #20.16
  movsd %xmm0, %xmm3 #20.16
  movaps %xmm1, %xmm0 #20.16
  testq $15, %rdx #21.5
  je ..B3.16 # Prob 60% #21.5
..B3.13: # Preds ..B3.11 ..B3.13
  movups (%rdi,%rax,8), %xmm4 #22.16
  movups 16(%rdi,%rax,8), %xmm5 #22.16
  movups 32(%rdi,%rax,8), %xmm6 #22.16
  movups 48(%rdi,%rax,8), %xmm7 #22.16
  mulpd (%r8,%rax,8), %xmm4 #22.23
  mulpd 16(%r8,%rax,8), %xmm5 #22.23
  mulpd 32(%r8,%rax,8), %xmm6 #22.23
  mulpd 48(%r8,%rax,8), %xmm7 #22.23
  addpd %xmm4, %xmm3 #22.9
  addpd %xmm5, %xmm2 #22.9
  addpd %xmm6, %xmm1 #22.9
  addpd %xmm7, %xmm0 #22.9
  addq $8, %rax #21.5
  cmpq %rcx, %rax #21.5
  jb ..B3.13 # Prob 82% #21.5
  jmp ..B3.18 # Prob 100% #21.5
..B3.16: # Preds ..B3.11 ..B3.16
  movups (%rdi,%rax,8), %xmm4 #22.16
  movups 16(%rdi,%rax,8), %xmm5 #22.16
  movups 32(%rdi,%rax,8), %xmm6 #22.16
  movups 48(%rdi,%rax,8), %xmm7 #22.16
  mulpd (%r8,%rax,8), %xmm4 #22.23
  mulpd 16(%r8,%rax,8), %xmm5 #22.23
  mulpd 32(%r8,%rax,8), %xmm6 #22.23
  mulpd 48(%r8,%rax,8), %xmm7 #22.23
  addpd %xmm4, %xmm3 #22.9
  addpd %xmm5, %xmm2 #22.9
  addpd %xmm6, %xmm1 #22.9
  addpd %xmm7, %xmm0 #22.9
  addq $8, %rax #21.5
  cmpq %rcx, %rax #21.5
  jb ..B3.16 # Prob 82% #21.5
..B3.18: # Preds ..B3.16 ..B3.13
  addpd %xmm2, %xmm3 #20.16
  addpd %xmm0, %xmm1 #20.16
  addpd %xmm1, %xmm3 #20.16
  movaps %xmm3, %xmm0 #20.16
  unpckhpd %xmm3, %xmm0 #20.16
  addsd %xmm0, %xmm3 #20.16
  movaps %xmm3, %xmm0 #20.16
..B3.19: # Preds ..B3.18 ..B3.24
  cmpq %rsi, %rcx #21.5
  jae ..B3.23 # Prob 9% #21.5
..B3.21: # Preds ..B3.19 ..B3.21
  movsd (%rdi,%rcx,8), %xmm1 #22.16
  mulsd (%r8,%rcx,8), %xmm1 #22.23
  incq %rcx #21.5
  addsd %xmm1, %xmm0 #22.9
  cmpq %rsi, %rcx #21.5
  jb ..B3.21 # Prob 82% #21.5
..B3.23: # Preds ..B3.21 ..B3.19 ..B3.1
  ret #23.12
..B3.24: # Preds ..B3.2 ..B3.4 ..B3.6
  xorl %ecx, %ecx #21.5
  jmp ..B3.19 # Prob 100% #21.5
a:
b:
.L_2il0floatpacket.0:
.L_2__STRING.0:
