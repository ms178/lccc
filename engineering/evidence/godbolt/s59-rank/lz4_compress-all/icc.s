main:
..B1.1: # Preds ..B1.0
  pushq %rbp #141.1
  movq %rsp, %rbp #141.1
  andq $-128, %rsp #141.1
  subq $128, %rsp #141.1
  movl $3, %edi #141.1
  xorl %esi, %esi #141.1
  call __intel_new_feature_proc_init #141.1
..B1.93: # Preds ..B1.1
  stmxcsr (%rsp) #141.1
  xorl %esi, %esi #143.16
  movl $-1843407944, %edx #131.5
  orl $32832, (%rsp) #141.1
  xorl %eax, %eax #130.8
  ldmxcsr (%rsp) #141.1
..B1.2: # Preds ..B1.5 ..B1.4 ..B1.93
  movl %edx, %ecx #135.35
  shrl $24, %ecx #135.35
  movb %cl, src_data(%rax) #135.7
  movl %eax, %ecx #130.29
  jmp ..B1.3 # Prob 100% #130.29
..B1.6: # Preds ..B1.5
  movl %edx, %edi #133.49
  andl $63, %edi #133.49
  lea -128(%rcx,%rdi), %r8d #133.49
  movb src_data(%r8), %r9b #133.21
  movb %r9b, src_data(%rax) #133.7
..B1.3: # Preds ..B1.2 ..B1.6
  incl %ecx #130.29
  movl %ecx, %eax #130.29
  cmpq $524288, %rax #130.19
  jae ..B1.9 # Prob 18% #130.19
..B1.4: # Preds ..B1.3
  imull $1664525, %edx, %edx #131.21
  addl $1013904223, %edx #131.32
  movl %edx, %edi #132.18
  andl $15, %edi #132.18
  cmpl $6, %edi #132.27
  jae ..B1.2 # Prob 50% #132.27
..B1.5: # Preds ..B1.4
  cmpl $128, %ecx #132.37
  jae ..B1.6 # Prob 50% #132.37
  jmp ..B1.2 # Prob 100% #132.37
..B1.9: # Preds ..B1.3
  xorb %dl, %dl #147.8
  xorl %eax, %eax #147.15
  movq %r12, (%rsp) #56.3[spill]
  movl $src_data+524288, %r11d #50.26
  movq %r13, 8(%rsp) #56.3[spill]
  movl $src_data+524276, %r10d #51.36
  movq %r14, 16(%rsp) #56.3[spill]
  movq %r15, 24(%rsp) #56.3[spill]
  movq %rbx, 32(%rsp) #56.3[spill]
  pcmpeqd %xmm0, %xmm0 #86.36
  movl %eax, %ebx #56.3
  movb %dl, %r12b #56.3
  movq %rsi, %r15 #56.3
  movl $src_data+1, %eax #56.3
..B1.10: # Preds ..B1.59 ..B1.9
  movl $src_data, %r8d #52.22
  movl $hash_table, %edi #56.3
  xorl %esi, %esi #56.3
  movl $65536, %edx #56.3
  movq %r8, 104(%rsp) #56.3[spill]
  movl $dst_data, %r13d #53.12
  call _intel_fast_memset #56.3
..B1.11: # Preds ..B1.10
  movl $src_data+1, %eax #
  movl $src_data+524276, %ecx #63.15
  movq 104(%rsp), %r8 #[spill]
  movq %rax, %rdx #62.3
  cmpq %rcx, %rax #63.15
  jae ..B1.49 # Prob 10% #63.15
..B1.12: # Preds ..B1.11
  movl %ebx, 80(%rsp) #[spill]
  movq %r13, %rdi #
  movb %r12b, 88(%rsp) #[spill]
  movq %r15, 96(%rsp) #[spill]
  movq %rdx, %r15 #
..B1.13: # Preds ..B1.47 ..B1.12
  movl (%r15), %eax #36.3
..B1.14: # Preds ..B1.13
  imull $-1640531535, %eax, %eax #43.17
  movq %r15, %rbx #66.27
  shrl $18, %eax #43.33
  movl $src_data, %ecx #66.27
  movl hash_table(,%rax,4), %r12d #65.27
  subq %rcx, %rbx #66.27
  movl %ebx, hash_table(,%rax,4) #66.5
  lea src_data(%r12), %r12 #65.21
  cmpq %r15, %r12 #68.15
  jae ..B1.46 # Prob 50% #68.15
..B1.15: # Preds ..B1.14
  movl $src_data, %eax #68.28
  cmpq %rax, %r12 #68.28
  jb ..B1.46 # Prob 78% #68.28
..B1.16: # Preds ..B1.15
  movl (%r12), %eax #36.3
..B1.17: # Preds ..B1.16
  movl (%r15), %ecx #36.3
..B1.18: # Preds ..B1.17
  cmpl %ecx, %eax #68.50
  jne ..B1.46 # Prob 50% #68.50
..B1.19: # Preds ..B1.18
  movq %r15, %rcx #70.45
  addq $4, %r15 #72.7
  movl $src_data+524288, %eax #74.19
  subq %r8, %rcx #70.45
  movl %ecx, %r13d #70.50
  lea 4(%r12), %r14 #71.25
  cmpq %rax, %r15 #74.19
  jae ..B1.24 # Prob 10% #74.19
..B1.20: # Preds ..B1.19
  movl $src_data+524288, %ebx #
..B1.21: # Preds ..B1.22 ..B1.20
  movb (%r15), %al #74.28
  cmpb (%r14), %al #74.35
  jne ..B1.24 # Prob 20% #74.35
..B1.22: # Preds ..B1.21
  incq %r15 #75.9
  incq %r14 #76.9
  cmpq %rbx, %r15 #74.19
  jb ..B1.21 # Prob 82% #74.19
..B1.24: # Preds ..B1.21 ..B1.22 ..B1.19
  negq %r12 #79.47
  lea 1(%rdi), %rbx #80.19
  addq %r14, %r12 #79.47
  movq %rbx, %r9 #80.19
  addq $-4, %r12 #79.61
  movq %rdi, 72(%rsp) #80.19[spill]
  cmpl $15, %r13d #83.22
  jb ..B1.31 # Prob 50% #83.22
..B1.25: # Preds ..B1.24
  movb $240, (%rdi) #84.10
  lea -15(%r13), %r10d #85.36
  cmpl $255, %r10d #86.21
  jb ..B1.29 # Prob 10% #86.21
..B1.26: # Preds ..B1.25
  movq $0x8080808080808081, %rax #86.9
  movq %r10, %r11 #86.9
  negq %r11 #86.9
  imulq %r11 #86.9
  subq %r10, %rdx #86.9
  shrq $7, %rdx #86.9
  sarq $63, %r11 #86.9
  negq %rdx #86.9
  addq %r11, %rdx #86.9
  movl %edx, 40(%rsp) #86.9[spill]
  cmpl $96, %edx #86.9
  jle ..B1.66 # Prob 0% #86.9
..B1.27: # Preds ..B1.26
  movslq %edx, %rdx #86.9
  movq %rbx, %rdi #86.9
  movl $255, %esi #86.9
  movq %rdx, 48(%rsp) #86.9[spill]
  movq %r9, 56(%rsp) #86.9[spill]
  movq %rcx, 64(%rsp) #86.9[spill]
  movq %r8, 104(%rsp) #86.9[spill]
  call _intel_fast_memset #86.9
..B1.95: # Preds ..B1.27
  movq 104(%rsp), %r8 #[spill]
  movq 64(%rsp), %rcx #[spill]
  movq 56(%rsp), %r9 #[spill]
  movq 48(%rsp), %rdx #[spill]
..B1.28: # Preds ..B1.73 ..B1.95
  movl 40(%rsp), %ebx #86.29[spill]
  movl %ebx, %eax #86.29
  shll $8, %eax #86.29
  subl %eax, %ebx #86.29
  lea -15(%r13,%rbx), %r10d #86.29
  lea (%r9,%rdx), %rbx #86.29
..B1.29: # Preds ..B1.28 ..B1.25
  movb %r10b, (%rbx) #87.10
  incq %rbx #87.10
  cmpl $96, %r13d #91.7
  jbe ..B1.62 # Prob 0% #91.7
