main:
..B1.1: # Preds ..B1.0
  pushq %rbp #125.1
  movq %rsp, %rbp #125.1
  andq $-128, %rsp #125.1
  pushq %r12 #125.1
  subq $120, %rsp #125.1
  movl $3, %edi #125.1
  xorl %esi, %esi #125.1
  call __intel_new_feature_proc_init #125.1
..B1.13: # Preds ..B1.1
  stmxcsr 64(%rsp) #125.1
  movq $0x102030405060708, %rax #108.3
  movq $-1, %rcx #109.3
  movl $7, %r8d #110.3
  pxor %xmm1, %xmm1 #107.3
  orl $32832, 64(%rsp) #125.1
  lea (%rsp), %rdi #115.7
  movl $4, %edx #115.7
  lea 32(%rsp), %rsi #115.7
  movq %rax, %xmm0 #108.3
  movq %rcx, %xmm3 #109.3
  movq %r8, %xmm2 #110.3
  xorl %r12d, %r12d #127.26
  unpcklpd %xmm0, %xmm1 #111.14
  unpcklpd %xmm2, %xmm3 #111.14
  ldmxcsr 32(%rsi) #125.1
  movq $0, (%rdi) #107.3
  movq %rax, -24(%rsi) #108.3
  movq %rcx, -16(%rsi) #109.3
  movq %r8, -8(%rsi) #110.3
  movups %xmm1, (%rsi) #111.3
  movups %xmm3, 16(%rsi) #111.3
  call glibc_memcmp_common_alignment..0 #115.7
..B1.12: # Preds ..B1.13
  testl %eax, %eax #115.58
  jne ..B1.3 # Prob 57% #115.58
..B1.2: # Preds ..B1.12
  movl $4, %edx #118.7
  lea (%rsp), %rdi #118.7
  movq $0x102030405060709, %rax #117.3
  lea 32(%rsp), %rsi #118.7
  movq %rax, 8(%rsi) #117.3
  call glibc_memcmp_common_alignment..0 #118.7
..B1.14: # Preds ..B1.2
  testl %eax, %eax #118.58
  jl ..B1.4 # Prob 16% #118.58
..B1.3: # Preds ..B1.12 ..B1.14
  movl $2, %eax #130.12
  addq $120, %rsp #130.12
  popq %r12 #130.12
  movq %rbp, %rsp #130.12
  popq %rbp #130.12
  ret #130.12
..B1.4: # Preds ..B1.14
  movq $0x9e3779b97f4a7c15, %rcx #90.23
  xorl %edx, %edx #92.3
  xorl %eax, %eax #93.5
..B1.5: # Preds ..B1.5 ..B1.4
  movq %rcx, %rdi #93.23
  incl %edx #92.3
  shlq $7, %rdi #93.23
  xorq %rdi, %rcx #93.5
  movq %rcx, %r8 #94.23
  shrq $9, %r8 #94.23
  xorq %r8, %rcx #94.5
  movq %rcx, %r9 #95.23
  shlq $8, %r9 #95.23
  xorq %r9, %rcx #95.5
  movq %rcx, %r10 #93.23
  shlq $7, %r10 #93.23
  movq %rcx, glibc_left(%rax) #96.5
  movq %rcx, glibc_right(%rax) #97.5
  xorq %r10, %rcx #93.5
  movq %rcx, %r11 #94.23
  shrq $9, %r11 #94.23
  xorq %r11, %rcx #94.5
  movq %rcx, %rdi #95.23
  shlq $8, %rdi #95.23
  xorq %rdi, %rcx #95.5
  movq %rcx, 8+glibc_left(%rax) #96.5
  movq %rcx, 8+glibc_right(%rax) #97.5
  addq $16, %rax #92.3
  cmpl $4096, %edx #92.3
  jb ..B1.5 # Prob 99% #92.3
..B1.6: # Preds ..B1.5
  xorl %edx, %edx #133.8
  xorl %eax, %eax #133.33
  movq %r13, 88(%rsp) #133.33[spill]
  movq %r14, 80(%rsp) #133.33[spill]
  movq %rdx, %r14 #133.33
  movq %r15, 72(%rsp) #133.33[spill]
  movq %rbx, 64(%rsp) #133.33[spill]
  movl %eax, %ebx #133.33
..B1.7: # Preds ..B1.15 ..B1.6
  imulq $4051, %r14, %r13 #134.49
  andq $8191, %r13 #134.59
  lea (%r14,%r14,8), %rcx #135.57
  movl $glibc_left, %edi #140.14
  lea (%rcx,%r14,2), %r8 #135.57
  movl $glibc_right, %esi #140.14
  movl $8192, %edx #140.14
  movq glibc_left(,%r13,8), %r15 #139.25
  movq %r15, %r9 #139.44
  btcq %r8, %r9 #139.44
  movq %r9, glibc_right(,%r13,8) #139.5
  call glibc_memcmp_common_alignment..1 #140.14
..B1.15: # Preds ..B1.7
  incq %r14 #141.72
  incl %ebx #133.33
  movslq %eax, %rax #141.42
  addq $257, %rax #141.42
  imulq %r14, %rax #141.72
  movl %ebx, %r14d #133.33
  addq %rax, %r12 #141.5
  movq %r15, glibc_right(,%r13,8) #142.5
  cmpl $4096, %ebx #133.25
  jb ..B1.7 # Prob 99% #133.25
..B1.8: # Preds ..B1.15
  movl $.L_2__STRING.0, %edi #145.3
  movq %r12, %rsi #145.3
  xorl %eax, %eax #145.3
  movq 88(%rsp), %r13 #[spill]
  movq 80(%rsp), %r14 #[spill]
  movq 72(%rsp), %r15 #[spill]
  movq 64(%rsp), %rbx #[spill]
  call printf #145.3
..B1.9: # Preds ..B1.8
  xorl %eax, %eax #146.10
  addq $120, %rsp #146.10
  popq %r12 #146.10
  movq %rbp, %rsp #146.10
  popq %rbp #146.10
  ret #146.10
glibc_memcmp_common_alignment..1:
..B2.1: # Preds ..B2.0
  movq %rdi, %r10 #51.1
  movq %r12, -16(%rsp) #51.1[spill]
  movl $8192, %r11d #51.1
  movq %r13, -24(%rsp) #51.1[spill]
..B2.2: # Preds ..B2.6 ..B2.1
  movq (%r10), %r9 #53.24
  movq (%rsi), %r8 #57.24
  movq 8(%rsi), %rdi #58.24
  movq 16(%rsi), %rax #59.24
  movq 24(%rsi), %r12 #60.24
  movq 8(%r10), %rcx #54.24
  movq 16(%r10), %rdx #55.24
  movq 24(%r10), %r13 #56.24
  cmpq %r8, %r9 #62.15
  jne ..B2.37 # Prob 20% #62.15
..B2.3: # Preds ..B2.2
  cmpq %rdi, %rcx #64.15
  jne ..B2.31 # Prob 20% #64.15
..B2.4: # Preds ..B2.3
  cmpq %rax, %rdx #66.15
  jne ..B2.25 # Prob 20% #66.15
..B2.5: # Preds ..B2.4
  cmpq %r12, %r13 #68.15
  jne ..B2.19 # Prob 20% #68.15
..B2.6: # Preds ..B2.5
  addq $-4, %r11 #73.5
  addq $32, %r10 #71.5
  addq $32, %rsi #72.5
  cmpq $4, %r11 #52.19
  jae ..B2.2 # Prob 82% #52.19
..B2.7: # Preds ..B2.6
  movq -16(%rsp), %r12 #[spill]
  movq -24(%rsp), %r13 #[spill]
  testq %r11, %r11 #76.10
  je ..B2.12 # Prob 4% #76.10
..B2.9: # Preds ..B2.7 ..B2.10
  movq (%r10), %rdx #77.10
  movq (%rsi), %rax #77.19
  cmpq %rax, %rdx #77.19
  jne ..B2.13 # Prob 20% #77.19
..B2.10: # Preds ..B2.9
  addq $8, %r10 #79.5
  addq $8, %rsi #80.5
  decq %r11 #81.5
  jne ..B2.9 # Prob 82% #76.10
..B2.12: # Preds ..B2.10 ..B2.7
  xorl %eax, %eax #83.10
  ret #83.10
..B2.13: # Preds ..B2.9
  movq %rdx, -40(%rsp) #33.1
  xorl %edx, %edx #39.8
  movq %rax, -32(%rsp) #33.1
..B2.14: # Preds ..B2.15 ..B2.13
  movb -40(%rsp,%rdx), %al #40.9
  cmpb -32(%rsp,%rdx), %al #40.18
  jne ..B2.18 # Prob 20% #40.18
..B2.15: # Preds ..B2.14
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B2.14 # Prob 82% #39.21
..B2.16: # Preds ..B2.15
  xorl %eax, %eax #78.14
..B2.17: # Preds ..B2.18 ..B2.16
  ret #78.14
..B2.18: # Preds ..B2.14
  movzbl -40(%rsp,%rdx), %eax #41.19
  movzbl -32(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  jmp ..B2.17 # Prob 100% #41.19
..B2.19: # Preds ..B2.5
  movq %r12, %rax #
  movq %r13, %rdx #
  movq -16(%rsp), %r12 #[spill]
  movq -24(%rsp), %r13 #[spill]
  movq %rdx, -40(%rsp) #33.1
  xorl %edx, %edx #39.8
  movq %rax, -32(%rsp) #33.1
..B2.20: # Preds ..B2.21 ..B2.19
  movb -40(%rsp,%rdx), %al #40.9
  cmpb -32(%rsp,%rdx), %al #40.18
  jne ..B2.24 # Prob 20% #40.18
..B2.21: # Preds ..B2.20
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B2.20 # Prob 82% #39.21
..B2.22: # Preds ..B2.21
  xorl %eax, %eax #69.14
..B2.23: # Preds ..B2.24 ..B2.22
  ret #69.14
..B2.24: # Preds ..B2.20
  movzbl -40(%rsp,%rdx), %eax #41.19
  movzbl -32(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  jmp ..B2.23 # Prob 100% #41.19
..B2.25: # Preds ..B2.4
  movq -16(%rsp), %r12 #[spill]
  movq -24(%rsp), %r13 #[spill]
  movq %rdx, -40(%rsp) #33.1
  xorl %edx, %edx #39.8
  movq %rax, -32(%rsp) #33.1
..B2.26: # Preds ..B2.27 ..B2.25
  movb -40(%rsp,%rdx), %al #40.9
  cmpb -32(%rsp,%rdx), %al #40.18
  jne ..B2.30 # Prob 20% #40.18
..B2.27: # Preds ..B2.26
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B2.26 # Prob 82% #39.21
..B2.28: # Preds ..B2.27
  xorl %eax, %eax #67.14
..B2.29: # Preds ..B2.30 ..B2.28
  ret #67.14
..B2.30: # Preds ..B2.26
  movzbl -40(%rsp,%rdx), %eax #41.19
  movzbl -32(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  jmp ..B2.29 # Prob 100% #41.19
..B2.31: # Preds ..B2.3
  movq -16(%rsp), %r12 #[spill]
  xorl %edx, %edx #39.8
  movq -24(%rsp), %r13 #[spill]
  movq %rcx, -40(%rsp) #33.1
  movq %rdi, -32(%rsp) #33.1
..B2.32: # Preds ..B2.33 ..B2.31
  movb -40(%rsp,%rdx), %al #40.9
  cmpb -32(%rsp,%rdx), %al #40.18
  jne ..B2.36 # Prob 20% #40.18
..B2.33: # Preds ..B2.32
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B2.32 # Prob 82% #39.21
..B2.34: # Preds ..B2.33
  xorl %eax, %eax #65.14
..B2.35: # Preds ..B2.36 ..B2.34
  ret #65.14
..B2.36: # Preds ..B2.32
  movzbl -40(%rsp,%rdx), %eax #41.19
  movzbl -32(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  jmp ..B2.35 # Prob 100% #41.19
..B2.37: # Preds ..B2.2
  movq -16(%rsp), %r12 #[spill]
  xorl %edx, %edx #39.8
  movq -24(%rsp), %r13 #[spill]
  movq %r9, -40(%rsp) #33.1
  movq %r8, -32(%rsp) #33.1
..B2.38: # Preds ..B2.39 ..B2.37
  movb -40(%rsp,%rdx), %al #40.9
  cmpb -32(%rsp,%rdx), %al #40.18
  jne ..B2.42 # Prob 20% #40.18
..B2.39: # Preds ..B2.38
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B2.38 # Prob 82% #39.21
..B2.40: # Preds ..B2.39
  xorl %eax, %eax #63.14
..B2.41: # Preds ..B2.42 ..B2.40
  ret #63.14
..B2.42: # Preds ..B2.38
  movzbl -40(%rsp,%rdx), %eax #41.19
  movzbl -32(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  jmp ..B2.41 # Prob 100% #41.19
glibc_memcmp_common_alignment..0:
..B3.1: # Preds ..B3.0
  movq (%rdi), %rcx #53.24
  movq (%rsi), %r10 #57.24
  movq 8(%rsi), %r9 #58.24
  movq 16(%rsi), %r8 #59.24
  movq 8(%rdi), %rdx #54.24
  movq 16(%rdi), %rax #55.24
  movq 24(%rdi), %rdi #56.24
  movq 24(%rsi), %rsi #60.24
  cmpq %r10, %rcx #62.15
  jne ..B3.24 # Prob 20% #62.15
..B3.2: # Preds ..B3.1
  cmpq %r9, %rdx #64.15
  jne ..B3.18 # Prob 20% #64.15
..B3.3: # Preds ..B3.2
  cmpq %r8, %rax #66.15
  jne ..B3.12 # Prob 20% #66.15
..B3.4: # Preds ..B3.3
  cmpq %rsi, %rdi #68.15
  jne ..B3.6 # Prob 20% #68.15
..B3.5: # Preds ..B3.4
  xorl %eax, %eax #83.10
  ret #83.10
..B3.6: # Preds ..B3.4
  movq %rdi, -24(%rsp) #33.1
  xorl %edx, %edx #39.8
  movq %rsi, -16(%rsp) #33.1
..B3.7: # Preds ..B3.8 ..B3.6
  movb -24(%rsp,%rdx), %al #40.9
  cmpb -16(%rsp,%rdx), %al #40.18
  jne ..B3.11 # Prob 20% #40.18
..B3.8: # Preds ..B3.7
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B3.7 # Prob 82% #39.21
..B3.9: # Preds ..B3.8
  xorl %eax, %eax #69.14
..B3.10: # Preds ..B3.9
  ret #69.14
..B3.11: # Preds ..B3.7
  movzbl -24(%rsp,%rdx), %eax #41.19
  movzbl -16(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  ret #41.19
..B3.12: # Preds ..B3.3
  movq %rax, -24(%rsp) #33.1
  xorl %edx, %edx #39.8
  movq %r8, -16(%rsp) #33.1
..B3.13: # Preds ..B3.14 ..B3.12
  movb -24(%rsp,%rdx), %al #40.9
  cmpb -16(%rsp,%rdx), %al #40.18
  jne ..B3.17 # Prob 20% #40.18
..B3.14: # Preds ..B3.13
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B3.13 # Prob 82% #39.21
..B3.15: # Preds ..B3.14
  xorl %eax, %eax #67.14
..B3.16: # Preds ..B3.15
  ret #67.14
..B3.17: # Preds ..B3.13
  movzbl -24(%rsp,%rdx), %eax #41.19
  movzbl -16(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  ret #41.19
..B3.18: # Preds ..B3.2
  movq %rdx, -24(%rsp) #33.1
  xorl %edx, %edx #39.8
  movq %r9, -16(%rsp) #33.1
..B3.19: # Preds ..B3.20 ..B3.18
  movb -24(%rsp,%rdx), %al #40.9
  cmpb -16(%rsp,%rdx), %al #40.18
  jne ..B3.23 # Prob 20% #40.18
..B3.20: # Preds ..B3.19
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B3.19 # Prob 82% #39.21
..B3.21: # Preds ..B3.20
  xorl %eax, %eax #65.14
..B3.22: # Preds ..B3.21
  ret #65.14
..B3.23: # Preds ..B3.19
  movzbl -24(%rsp,%rdx), %eax #41.19
  movzbl -16(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  ret #41.19
..B3.24: # Preds ..B3.1
  movq %rcx, -24(%rsp) #33.1
  xorl %edx, %edx #39.8
  movq %r10, -16(%rsp) #33.1
..B3.25: # Preds ..B3.26 ..B3.24
  movb -24(%rsp,%rdx), %al #40.9
  cmpb -16(%rsp,%rdx), %al #40.18
  jne ..B3.29 # Prob 20% #40.18
..B3.26: # Preds ..B3.25
  incq %rdx #39.44
  cmpq $8, %rdx #39.21
  jb ..B3.25 # Prob 82% #39.21
..B3.27: # Preds ..B3.26
  xorl %eax, %eax #63.14
..B3.28: # Preds ..B3.27
  ret #63.14
..B3.29: # Preds ..B3.25
  movzbl -24(%rsp,%rdx), %eax #41.19
  movzbl -16(%rsp,%rdx), %edx #41.32
  subl %edx, %eax #41.19
  ret #41.19
glibc_left:
glibc_right:
.L_2__STRING.0:
