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