..B1.30: # Preds ..B1.29
  movl %ecx, %edx #91.7
  movq %rbx, %rdi #91.7
  movq %r8, %rsi #91.7
  call _intel_fast_memcpy #91.7
  jmp ..B1.37 # Prob 100% #91.7
..B1.31: # Preds ..B1.24
  movl %r13d, %eax #89.34
  shll $4, %eax #89.34
  movb %al, (%rdi) #89.10
  testl %r13d, %r13d #91.23
  jbe ..B1.38 # Prob 50% #91.23
..B1.32: # Preds ..B1.31 ..B1.62
  xorl %edx, %edx #91.7
..B1.33: # Preds ..B1.64 ..B1.32
  movslq %edx, %rax #91.7
  cmpl %r13d, %edx #91.7
  jae ..B1.37 # Prob 9% #91.7
..B1.35: # Preds ..B1.33 ..B1.35
  movb (%rdx,%r8), %cl #92.17
  incl %edx #91.7
  movb %cl, (%rax,%rbx) #92.10
  incq %rax #91.7
  cmpl %r13d, %edx #91.7
  jb ..B1.35 # Prob 82% #91.7
..B1.37: # Preds ..B1.35 ..B1.33 ..B1.30
  decl %r13d #92.10
  movslq %r13d, %r13 #92.10
  lea 1(%rbx,%r13), %rbx #92.10
..B1.38: # Preds ..B1.37 ..B1.31
  negq %r14 #95.26
  lea 2(%rbx), %rdi #97.8
  addq %r15, %r14 #95.26
  movb %r14b, (%rbx) #96.8
  shrl $8, %r14d #97.30
  movb %r14b, 1(%rbx) #97.8
  cmpl $15, %r12d #100.24
  jb ..B1.44 # Prob 50% #100.24
..B1.39: # Preds ..B1.38
  movq 72(%rsp), %rax #101.10[spill]
  lea -15(%r12), %esi #102.38
  orb $15, (%rax) #101.10
  cmpl $255, %esi #103.21
  jb ..B1.43 # Prob 10% #103.21
..B1.40: # Preds ..B1.39
  movq $0x8080808080808081, %rax #103.9
  movq %rsi, %r8 #103.9
  negq %r8 #103.9
  imulq %r8 #103.9
  movq %rdx, %rcx #103.9
  subq %rsi, %rcx #103.9
  shrq $7, %rcx #103.9
  sarq $63, %r8 #103.9
  negq %rcx #103.9
  addq %r8, %rcx #103.9
  movl %ecx, %r14d #103.9
  cmpl $96, %r14d #103.9
  jle ..B1.76 # Prob 0% #103.9
..B1.41: # Preds ..B1.40
  movslq %ecx, %r13 #103.9
  movl $255, %esi #103.9
  movq %r13, %rdx #103.9
  call _intel_fast_memset #103.9
..B1.42: # Preds ..B1.83 ..B1.41
  movl %r14d, %eax #103.29
  lea 2(%rbx,%r13), %rdi #103.29
  shll $8, %eax #103.29
  subl %eax, %r14d #103.29
  lea -15(%r12,%r14), %esi #103.29
..B1.43: # Preds ..B1.42 ..B1.39
  movb %sil, (%rdi) #104.10
  incq %rdi #104.10
  jmp ..B1.45 # Prob 100% #104.10
..B1.44: # Preds ..B1.38
  movq 72(%rsp), %rax #106.10[spill]
  orb %r12b, (%rax) #106.10
..B1.45: # Preds ..B1.43 ..B1.44
  movq %r15, %r8 #109.7
  jmp ..B1.47 # Prob 100% #109.7
..B1.46: # Preds ..B1.14 ..B1.15 ..B1.18
  movq %r15, %rax #111.19
  subq %r8, %rax #111.19
  sarq $6, %rax #111.35
  lea 1(%r15,%rax), %r15 #111.7
..B1.47: # Preds ..B1.46 ..B1.45
  movl $src_data+524276, %eax #63.15
  cmpq %rax, %r15 #63.15
  jb ..B1.13 # Prob 82% #63.15
..B1.48: # Preds ..B1.47
  movl 80(%rsp), %ebx #[spill]
  movq %rdi, %r13 #
  movb 88(%rsp), %r12b #[spill]
  movl $src_data+1, %eax #
  movq 96(%rsp), %r15 #[spill]
..B1.49: # Preds ..B1.48 ..B1.11
  movl $src_data+524288, %ecx #116.43
  subq %r8, %rcx #116.43
  movl %ecx, %r14d #116.50
  cmpl $15, %r14d #117.28
  jae ..B1.56 # Prob 50% #117.28
..B1.50: # Preds ..B1.49
  movl %r14d, %ecx #117.47
  shll $4, %ecx #117.47
  movb %cl, (%r13) #117.4
  incq %r13 #117.4
  testl %r14d, %r14d #118.19
  jbe ..B1.59 # Prob 50% #118.19
..B1.51: # Preds ..B1.50 ..B1.87
  xorl %r9d, %r9d #118.3
..B1.52: # Preds ..B1.89 ..B1.51
  movslq %r9d, %rcx #118.3
  cmpl %r14d, %r9d #118.3
  jae ..B1.58 # Prob 10% #118.3
..B1.54: # Preds ..B1.52 ..B1.54
  movb (%r9,%r8), %r10b #119.13
  incl %r9d #118.3
  movb %r10b, (%rcx,%r13) #119.6
  incq %rcx #118.3
  cmpl %r14d, %r9d #118.3
  jb ..B1.54 # Prob 82% #118.3
  jmp ..B1.58 # Prob 100% #118.3
..B1.56: # Preds ..B1.49
  movb $240, (%r13) #117.4
  incq %r13 #117.4
  cmpl $96, %r14d #118.3
  jbe ..B1.87 # Prob 0% #118.3
..B1.57: # Preds ..B1.56
  movl %ecx, %edx #118.3
  movq %r13, %rdi #118.3
  movq %r8, %rsi #118.3
  call _intel_fast_memcpy #118.3
..B1.94: # Preds ..B1.57
  movl $src_data+1, %eax #
..B1.58: # Preds ..B1.54 ..B1.52 ..B1.94
  decl %r14d #119.6
  movslq %r14d, %r14 #119.6
  lea 1(%r13,%r14), %r13 #119.6
..B1.59: # Preds ..B1.58 ..B1.50
  movl $dst_data, %ecx #121.25
  incb %r12b #147.33
  subq %rcx, %r13 #121.25
  movl %ebx, %r10d #150.22
  addl $4099, %ebx #147.33
  movl %r13d, %r9d #149.23
  andq $524287, %r10 #150.31
  shrl $1, %r13d #149.79
  shlq $32, %r9 #149.34
  movzbl dst_data(%rip), %r8d #149.40
  xorq %r8, %r9 #149.40
  movzbl dst_data(%r13), %edi #149.60
  shlq $16, %rdi #149.85
  xorq %rdi, %r9 #149.85
  xorb $85, src_data(%r10) #150.5
  addq %r9, %r15 #149.5
  cmpb $24, %r12b #147.25
  jb ..B1.10 # Prob 95% #147.25
..B1.60: # Preds ..B1.59
  movq %r15, %rsi #
  movl $.L_2__STRING.0, %edi #153.3
  xorl %eax, %eax #153.3
  movq (%rsp), %r12 #[spill]
  movq 8(%rsp), %r13 #[spill]
  movq 16(%rsp), %r14 #[spill]
  movq 24(%rsp), %r15 #[spill]
  movq 32(%rsp), %rbx #[spill]
  call printf #153.3
..B1.61: # Preds ..B1.60
  xorl %eax, %eax #154.10
  movq %rbp, %rsp #154.10
  popq %rbp #154.10
  ret #154.10
..B1.62: # Preds ..B1.29
  cmpl $16, %r13d #91.7
  jb ..B1.32 # Prob 10% #91.7
..B1.63: # Preds ..B1.62
  movl %r13d, %edx #91.7
  xorl %ecx, %ecx #91.7
  andl $-16, %edx #91.7
  xorl %eax, %eax #92.10
..B1.64: # Preds ..B1.64 ..B1.63
  movdqu (%rcx,%r8), %xmm0 #92.17
  addl $16, %ecx #91.7
  movdqu %xmm0, (%rax,%rbx) #92.10
  addq $16, %rax #91.7
  cmpl %edx, %ecx #91.7
  jb ..B1.64 # Prob 82% #91.7
  jmp ..B1.33 # Prob 100% #91.7
..B1.66: # Preds ..B1.26
  cmpl $16, 40(%rsp) #86.9[spill]
  jl ..B1.75 # Prob 10% #86.9
..B1.67: # Preds ..B1.66
  xorl %r10d, %r10d #86.9
  movl %edx, %eax #86.9
  pcmpeqd %xmm0, %xmm0 #86.9
  andl $-16, %eax #86.9
..B1.68: # Preds ..B1.68 ..B1.67
  addl $16, %r10d #86.9
  movdqu %xmm0, (%rbx) #86.29
  addq $16, %rbx #86.29
  cmpl %eax, %r10d #86.9
  jb ..B1.68 # Prob 82% #86.9
..B1.70: # Preds ..B1.68 ..B1.75
  cmpl 40(%rsp), %eax #86.9[spill]
  jae ..B1.73 # Prob 0% #86.9
..B1.71: # Preds ..B1.70
  movl 40(%rsp), %r10d #[spill]
..B1.72: # Preds ..B1.72 ..B1.71
  incl %eax #86.9
  movb $255, (%rbx) #86.29
  incq %rbx #86.29
  cmpl %r10d, %eax #86.9
  jb ..B1.72 # Prob 82% #86.9
..B1.73: # Preds ..B1.70 ..B1.72
  movslq %edx, %rdx #86.41
  jmp ..B1.28 # Prob 100% #86.41
..B1.75: # Preds ..B1.66
  xorl %eax, %eax #86.9
  jmp ..B1.70 # Prob 100% #86.9
..B1.76: # Preds ..B1.40
  cmpl $16, %r14d #103.9
  jl ..B1.85 # Prob 10% #103.9
..B1.77: # Preds ..B1.76
  movl %r14d, %eax #103.9
  xorl %edx, %edx #103.9
  andl $-16, %eax #103.9
  pcmpeqd %xmm0, %xmm0 #103.9
..B1.78: # Preds ..B1.78 ..B1.77
  addl $16, %edx #103.9
  movdqu %xmm0, (%rdi) #103.29
  addq $16, %rdi #103.29
  cmpl %eax, %edx #103.9
  jb ..B1.78 # Prob 82% #103.9
..B1.80: # Preds ..B1.78 ..B1.85
  cmpl %r14d, %eax #103.9
  jae ..B1.83 # Prob 0% #103.9
..B1.82: # Preds ..B1.80 ..B1.82
  incl %eax #103.9
  movb $255, (%rdi) #103.29
  incq %rdi #103.29
  cmpl %r14d, %eax #103.9
  jb ..B1.82 # Prob 82% #103.9
..B1.83: # Preds ..B1.80 ..B1.82
  movslq %ecx, %r13 #103.41
  jmp ..B1.42 # Prob 100% #103.41
..B1.85: # Preds ..B1.76
  xorl %eax, %eax #103.9
  jmp ..B1.80 # Prob 100% #103.9
..B1.87: # Preds ..B1.56
  cmpl $16, %r14d #118.3
  jb ..B1.51 # Prob 10% #118.3
..B1.88: # Preds ..B1.87
  movl %r14d, %r9d #118.3
  xorl %r10d, %r10d #118.3
  andl $-16, %r9d #118.3
  xorl %ecx, %ecx #119.6
..B1.89: # Preds ..B1.89 ..B1.88
  movdqu (%r10,%r8), %xmm0 #119.13
  addl $16, %r10d #118.3
  movdqu %xmm0, (%rcx,%r13) #119.6
  addq $16, %rcx #118.3
  cmpl %r9d, %r10d #118.3
  jb ..B1.89 # Prob 82% #118.3
  jmp ..B1.52 # Prob 100% #118.3
src_data:
hash_table:
dst_data:
.L_2__STRING.0:
