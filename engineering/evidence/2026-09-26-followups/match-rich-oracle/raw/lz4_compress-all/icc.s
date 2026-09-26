main:
..B1.1: # Preds ..B1.0
  pushq %rbp #152.1
  movq %rsp, %rbp #152.1
  andq $-128, %rsp #152.1
  subq $128, %rsp #152.1
  movl $3, %edi #152.1
  xorl %esi, %esi #152.1
  call __intel_new_feature_proc_init #152.1
..B1.95: # Preds ..B1.1
  stmxcsr (%rsp) #152.1
  xorl %esi, %esi #154.16
  xorl %eax, %eax #136.8
  orl $32832, (%rsp) #152.1
  movl $-1843407944, %edx #137.5
  ldmxcsr (%rsp) #152.1
..B1.2: # Preds ..B1.7 ..B1.4 ..B1.95
  movl %edx, %ecx #146.35
  shrl $24, %ecx #146.35
  movb %cl, src_data(%rax) #146.7
..B1.3: # Preds ..B1.2 ..B1.8 ..B1.6
  incl %eax #136.29
  cmpl $524288, %eax #136.19
  jae ..B1.11 # Prob 18% #136.19
..B1.4: # Preds ..B1.3
  imull $1664525, %edx, %edx #137.21
  addl $1013904223, %edx #137.32
  cmpl $128, %eax #139.14
  jb ..B1.2 # Prob 50% #139.14
..B1.5: # Preds ..B1.4
  movl %eax, %eax #139.34
  cmpb $96, %al #139.34
  jae ..B1.7 # Prob 50% #139.34
..B1.6: # Preds ..B1.5
  lea -128(%rax), %ecx #140.34
  movl %eax, %eax #140.7
  movb src_data(%rcx), %dil #140.21
  movb %dil, src_data(%rax) #140.7
  jmp ..B1.3 # Prob 100% #140.7
..B1.7: # Preds ..B1.5
  movl %edx, %ecx #143.18
  andl $15, %ecx #143.18
  cmpl $6, %ecx #143.27
  jae ..B1.2 # Prob 50% #143.27
..B1.8: # Preds ..B1.7
  movl %edx, %ecx #144.49
  andl $63, %ecx #144.49
  lea -128(%rax,%rcx), %edi #136.29
  movl %eax, %eax #144.7
  movb src_data(%rdi), %r8b #144.21
  movb %r8b, src_data(%rax) #144.7
  jmp ..B1.3 # Prob 100% #144.7
..B1.11: # Preds ..B1.3
  xorl %ecx, %ecx #158.8
  movl $src_data+524288, %r11d #56.26
  movl $src_data+524276, %r10d #57.36
  movq %r12, (%rsp) #62.3[spill]
  movl $src_data+1, %eax #68.3
  movq %r13, 8(%rsp) #62.3[spill]
  movl $src_data, %r9d #158.15
  movq %r14, 16(%rsp) #62.3[spill]
  movq %r15, 24(%rsp) #62.3[spill]
  movq %rbx, 32(%rsp) #62.3[spill]
  movl %ecx, %ebx #62.3
  pcmpeqd %xmm0, %xmm0 #92.36
  movl %ecx, %r15d #62.3
  movq %rsi, %r14 #62.3
..B1.12: # Preds ..B1.61 ..B1.11
  movl $src_data, %ecx #58.22
  movl $hash_table, %edi #62.3
  xorl %esi, %esi #62.3
  movl $65536, %edx #62.3
  movq %rcx, 104(%rsp) #62.3[spill]
  movl $dst_data, %r12d #59.12
  call _intel_fast_memset #62.3
..B1.13: # Preds ..B1.12
  movl $src_data+1, %eax #
  movl $src_data+524276, %r8d #69.15
  movq 104(%rsp), %rcx #[spill]
  movq %rax, %r13 #68.3
  cmpq %r8, %rax #69.15
  jae ..B1.51 # Prob 10% #69.15
..B1.14: # Preds ..B1.13
  movl %ebx, 80(%rsp) #[spill]
  movq %r12, %rdi #
  movl %r15d, 88(%rsp) #[spill]
  movq %r14, 96(%rsp) #[spill]
..B1.15: # Preds ..B1.49 ..B1.14
  movl (%r13), %eax #42.3
..B1.16: # Preds ..B1.15
  imull $-1640531535, %eax, %eax #49.17
  movq %r13, %r8 #72.27
  shrl $18, %eax #49.33
  movl $src_data, %edx #72.27
  movl hash_table(,%rax,4), %ebx #71.27
  subq %rdx, %r8 #72.27
  movl %r8d, hash_table(,%rax,4) #72.5
  lea src_data(%rbx), %rbx #71.21
  cmpq %r13, %rbx #74.15
  jae ..B1.48 # Prob 50% #74.15
..B1.17: # Preds ..B1.16
  movl $src_data, %eax #74.28
  cmpq %rax, %rbx #74.28
  jb ..B1.48 # Prob 78% #74.28
..B1.18: # Preds ..B1.17
  movl (%rbx), %eax #42.3
..B1.19: # Preds ..B1.18
  movl (%r13), %edx #42.3
..B1.20: # Preds ..B1.19
  cmpl %edx, %eax #74.50
  jne ..B1.48 # Prob 50% #74.50
..B1.21: # Preds ..B1.20
  movq %r13, %r9 #76.45
  addq $4, %r13 #78.7
  movl $src_data+524288, %eax #80.19
  subq %rcx, %r9 #76.45
  movl %r9d, %r12d #76.50
  lea 4(%rbx), %r15 #77.25
  cmpq %rax, %r13 #80.19
  jae ..B1.26 # Prob 10% #80.19
..B1.22: # Preds ..B1.21
  movl $src_data+524288, %edx #
..B1.23: # Preds ..B1.24 ..B1.22
  movb (%r13), %al #80.28
  cmpb (%r15), %al #80.35
  jne ..B1.26 # Prob 20% #80.35
..B1.24: # Preds ..B1.23
  incq %r13 #81.9
  incq %r15 #82.9
  cmpq %rdx, %r13 #80.19
  jb ..B1.23 # Prob 82% #80.19
..B1.26: # Preds ..B1.23 ..B1.24 ..B1.21
  negq %rbx #85.47
  lea 1(%rdi), %r14 #86.19
  addq %r15, %rbx #85.47
  movq %r14, %r8 #86.19
  addq $-4, %rbx #85.61
  movq %rdi, 72(%rsp) #86.19[spill]
  cmpl $15, %r12d #89.22
  jb ..B1.33 # Prob 50% #89.22
..B1.27: # Preds ..B1.26
  movb $240, (%rdi) #90.10
  lea -15(%r12), %r11d #91.36
  cmpl $255, %r11d #92.21
  jb ..B1.31 # Prob 10% #92.21
..B1.28: # Preds ..B1.27
  movq $0x8080808080808081, %rax #92.9
  movq %r11, %rsi #92.9
  negq %rsi #92.9
  imulq %rsi #92.9
  movq %rdx, %r10 #92.9
  subq %r11, %r10 #92.9
  shrq $7, %r10 #92.9
  sarq $63, %rsi #92.9
  negq %r10 #92.9
  addq %rsi, %r10 #92.9
  movl %r10d, 40(%rsp) #92.9[spill]
  cmpl $96, %r10d #92.9
  jle ..B1.68 # Prob 0% #92.9
..B1.29: # Preds ..B1.28
  movslq %r10d, %rdx #92.9
  movq %r14, %rdi #92.9
  movl $255, %esi #92.9
  movq %rdx, 48(%rsp) #92.9[spill]
  movq %r8, 56(%rsp) #92.9[spill]
  movq %r9, 64(%rsp) #92.9[spill]
  movq %rcx, 104(%rsp) #92.9[spill]
  call _intel_fast_memset #92.9
..B1.97: # Preds ..B1.29
  movq 104(%rsp), %rcx #[spill]
  movq 64(%rsp), %r9 #[spill]
  movq 56(%rsp), %r8 #[spill]
  movq 48(%rsp), %rdx #[spill]
..B1.30: # Preds ..B1.75 ..B1.97
  movl 40(%rsp), %edi #92.29[spill]
  movl %edi, %eax #92.29
  shll $8, %eax #92.29
  lea (%r8,%rdx), %r14 #92.29
  subl %eax, %edi #92.29
  lea -15(%r12,%rdi), %r11d #92.29
..B1.31: # Preds ..B1.30 ..B1.27
  movb %r11b, (%r14) #93.10
  incq %r14 #93.10
  cmpl $96, %r12d #97.7
  jbe ..B1.64 # Prob 0% #97.7
..B1.32: # Preds ..B1.31
  movl %r9d, %edx #97.7
  movq %r14, %rdi #97.7
  movq %rcx, %rsi #97.7
  call _intel_fast_memcpy #97.7
  jmp ..B1.39 # Prob 100% #97.7
..B1.33: # Preds ..B1.26
  movl %r12d, %eax #95.34
  shll $4, %eax #95.34
  movb %al, (%rdi) #95.10
  testl %r12d, %r12d #97.23
  jbe ..B1.40 # Prob 50% #97.23
..B1.34: # Preds ..B1.33 ..B1.64
  xorl %edx, %edx #97.7
..B1.35: # Preds ..B1.66 ..B1.34
  movslq %edx, %rax #97.7
  cmpl %r12d, %edx #97.7
  jae ..B1.39 # Prob 10% #97.7
..B1.37: # Preds ..B1.35 ..B1.37
  movb (%rdx,%rcx), %r8b #98.17
  incl %edx #97.7
  movb %r8b, (%rax,%r14) #98.10
  incq %rax #97.7
  cmpl %r12d, %edx #97.7
  jb ..B1.37 # Prob 82% #97.7
..B1.39: # Preds ..B1.37 ..B1.35 ..B1.32
  decl %r12d #98.10
  movslq %r12d, %r12 #98.10
  lea 1(%r14,%r12), %r14 #98.10
..B1.40: # Preds ..B1.39 ..B1.33
  negq %r15 #101.26
  lea 2(%r14), %rdi #103.8
  addq %r13, %r15 #101.26
  movb %r15b, (%r14) #102.8
  shrl $8, %r15d #103.30
  movb %r15b, 1(%r14) #103.8
  cmpl $15, %ebx #106.24
  jb ..B1.46 # Prob 50% #106.24
..B1.41: # Preds ..B1.40
  movq 72(%rsp), %rax #107.10[spill]
  lea -15(%rbx), %esi #108.38
  orb $15, (%rax) #107.10
  cmpl $255, %esi #109.21
  jb ..B1.45 # Prob 10% #109.21
..B1.42: # Preds ..B1.41
  movq $0x8080808080808081, %rax #109.9
  movq %rsi, %r8 #109.9
  negq %r8 #109.9
  imulq %r8 #109.9
  movq %rdx, %rcx #109.9
  subq %rsi, %rcx #109.9
  shrq $7, %rcx #109.9
  sarq $63, %r8 #109.9
  negq %rcx #109.9
  addq %r8, %rcx #109.9
  movl %ecx, %r12d #109.9
  cmpl $96, %r12d #109.9
  jle ..B1.78 # Prob 0% #109.9
..B1.43: # Preds ..B1.42
  movslq %ecx, %r15 #109.9
  movl $255, %esi #109.9
  movq %r15, %rdx #109.9
  call _intel_fast_memset #109.9
..B1.44: # Preds ..B1.85 ..B1.43
  movl %r12d, %eax #109.29
  lea 2(%r14,%r15), %rdi #109.29
  shll $8, %eax #109.29
  subl %eax, %r12d #109.29
  lea -15(%rbx,%r12), %esi #109.29
..B1.45: # Preds ..B1.44 ..B1.41
  movb %sil, (%rdi) #110.10
  incq %rdi #110.10
  jmp ..B1.47 # Prob 100% #110.10
..B1.46: # Preds ..B1.40
  movq 72(%rsp), %rax #112.10[spill]
  orb %bl, (%rax) #112.10
..B1.47: # Preds ..B1.45 ..B1.46
  movq %r13, %rcx #115.7
  jmp ..B1.49 # Prob 100% #115.7
..B1.48: # Preds ..B1.16 ..B1.17 ..B1.20
  movq %r13, %rax #117.19
  subq %rcx, %rax #117.19
  sarq $6, %rax #117.35
  lea 1(%r13,%rax), %r13 #117.7
..B1.49: # Preds ..B1.48 ..B1.47
  movl $src_data+524276, %eax #69.15
  cmpq %rax, %r13 #69.15
  jb ..B1.15 # Prob 82% #69.15
..B1.50: # Preds ..B1.49
  movl 80(%rsp), %ebx #[spill]
  movq %rdi, %r12 #
  movl 88(%rsp), %r15d #[spill]
  movl $src_data+1, %eax #
  movq 96(%rsp), %r14 #[spill]
..B1.51: # Preds ..B1.50 ..B1.13
  movl $src_data+524288, %r8d #122.43
  subq %rcx, %r8 #122.43
  movl %r8d, %r13d #122.50
  cmpl $15, %r13d #123.28
  jae ..B1.58 # Prob 50% #123.28
..B1.52: # Preds ..B1.51
  movl %r13d, %r8d #123.47
  shll $4, %r8d #123.47
  movb %r8b, (%r12) #123.4
  incq %r12 #123.4
  testl %r13d, %r13d #124.19
  jbe ..B1.61 # Prob 50% #124.19
..B1.53: # Preds ..B1.52 ..B1.89
  xorl %r9d, %r9d #124.3
..B1.54: # Preds ..B1.91 ..B1.53
  movslq %r9d, %r8 #124.3
  cmpl %r13d, %r9d #124.3
  jae ..B1.60 # Prob 10% #124.3
..B1.56: # Preds ..B1.54 ..B1.56
  movb (%r9,%rcx), %r10b #125.13
  incl %r9d #124.3
  movb %r10b, (%r8,%r12) #125.6
  incq %r8 #124.3
  cmpl %r13d, %r9d #124.3
  jb ..B1.56 # Prob 82% #124.3
  jmp ..B1.60 # Prob 100% #124.3
..B1.58: # Preds ..B1.51
  movb $240, (%r12) #123.4
  incq %r12 #123.4
  cmpl $96, %r13d #124.3
  jbe ..B1.89 # Prob 0% #124.3
..B1.59: # Preds ..B1.58
  movl %r8d, %edx #124.3
  movq %r12, %rdi #124.3
  movq %rcx, %rsi #124.3
  call _intel_fast_memcpy #124.3
..B1.96: # Preds ..B1.59
  movl $src_data+1, %eax #
..B1.60: # Preds ..B1.56 ..B1.54 ..B1.96
  decl %r13d #125.6
  movslq %r13d, %r13 #125.6
  lea 1(%r12,%r13), %r12 #125.6
..B1.61: # Preds ..B1.60 ..B1.52
  movl $dst_data, %r8d #127.25
  incl %r15d #158.33
  subq %r8, %r12 #127.25
  movl %ebx, %r11d #161.22
  addl $4099, %ebx #158.33
  movl %r12d, %r10d #160.23
  andq $524287, %r11 #161.31
  shrl $1, %r12d #160.79
  shlq $32, %r10 #160.34
  movzbl dst_data(%rip), %r9d #160.40
  xorq %r9, %r10 #160.40
  movzbl dst_data(%r12), %edi #160.60
  shlq $16, %rdi #160.85
  xorq %rdi, %r10 #160.85
  xorb $85, src_data(%r11) #161.5
  addq %r10, %r14 #160.5
  cmpl $2048, %r15d #158.25
  jb ..B1.12 # Prob 99% #158.25
..B1.62: # Preds ..B1.61
  movq %r14, %rsi #
  movl $.L_2__STRING.0, %edi #164.3
  xorl %eax, %eax #164.3
  movq (%rsp), %r12 #[spill]
  movq 8(%rsp), %r13 #[spill]
  movq 16(%rsp), %r14 #[spill]
  movq 24(%rsp), %r15 #[spill]
  movq 32(%rsp), %rbx #[spill]
  call printf #164.3
..B1.63: # Preds ..B1.62
  xorl %eax, %eax #165.10
  movq %rbp, %rsp #165.10
  popq %rbp #165.10
  ret #165.10
..B1.64: # Preds ..B1.31
  cmpl $16, %r12d #97.7
  jb ..B1.34 # Prob 10% #97.7
..B1.65: # Preds ..B1.64
  movl %r12d, %edx #97.7
  xorl %r8d, %r8d #97.7
  andl $-16, %edx #97.7
  xorl %eax, %eax #98.10
..B1.66: # Preds ..B1.66 ..B1.65
  movdqu (%r8,%rcx), %xmm0 #98.17
  addl $16, %r8d #97.7
  movdqu %xmm0, (%rax,%r14) #98.10
  addq $16, %rax #97.7
  cmpl %edx, %r8d #97.7
  jb ..B1.66 # Prob 82% #97.7
  jmp ..B1.35 # Prob 100% #97.7
..B1.68: # Preds ..B1.28
  cmpl $16, 40(%rsp) #92.9[spill]
  jl ..B1.77 # Prob 10% #92.9
..B1.69: # Preds ..B1.68
  movl %r10d, %edx #92.9
  xorl %eax, %eax #92.9
  andl $-16, %edx #92.9
  pcmpeqd %xmm0, %xmm0 #92.9
..B1.70: # Preds ..B1.70 ..B1.69
  addl $16, %eax #92.9
  movdqu %xmm0, (%r14) #92.29
  addq $16, %r14 #92.29
  cmpl %edx, %eax #92.9
  jb ..B1.70 # Prob 82% #92.9
..B1.72: # Preds ..B1.70 ..B1.77
  cmpl 40(%rsp), %edx #92.9[spill]
  jae ..B1.75 # Prob 0% #92.9
..B1.73: # Preds ..B1.72
  movl 40(%rsp), %eax #[spill]
..B1.74: # Preds ..B1.74 ..B1.73
  incl %edx #92.9
  movb $255, (%r14) #92.29
  incq %r14 #92.29
  cmpl %eax, %edx #92.9
  jb ..B1.74 # Prob 82% #92.9
..B1.75: # Preds ..B1.72 ..B1.74
  movslq %r10d, %rdx #92.41
  jmp ..B1.30 # Prob 100% #92.41
..B1.77: # Preds ..B1.68
  xorl %edx, %edx #92.9
  jmp ..B1.72 # Prob 100% #92.9
..B1.78: # Preds ..B1.42
  cmpl $16, %r12d #109.9
  jl ..B1.87 # Prob 10% #109.9
..B1.79: # Preds ..B1.78
  movl %r12d, %edx #109.9
  xorl %eax, %eax #109.9
  andl $-16, %edx #109.9
  pcmpeqd %xmm0, %xmm0 #109.9
..B1.80: # Preds ..B1.80 ..B1.79
  addl $16, %eax #109.9
  movdqu %xmm0, (%rdi) #109.29
  addq $16, %rdi #109.29
  cmpl %edx, %eax #109.9
  jb ..B1.80 # Prob 82% #109.9
..B1.82: # Preds ..B1.80 ..B1.87
  cmpl %r12d, %edx #109.9
  jae ..B1.85 # Prob 0% #109.9
..B1.84: # Preds ..B1.82 ..B1.84
  incl %edx #109.9
  movb $255, (%rdi) #109.29
  incq %rdi #109.29
  cmpl %r12d, %edx #109.9
  jb ..B1.84 # Prob 82% #109.9
..B1.85: # Preds ..B1.82 ..B1.84
  movslq %ecx, %r15 #109.41
  jmp ..B1.44 # Prob 100% #109.41
..B1.87: # Preds ..B1.78
  xorl %edx, %edx #109.9
  jmp ..B1.82 # Prob 100% #109.9
..B1.89: # Preds ..B1.58
  cmpl $16, %r13d #124.3
  jb ..B1.53 # Prob 10% #124.3
..B1.90: # Preds ..B1.89
  movl %r13d, %r9d #124.3
  xorl %r10d, %r10d #124.3
  andl $-16, %r9d #124.3
  xorl %r8d, %r8d #125.6
..B1.91: # Preds ..B1.91 ..B1.90
  movdqu (%r10,%rcx), %xmm0 #125.13
  addl $16, %r10d #124.3
  movdqu %xmm0, (%r8,%r12) #125.6
  addq $16, %r8 #124.3
  cmpl %r9d, %r10d #124.3
  jb ..B1.91 # Prob 82% #124.3
  jmp ..B1.54 # Prob 100% #124.3
src_data:
hash_table:
dst_data:
.L_2__STRING.0:
