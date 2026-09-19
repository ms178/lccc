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
..B1.46: # Preds ..B1.1
  stmxcsr (%rsp) #78.1
  xorl %esi, %esi #80.16
  movl $523124044, %edi #81.22
  orl $32832, (%rsp) #78.1
  movl $2084213038, %r8d #67.22
  ldmxcsr (%rsp) #78.1
  xorl %ecx, %ecx #69.8
..B1.2: # Preds ..B1.2 ..B1.46
  imull $1664525, %r8d, %r10d #70.21
  movl $1321528399, %eax #71.39
  lea 1013904223(%r10), %r8d #70.32
  mull %r8d #71.39
  shrl $3, %edx #71.39
  imull $-26, %edx, %r9d #71.39
  lea 1013904320(%r9,%r10), %r11d #71.39
  movb %r11b, haystack(%rcx) #71.5
  incl %ecx #69.33
  cmpq $524288, %rcx #69.19
  jb ..B1.2 # Prob 82% #69.19
..B1.3: # Preds ..B1.2
  xorl %r8d, %r8d #85.8
  movb $0, 524288+haystack(%rip) #73.3
  xorl %r9d, %r9d #85.15
..B1.4: # Preds ..B1.32 ..B1.3
  xorl %r10d, %r10d #86.10
..B1.5: # Preds ..B1.31 ..B1.4
  imull $1664525, %edi, %edi #91.23
  movl %r10d, %r11d #88.36
  andl $7, %r11d #88.36
  addl $1013904223, %edi #91.34
  lea 4(%r11), %ebx #88.36
  testl $1, %r10d #93.15
  je ..B1.9 # Prob 50% #93.15
..B1.6: # Preds ..B1.5
  imull $31337, %edi, %ecx #94.37
  movq $0x4001000400101, %r12 #94.47
  movq %rcx, %rax #94.47
  movq %rcx, %r13 #94.47
  mulq %r12 #94.47
  subq %rdx, %r13 #94.47
  shrq $1, %r13 #94.47
  addq %r13, %rdx #94.47
  shrq $18, %rdx #94.47
  imulq $-524256, %rdx, %r14 #94.47
  addq %r14, %rcx #94.47
..B1.7: # Preds ..B1.6
  jmp ..B1.35 # Prob 100% #95.9
..B1.9: # Preds ..B1.5
  xorl %r12d, %r12d #98.14
  xorl %ecx, %ecx #98.18
..B1.11: # Preds ..B1.9 ..B1.11
  movl %edi, %r13d #99.50
  movl $1321528399, %eax #99.56
  shrl %cl, %r13d #99.50
  addl $3, %ecx #98.31
  mull %r13d #99.56
  shrl $3, %edx #99.56
  imull $-26, %edx, %r14d #99.56
  lea 97(%r13,%r14), %r15d #99.56
  movb %r15b, (%rsp,%r12) #99.11
  incl %r12d #98.31
  cmpl %ebx, %r12d #98.25
  jb ..B1.11 # Prob 93% #98.25
..B1.13: # Preds ..B1.11 ..B1.39 ..B1.40
  movd %ebx, %xmm0 #47.26
  xorl %eax, %eax #46.3
  punpcklbw %xmm0, %xmm0 #47.26
  punpcklwd %xmm0, %xmm0 #47.26
  punpckldq %xmm0, %xmm0 #47.26
  punpcklqdq %xmm0, %xmm0 #47.26
..B1.14: # Preds ..B1.14 ..B1.13
  lea 16(%rax), %edx #46.3
  lea 32(%rax), %ecx #46.3
  lea 48(%rax), %r12d #46.3
  movdqu %xmm0, 16(%rsp,%rax) #47.5
  lea 64(%rax), %r13d #46.3
  movdqu %xmm0, 16(%rsp,%rdx) #47.5
  lea 80(%rax), %r14d #46.3
  movdqu %xmm0, 16(%rsp,%rcx) #47.5
  lea 96(%rax), %r15d #46.3
  movdqu %xmm0, 16(%rsp,%r12) #47.5
  lea 112(%rax), %edx #46.3
  addl $128, %eax #46.3
  movdqu %xmm0, 16(%rsp,%r13) #47.5
  movdqu %xmm0, 16(%rsp,%r14) #47.5
  movdqu %xmm0, 16(%rsp,%r15) #47.5
  movdqu %xmm0, 16(%rsp,%rdx) #47.5
  cmpl $256, %eax #46.3
  jb ..B1.14 # Prob 99% #46.3
..B1.16: # Preds ..B1.14
  xorl %edx, %edx #48.3
  lea 3(%r11), %eax #48.3
  movl %eax, %ecx #48.3
  shrl $1, %ecx #48.3
..B1.18: # Preds ..B1.16 ..B1.18
  movl %ebx, %r12d #49.48
  lea (%rdx,%rdx), %r14d #49.5
  subl %r14d, %r12d #49.48
  movzbl (%rsp,%r14), %r15d #102.62
  lea -1(%r12), %r13d #49.48
  movb %r13b, 16(%rsp,%r15) #49.5
  addl $-2, %r12d #49.48
  lea 1(%rdx,%rdx), %r13d #49.17
  movzbl (%rsp,%r13), %r14d #102.62
  incl %edx #48.3
  movb %r12b, 16(%rsp,%r14) #49.5
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
  addl %ebx, %ecx #49.48
  movzbl (%rsp,%rdx), %r12d #102.62
  movb %cl, 16(%rsp,%r12) #49.5
..B1.22: # Preds ..B1.20 ..B1.21
  negl %ebx #102.48
  xorl %r14d, %r14d #51.18
  movl %eax, %r13d #53.31
  addl $524288, %ebx #102.48
..B1.23: # Preds ..B1.41 ..B1.22
  movl %eax, %r12d #53.31
  lea 3(%r14,%r11), %r15d #53.31
  movq %r13, %rdx #53.31
..B1.25: # Preds ..B1.23 ..B1.26
  movb (%rsp,%rdx), %cl #102.62
  cmpb haystack(%r15), %cl #54.35
  jne ..B1.41 # Prob 20% #54.35
..B1.26: # Preds ..B1.25
  decq %rdx #55.7
  decl %r15d #55.7
  decl %r12d #55.7
  jns ..B1.25 # Prob 82% #54.17
..B1.28: # Preds ..B1.26
  testl %r14d, %r14d #103.18
  jl ..B1.30 # Prob 16% #103.18
..B1.29: # Preds ..B1.28
  movl %r10d, %eax #104.38
  movslq %r14d, %r14 #104.26
  shlq $24, %rax #104.43
  xorq %rax, %r14 #104.43
  addq %r14, %rsi #104.9
  jmp ..B1.31 # Prob 100% #104.9
..B1.30: # Preds ..B1.41 ..B1.28
  imulq $104729, %r10, %rax #106.30
  addq %rax, %rsi #106.9
..B1.31: # Preds ..B1.29 ..B1.30
  incl %r10d #86.34
  cmpl $2048, %r10d #86.21
  jb ..B1.5 # Prob 99% #86.21
..B1.32: # Preds ..B1.31
  movl $1321528399, %eax #108.72
  mull %r8d #108.72
  movl %r9d, %r10d #108.22
  addl $16381, %r9d #85.33
  shrl $3, %edx #108.72
  andq $524287, %r10 #108.32
  imull $-26, %edx, %ecx #108.72
  lea 97(%r8,%rcx), %ebx #108.72
  incl %r8d #85.33
  movb %bl, haystack(%r10) #108.5
  cmpl $8, %r8d #85.25
  jb ..B1.4 # Prob 87% #85.25
..B1.33: # Preds ..B1.32
  movl $.L_2__STRING.0, %edi #111.3
  xorl %eax, %eax #111.3
  call printf #111.3
..B1.34: # Preds ..B1.33
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
..B1.35: # Preds ..B1.7
  movl %ebx, %edx #88.25
  xorl %eax, %eax #95.9
  shrl $1, %edx #88.25
..B1.37: # Preds ..B1.35 ..B1.37
  lea (%rcx,%rax,2), %r13d #96.23
  movb haystack(%r13), %r14b #96.23
  lea (%rax,%rax), %r12d #96.23
  incl %r13d #96.23
  lea 1(%rax,%rax), %r15d #96.11
  movb %r14b, (%rsp,%r12) #96.11
  incl %eax #95.9
  movb haystack(%r13), %r12b #96.23
  movb %r12b, (%rsp,%r15) #96.11
  cmpl %edx, %eax #95.9
  jb ..B1.37 # Prob 87% #95.9
..B1.38: # Preds ..B1.37
  lea 1(%rax,%rax), %eax #96.11
..B1.39: # Preds ..B1.38
  lea -1(%rax), %edx #95.9
  cmpl %ebx, %edx #95.9
  jae ..B1.13 # Prob 10% #95.9
..B1.40: # Preds ..B1.39
  lea -1(%rax,%rcx), %eax #96.23
  movb haystack(%rax), %cl #96.23
  movb %cl, (%rsp,%rdx) #96.11
  jmp ..B1.13 # Prob 100% #96.11
..B1.41: # Preds ..B1.25
  lea 3(%r14,%r11), %edx #58.52
  movzbl haystack(%rdx), %ecx #102.38
  movzbl 16(%rsp,%rcx), %r12d #58.10
  addl %r12d, %r14d #58.5
  cmpl %ebx, %r14d #52.24
  jbe ..B1.23 # Prob 82% #52.24
  jmp ..B1.30 # Prob 100% #52.24
haystack:
.L_2__STRING.0:
