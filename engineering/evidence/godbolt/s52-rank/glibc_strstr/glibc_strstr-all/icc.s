main:
..B1.1: # Preds ..B1.0
  pushq %rbp #78.1
  movq %rsp, %rbp #78.1
  andq $-128, %rsp #78.1
  pushq %r12 #78.1
  pushq %r13 #78.1
  pushq %r14 #78.1
  pushq %r15 #78.1
  pushq %rbx #78.1
  subq $344, %rsp #78.1
  movl $3, %edi #78.1
  xorl %esi, %esi #78.1
  call __intel_new_feature_proc_init #78.1
..B1.47: # Preds ..B1.1
  stmxcsr (%rsp) #78.1
  xorl %esi, %esi #80.16
  movl $523124044, %r9d #81.22
  orl $32832, (%rsp) #78.1
  movl $2084213038, %edi #67.22
  ldmxcsr (%rsp) #78.1
  xorl %ecx, %ecx #69.3
..B1.2: # Preds ..B1.2 ..B1.47
  imull $1664525, %edi, %r10d #70.21
  movl $1321528399, %eax #71.39
  lea 1013904223(%r10), %edi #70.32
  mull %edi #71.39
  shrl $3, %edx #71.39
  imull $-26, %edx, %r8d #71.39
  lea 1013904320(%r8,%r10), %r11d #71.39
  movb %r11b, haystack(%rcx) #71.5
  incl %ecx #69.3
  cmpl $524288, %ecx #69.3
  jb ..B1.2 # Prob 82% #69.3
..B1.3: # Preds ..B1.2
  xorl %ecx, %ecx #85.3
  movb $0, 524288+haystack(%rip) #73.3
  xorl %edi, %edi #86.10
..B1.4: # Preds ..B1.33 ..B1.3
  movl %edi, 8(%rsp) #86.5[spill]
  xorl %r8d, %r8d #86.5
  movl %ecx, (%rsp) #86.5[spill]
..B1.5: # Preds ..B1.32 ..B1.4
  imull $1664525, %r9d, %r9d #91.23
  movl %r8d, %ebx #88.36
  andl $7, %ebx #88.36
  addl $1013904223, %r9d #91.34
  lea 4(%rbx), %edi #88.36
  testl $1, %r8d #93.15
  je ..B1.9 # Prob 50% #93.15
..B1.6: # Preds ..B1.5
  imull $31337, %r9d, %ecx #94.37
  movq $0x4001000400101, %r10 #94.47
  movq %rcx, %rax #94.47
  movq %rcx, %r11 #94.47
  mulq %r10 #94.47
  subq %rdx, %r11 #94.47
  shrq $1, %r11 #94.47
  addq %r11, %rdx #94.47
  shrq $18, %rdx #94.47
  imulq $-524256, %rdx, %r12 #94.47
  addq %r12, %rcx #94.47
..B1.7: # Preds ..B1.6
  jmp ..B1.36 # Prob 100% #95.9
..B1.9: # Preds ..B1.5
  xorl %r10d, %r10d #98.9
  xorl %ecx, %ecx #99.11
..B1.11: # Preds ..B1.9 ..B1.11
  movl %r9d, %r11d #99.50
  movl $1321528399, %eax #99.56
  shrl %cl, %r11d #99.50
  addl $3, %ecx #98.9
  mull %r11d #99.56
  shrl $3, %edx #99.56
  imull $-26, %edx, %r12d #99.56
  lea 97(%r11,%r12), %r13d #99.56
  movb %r13b, 32(%rsp,%r10) #99.11
  incl %r10d #98.9
  cmpl %edi, %r10d #98.9
  jb ..B1.11 # Prob 93% #98.9
..B1.13: # Preds ..B1.11 ..B1.40 ..B1.41
  movd %edi, %xmm0 #47.26
  xorl %eax, %eax #46.3
  punpcklbw %xmm0, %xmm0 #47.26
  punpcklwd %xmm0, %xmm0 #47.26
  punpckldq %xmm0, %xmm0 #47.26
  punpcklqdq %xmm0, %xmm0 #47.26
..B1.14: # Preds ..B1.14 ..B1.13
  lea 16(%rax), %edx #46.3
  lea 32(%rax), %ecx #46.3
  lea 48(%rax), %r10d #46.3
  movdqu %xmm0, 48(%rsp,%rax) #47.5
  lea 64(%rax), %r11d #46.3
  movdqu %xmm0, 48(%rsp,%rdx) #47.5
  lea 80(%rax), %r12d #46.3
  movdqu %xmm0, 48(%rsp,%rcx) #47.5
  lea 96(%rax), %r13d #46.3
  movdqu %xmm0, 48(%rsp,%r10) #47.5
  lea 112(%rax), %r14d #46.3
  addl $128, %eax #46.3
  movdqu %xmm0, 48(%rsp,%r11) #47.5
  movdqu %xmm0, 48(%rsp,%r12) #47.5
  movdqu %xmm0, 48(%rsp,%r13) #47.5
  movdqu %xmm0, 48(%rsp,%r14) #47.5
  cmpl $256, %eax #46.3
  jb ..B1.14 # Prob 99% #46.3
..B1.16: # Preds ..B1.14
  xorl %edx, %edx #48.3
  lea 3(%rbx), %eax #48.3
  movl %eax, %ecx #48.3
  shrl $1, %ecx #48.3
..B1.18: # Preds ..B1.16 ..B1.18
  movl %edi, %r10d #49.48
  lea (%rdx,%rdx), %r11d #49.5
  subl %r11d, %r10d #49.48
  lea 1(%rdx,%rdx), %r14d #49.17
  movzbl 32(%rsp,%r11), %r13d #102.62
  incl %edx #48.3
  movzbl 32(%rsp,%r14), %r15d #102.62
  lea -1(%r10), %r12d #49.48
  addl $-2, %r10d #49.48
  movb %r12b, 48(%rsp,%r13) #49.5
  movb %r10b, 48(%rsp,%r15) #49.5
  cmpl %ecx, %edx #48.3
  jb ..B1.18 # Prob 63% #48.3
..B1.19: # Preds ..B1.18
  lea 1(%rdx,%rdx), %ecx #49.5
..B1.20: # Preds ..B1.19
  lea -1(%rcx), %edx #48.3
  cmpl %eax, %edx #48.3
  jae ..B1.22 # Prob 9% #48.3
..B1.21: # Preds ..B1.20
  negl %ecx #49.48
  addl %edi, %ecx #49.48
  movzbl 32(%rsp,%rdx), %r10d #102.62
  movb %cl, 48(%rsp,%r10) #49.5
..B1.22: # Preds ..B1.20 ..B1.21
  movl %edi, %r12d #102.48
  xorl %r13d, %r13d #51.18
  negl %r12d #102.48
  movl %edi, %r14d #54.5
  addl $524288, %r12d #102.48
  movq %rsi, 16(%rsp) #54.5[spill]
..B1.23: # Preds ..B1.29 ..B1.22
  movl %eax, %ecx #53.31
  xorl %esi, %esi #54.5
  movl %edi, %r10d #54.5
  movq %r14, %rdx #54.5
..B1.24: # Preds ..B1.23
  lea 3(%r13,%rbx), %r15d #54.5
..B1.25: # Preds ..B1.26 ..B1.24
  movb 31(%rsp,%rdx), %r11b #102.62
  cmpb haystack(%r15), %r11b #54.35
  jne ..B1.28 # Prob 20% #54.35
..B1.26: # Preds ..B1.25
  incl %esi #54.5
  decl %r15d #54.5
  decq %rdx #54.5
  lea -2(%r10), %ecx #55.7
  decl %r10d #54.5
  cmpl %edi, %esi #54.5
  jb ..B1.25 # Prob 82% #54.5
..B1.28: # Preds ..B1.25 ..B1.26
  testl %ecx, %ecx #56.13
  jl ..B1.42 # Prob 20% #56.13
..B1.29: # Preds ..B1.28
  lea 3(%r13,%rbx), %edx #58.52
  movzbl haystack(%rdx), %ecx #102.38
  movzbl 48(%rsp,%rcx), %esi #58.10
  addl %esi, %r13d #58.5
  cmpl %r12d, %r13d #52.24
  jbe ..B1.23 # Prob 82% #52.24
..B1.30: # Preds ..B1.29
  movq 16(%rsp), %rsi #[spill]
..B1.31: # Preds ..B1.42 ..B1.30
  imulq $104729, %r8, %rax #106.30
  addq %rax, %rsi #106.9
..B1.32: # Preds ..B1.43 ..B1.31
  incl %r8d #86.5
  cmpl $2048, %r8d #86.5
  jb ..B1.5 # Prob 99% #86.5
..B1.33: # Preds ..B1.32
  movl $1321528399, %eax #108.72
  movl (%rsp), %ecx #[spill]
  mull %ecx #108.72
  movl 8(%rsp), %edi #[spill]
  shrl $3, %edx #108.72
  imull $-26, %edx, %ebx #108.72
  movl %edi, %r10d #108.22
  lea 97(%rcx,%rbx), %r8d #108.72
  andq $524287, %r10 #108.32
  incl %ecx #85.3
  addl $16381, %edi #85.3
  movb %r8b, haystack(%r10) #108.5
  cmpl $8, %ecx #85.3
  jb ..B1.4 # Prob 87% #85.3
..B1.34: # Preds ..B1.33
  movl $.L_2__STRING.0, %edi #111.3
  xorl %eax, %eax #111.3
  call printf #111.3
..B1.35: # Preds ..B1.34
  xorl %eax, %eax #112.10
  addq $344, %rsp #112.10
  popq %rbx #112.10
  popq %r15 #112.10
  popq %r14 #112.10
  popq %r13 #112.10
  popq %r12 #112.10
  movq %rbp, %rsp #112.10
  popq %rbp #112.10
  ret #112.10
..B1.36: # Preds ..B1.7
  movl %edi, %eax #88.25
  xorl %edx, %edx #95.9
  shrl $1, %eax #88.25
..B1.38: # Preds ..B1.36 ..B1.38
  lea (%rcx,%rdx,2), %r11d #96.23
  movb haystack(%r11), %r12b #96.23
  lea (%rdx,%rdx), %r10d #96.23
  incl %r11d #96.23
  lea 1(%rdx,%rdx), %r13d #96.11
  incl %edx #95.9
  movb haystack(%r11), %r14b #96.23
  movb %r12b, 32(%rsp,%r10) #96.11
  movb %r14b, 32(%rsp,%r13) #96.11
  cmpl %eax, %edx #95.9
  jb ..B1.38 # Prob 87% #95.9
..B1.39: # Preds ..B1.38
  lea 1(%rdx,%rdx), %eax #96.11
..B1.40: # Preds ..B1.39
  lea -1(%rax), %edx #95.9
  cmpl %edi, %edx #95.9
  jae ..B1.13 # Prob 10% #95.9
..B1.41: # Preds ..B1.40
  lea -1(%rax,%rcx), %eax #96.23
  movb haystack(%rax), %cl #96.23
  movb %cl, 32(%rsp,%rdx) #96.11
  jmp ..B1.13 # Prob 100% #96.11
..B1.42: # Preds ..B1.28
  movslq %r13d, %r13 #102.17
  movq 16(%rsp), %rsi #[spill]
  testq %r13, %r13 #103.18
  jl ..B1.31 # Prob 16% #103.18
..B1.43: # Preds ..B1.42
  movl %r8d, %eax #104.38
  shlq $24, %rax #104.43
  xorq %rax, %r13 #104.43
  addq %r13, %rsi #104.9
  jmp ..B1.32 # Prob 100% #104.9
haystack:
.L_2__STRING.0:
