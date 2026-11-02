main:
..B1.1: # Preds ..B1.0
  pushq %rbp #29.33
  movq %rsp, %rbp #29.33
  andq $-128, %rsp #29.33
  pushq %r12 #29.33
  pushq %r13 #29.33
  subq $112, %rsp #29.33
  movq %rsi, %r13 #29.33
  movl %edi, %r12d #29.33
  movl $3, %edi #29.33
  xorl %esi, %esi #29.33
  call __intel_new_feature_proc_init #29.33
..B1.12: # Preds ..B1.1
  stmxcsr (%rsp) #29.33
  movl $1000, %r8d #30.25
  orl $32832, (%rsp) #29.33
  ldmxcsr (%rsp) #29.33
  cmpl $1, %r12d #31.16
  jle ..B1.6 # Prob 78% #31.16
..B1.2: # Preds ..B1.12
  xorl %esi, %esi #32.32
  xorl %edx, %edx #32.32
  movq 8(%r13), %rdi #32.32
  call strtoul #32.32
..B1.13: # Preds ..B1.2
  movq %rax, %r8 #32.32
..B1.3: # Preds ..B1.13
  testq %r8, %r8 #33.23
  je ..B1.9 # Prob 28% #33.23
..B1.4: # Preds ..B1.3
  movq $0x0ffffffff, %rax #33.9
  cmpq %r8, %rax #33.37
  jb ..B1.9 # Prob 28% #33.37
..B1.6: # Preds ..B1.4 ..B1.12
  movdqu .L_2il0floatpacket.0(%rip), %xmm0 #40.35
  lea 16(%rsp), %rdi #44.42
  movdqu .L_2il0floatpacket.1(%rip), %xmm1 #41.35
  lea 32(%rsp), %rsi #44.42
  movdqu .L_2il0floatpacket.2(%rip), %xmm2 #42.34
  lea 48(%rsp), %rdx #44.42
  movdqu %xmm0, -32(%rdx) #40.9
  lea (%rsp), %rcx #44.42
  movdqu %xmm1, -16(%rdx) #41.9
  movdqu %xmm2, (%rdx) #42.9
  call sat_kernel #44.42
..B1.7: # Preds ..B1.6
  movl $.L_2__STRING.0, %edi #44.5
  movq %rax, %rsi #44.5
  xorl %eax, %eax #44.5
  call printf #44.5
..B1.8: # Preds ..B1.7
  xorl %eax, %eax #45.12
  addq $112, %rsp #45.12
  popq %r13 #45.12
  popq %r12 #45.12
  movq %rbp, %rsp #45.12
  popq %rbp #45.12
  ret #45.12
..B1.9: # Preds ..B1.4 ..B1.3
  movl $2, %eax #34.20
  addq $112, %rsp #34.20
  popq %r13 #34.20
  popq %r12 #34.20
  movq %rbp, %rsp #34.20
  popq %rbp #34.20
  ret #34.20
sat_kernel:
..B2.1: # Preds ..B2.0
  testl %r8d, %r8d #13.30
  jbe ..B2.3 # Prob 9% #13.30
..B2.2: # Preds ..B2.1
  movdqu (%rdi), %xmm1 #14.55
  movdqu (%rsi), %xmm0 #15.55
  movdqu (%rdx), %xmm2 #16.55
  paddusb %xmm0, %xmm1 #17.21
  paddusb %xmm1, %xmm2 #18.21
  movdqa %xmm1, %xmm3 #19.21
  pavgb %xmm2, %xmm3 #19.21
  psubq %xmm1, %xmm3 #20.21
  pxor %xmm2, %xmm3 #21.42
  movdqu %xmm3, (%rcx) #21.37
..B2.3: # Preds ..B2.1 ..B2.2
  movq (%rcx), %rax #24.5
..B2.4: # Preds ..B2.3
  movq 8(%rcx), %rdx #25.5
..B2.5: # Preds ..B2.4
  xorq %rdx, %rax #26.17
  ret #26.17
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2__STRING.0:
