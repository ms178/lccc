main:
..B1.1: # Preds ..B1.0
  pushq %rbp #117.1
  movq %rsp, %rbp #117.1
  andq $-128, %rsp #117.1
  pushq %r12 #117.1
  pushq %r13 #117.1
  pushq %r14 #117.1
  pushq %r15 #117.1
  pushq %rbx #117.1
  subq $472, %rsp #117.1
  movl $3, %edi #117.1
  xorl %esi, %esi #117.1
  call __intel_new_feature_proc_init #117.1
..B1.23: # Preds ..B1.1
  stmxcsr 32(%rsp) #117.1
  movq $0, 88(%rsp) #120.16[spill]
  orl $32832, 32(%rsp) #117.1
  movups state.130.0.0.2(%rip), %xmm0 #98.16
  movups 16+state.130.0.0.2(%rip), %xmm1 #98.16
  ldmxcsr 32(%rsp) #117.1
  movups %xmm0, (%rsp) #98.16
  movups %xmm1, 16(%rsp) #98.16
..B1.2: # Preds ..B1.23
  movups data.130.0.0.2(%rip), %xmm0 #102.16
  movups 16+data.130.0.0.2(%rip), %xmm1 #102.16
  movups 32+data.130.0.0.2(%rip), %xmm2 #102.16
  movups 48+data.130.0.0.2(%rip), %xmm3 #102.16
..B1.3: # Preds ..B1.2
  movdqu %xmm0, 96(%rsp) #58.5
  xorl %eax, %eax #59.3
  movdqu %xmm1, 112(%rsp) #58.5
  movdqu %xmm2, 128(%rsp) #58.5
  movdqu %xmm3, 144(%rsp) #58.5
..B1.4: # Preds ..B1.4 ..B1.3
  movl 152(%rsp,%rax,4), %ecx #60.12
  movl %ecx, %r8d #60.12
  movl 100(%rsp,%rax,4), %esi #60.40
  movl %ecx, %edx #60.12
  movl %esi, %edi #60.40
  movl %esi, %ebx #60.40
  shldl $15, %ecx, %r8d #60.12
  shldl $13, %ecx, %edx #60.12
  shldl $25, %esi, %edi #60.40
  shldl $14, %esi, %ebx #60.40
  shrl $10, %ecx #60.12
  xorl %edx, %r8d #60.12
  shrl $3, %esi #60.40
  xorl %ebx, %edi #60.40
  xorl %ecx, %r8d #60.12
  xorl %esi, %edi #60.40
  addl 132(%rsp,%rax,4), %r8d #60.29
  addl 96(%rsp,%rax,4), %edi #60.58
  addl %edi, %r8d #60.40
  movl %r8d, 160(%rsp,%rax,4) #60.5
  incq %rax #59.3
  cmpq $48, %rax #59.3
  jb ..B1.4 # Prob 97% #59.3
..B1.5: # Preds ..B1.4
  movl 16(%rsp), %eax #108.20
  movl %eax, %r10d #66.3
  movl (%rsp), %r15d #108.20
  movl %r15d, %edi #62.3
  movl 4(%rsp), %r14d #108.20
  movl %r14d, %esi #63.3
  movl 8(%rsp), %r13d #108.20
  movl %r13d, %r12d #64.3
  movl 12(%rsp), %r11d #108.20
  movl 20(%rsp), %r9d #108.20
  movq $0, 80(%rsp) #71.3[spill]
  movl 24(%rsp), %r8d #108.20
  movl %r8d, %ebx #68.3
  movl 28(%rsp), %ecx #108.20
  movl %ecx, %edx #69.3
  movl %eax, 64(%rsp) #71.3[spill]
  movl %r11d, 56(%rsp) #108.20[spill]
  movl %r9d, 72(%rsp) #108.20[spill]
  movl %r13d, 48(%rsp) #71.3[spill]
  movl %r14d, 40(%rsp) #71.3[spill]
  movl %r15d, 32(%rsp) #71.3[spill]
  movq 80(%rsp), %rax #71.3[spill]
..B1.6: # Preds ..B1.6 ..B1.5
  movl %r10d, %r13d #72.14
  movl %r10d, %r14d #72.14
  shldl $26, %r10d, %r13d #72.14
  shldl $21, %r10d, %r14d #72.14
  movl %r10d, %r15d #72.14
  xorl %r14d, %r13d #72.14
  shldl $7, %r10d, %r15d #72.14
  movl %r10d, %r14d #72.23
  xorl %r15d, %r13d #72.14
  movl %r10d, %r15d #72.23
  notl %r14d #72.23
  andl %r9d, %r15d #72.23
  andl %ebx, %r14d #72.23
  xorl %r14d, %r15d #72.23
  movl %edi, %r14d #73.10
  addl %r15d, %r13d #72.14
  movl %ebx, %r15d #74.5
  addl %edx, %r13d #72.23
  movl %r9d, %edx #75.5
  movl K(,%rax,8), %r9d #72.37
  movl %r10d, %ebx #76.5
  addl 96(%rsp,%rax,8), %r9d #72.44
  movl %edi, %r10d #73.10
  addl %r9d, %r13d #72.37
  shldl $30, %edi, %r14d #73.10
  shldl $19, %edi, %r10d #73.10
  xorl %r10d, %r14d #73.10
  lea (%r11,%r13), %r9d #77.13
  movl %edi, %r11d #73.10
  movl %esi, %r10d #73.19
  shldl $10, %edi, %r11d #73.10
  xorl %r11d, %r14d #73.10
  movl %esi, %r11d #73.19
  xorl %r12d, %r11d #73.19
  andl %r12d, %r10d #73.19
  andl %edi, %r11d #73.19
  xorl %r10d, %r11d #73.19
  movl %r12d, %r10d #78.5
  addl %r11d, %r14d #73.19
  movl %esi, %r11d #79.5
  movl %edi, %r12d #80.5
  movl %r9d, %edi #72.14
  shldl $26, %r9d, %edi #72.14
  lea (%r13,%r14), %esi #81.14
  movl %r9d, %r13d #72.14
  shldl $21, %r9d, %r13d #72.14
  movl %r9d, %r14d #72.14
  xorl %r13d, %edi #72.14
  shldl $7, %r9d, %r14d #72.14
  movl %r9d, %r13d #72.23
  xorl %r14d, %edi #72.14
  movl %ebx, %r14d #72.23
  notl %r13d #72.23
  andl %r9d, %r14d #72.23
  andl %edx, %r13d #72.23
  xorl %r13d, %r14d #72.23
  movl %esi, %r13d #73.10
  addl %r14d, %edi #72.14
  movl %esi, %r14d #73.10
  addl %r15d, %edi #72.23
  movl 4+K(,%rax,8), %r15d #72.37
  addl 100(%rsp,%rax,8), %r15d #72.44
  incq %rax #71.3
  shldl $30, %esi, %r13d #73.10
  shldl $19, %esi, %r14d #73.10
  addl %r15d, %edi #72.37
  movl %esi, %r15d #73.10
  shldl $10, %esi, %r15d #73.10
  xorl %r14d, %r13d #73.10
  movl %r12d, %r14d #73.19
  xorl %r15d, %r13d #73.10
  movl %r12d, %r15d #73.19
  xorl %r11d, %r15d #73.19
  andl %r11d, %r14d #73.19
  andl %esi, %r15d #73.19
  addl %edi, %r10d #77.13
  xorl %r14d, %r15d #73.19
  addl %r15d, %r13d #73.19
  addl %r13d, %edi #81.14
  cmpq $32, %rax #71.3
  jb ..B1.6 # Prob 96% #71.3
..B1.7: # Preds ..B1.6
  movl 32(%rsp), %r15d #[spill]
  addl %r15d, %edi #84.3
  movl 40(%rsp), %r14d #[spill]
  addl %r14d, %esi #85.3
  addl %ecx, %edx #91.3
  movl 64(%rsp), %eax #[spill]
  movl 48(%rsp), %r13d #[spill]
  cmpl $-1166534977, %edi #109.19
  jne ..B1.20 # Prob 28% #109.19
..B1.8: # Preds ..B1.7
  cmpl $-1895706646, %esi #109.45
  jne ..B1.20 # Prob 28% #109.45
..B1.9: # Preds ..B1.8
  cmpl $-234875475, %edx #110.19
  jne ..B1.20 # Prob 50% #110.19
..B1.10: # Preds ..B1.9
  movdqu .L_2il0floatpacket.2(%rip), %xmm6 #139.49
  addl %r8d, %ebx #90.3
  movdqu .L_2il0floatpacket.3(%rip), %xmm8 #139.49
  movdqa %xmm6, %xmm4 #139.49
  movdqu .L_2il0floatpacket.5(%rip), %xmm3 #139.49
  psrlq $32, %xmm8 #139.49
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #139.9
  psrlq $32, %xmm3 #139.49
  pmuludq %xmm1, %xmm4 #139.49
  addl %eax, %r10d #88.3
  pmuludq %xmm8, %xmm3 #139.49
  movdqu .L_2il0floatpacket.4(%rip), %xmm0 #139.49
  psllq $32, %xmm3 #139.49
  movdqu .L_2il0floatpacket.6(%rip), %xmm2 #139.49
  pand %xmm0, %xmm4 #139.49
  movdqu .L_2il0floatpacket.1(%rip), %xmm7 #139.9
  por %xmm3, %xmm4 #139.49
  movdqa %xmm6, %xmm3 #139.49
  psrlq $32, %xmm2 #139.49
  pmuludq %xmm7, %xmm3 #139.49
  addl %r13d, %r12d #86.3
  pmuludq %xmm8, %xmm2 #139.49
  movdqu .L_2il0floatpacket.7(%rip), %xmm5 #139.9
  pand %xmm0, %xmm3 #139.49
  psllq $32, %xmm2 #139.49
  paddd %xmm5, %xmm1 #139.9
  por %xmm2, %xmm3 #139.49
  movdqa %xmm6, %xmm2 #139.49
  paddd %xmm5, %xmm7 #139.9
  pmuludq %xmm1, %xmm2 #139.49
  psrlq $32, %xmm1 #139.49
  pmuludq %xmm7, %xmm6 #139.49
  pmuludq %xmm8, %xmm1 #139.49
  psrlq $32, %xmm7 #139.49
  pand %xmm0, %xmm2 #139.49
  pmuludq %xmm7, %xmm8 #139.49
  psllq $32, %xmm1 #139.49
  addl 72(%rsp), %r9d #89.3[spill]
  pand %xmm6, %xmm0 #139.49
  addl 56(%rsp), %r11d #87.3[spill]
  psllq $32, %xmm8 #139.49
  movl %edx, 28(%rsp) #108.20
  xorl %edx, %edx #127.3
  movl %ebx, 24(%rsp) #108.20
  por %xmm1, %xmm2 #139.49
  movl %r9d, 20(%rsp) #108.20
  por %xmm8, %xmm0 #139.49
  movl %r10d, 16(%rsp) #108.20
  movl %r11d, 12(%rsp) #108.20
  movl %r12d, 8(%rsp) #108.20
  movl %esi, 4(%rsp) #108.20
  movl %edi, (%rsp) #108.20
..B1.11: # Preds ..B1.17 ..B1.10
  xorl %edi, %edi #137.5
  movl %edx, %r14d #128.29
  movl $1013904242, %r9d #130.5
  movl $1359893119, %r8d #132.5
  xorl $1779033703, %r14d #128.29
  movl %edi, 72(%rsp) #138.12[spill]
  movl $-1150833019, %esi #129.5
  movl %edi, 8(%rsp) #138.12[spill]
  movl $-1521486534, %r13d #131.5
  movl %r8d, 48(%rsp) #138.12[spill]
  movl $-1694144372, %r15d #133.5
  movl %r9d, 32(%rsp) #138.12[spill]
  movl $528734635, %ebx #134.5
  movl %edx, (%rsp) #138.12[spill]
  movl $1541459225, %eax #135.5
..B1.12: # Preds ..B1.16 ..B1.11
  movd 48(%rsp), %xmm1 #139.64[spill]
  movd %r15d, %xmm5 #139.64
  movd 32(%rsp), %xmm10 #139.64[spill]
  movd %ebx, %xmm7 #139.64
  movd 72(%rsp), %xmm11 #139.29[spill]
  movd %eax, %xmm6 #139.64
  movd %esi, %xmm8 #139.64
  movd %r13d, %xmm9 #139.64
  movd %r14d, %xmm12 #139.64
  movdqa %xmm4, %xmm13 #139.49
  punpckldq %xmm5, %xmm1 #139.64
  movdqa %xmm3, %xmm14 #139.49
  punpckldq %xmm6, %xmm7 #139.64
  movdqa %xmm2, %xmm15 #139.49
  punpckldq %xmm8, %xmm12 #139.64
  xorl %edx, %edx #59.3
  punpckldq %xmm9, %xmm10 #139.64
  pshufd $0, %xmm11, %xmm5 #139.29
  punpcklqdq %xmm7, %xmm1 #139.64
  paddd %xmm5, %xmm13 #139.49
  punpcklqdq %xmm10, %xmm12 #139.64
  paddd %xmm5, %xmm14 #139.49
  paddd %xmm5, %xmm15 #139.49
  paddd %xmm0, %xmm5 #139.49
  pxor %xmm12, %xmm13 #139.64
  pxor %xmm1, %xmm14 #139.64
  pxor %xmm12, %xmm15 #139.64
  pxor %xmm1, %xmm5 #139.64
  movdqu %xmm13, 96(%rsp) #58.5
  movdqu %xmm14, 112(%rsp) #58.5
  movdqu %xmm15, 128(%rsp) #58.5
  movdqu %xmm5, 144(%rsp) #58.5
..B1.13: # Preds ..B1.13 ..B1.12
  movl 152(%rsp,%rdx,4), %edi #60.12
  movl %edi, %r11d #60.12
  movl 100(%rsp,%rdx,4), %r9d #60.40
  movl %edi, %ecx #60.12
  movl %r9d, %r10d #60.40
  movl %r9d, %r8d #60.40
  shldl $15, %edi, %r11d #60.12
  shldl $13, %edi, %ecx #60.12
  shldl $25, %r9d, %r10d #60.40
  shldl $14, %r9d, %r8d #60.40
  shrl $10, %edi #60.12
  xorl %ecx, %r11d #60.12
  shrl $3, %r9d #60.40
  xorl %r8d, %r10d #60.40
  xorl %edi, %r11d #60.12
  xorl %r9d, %r10d #60.40
  addl 132(%rsp,%rdx,4), %r11d #60.29
  addl 96(%rsp,%rdx,4), %r10d #60.58
  addl %r10d, %r11d #60.40
  movl %r11d, 160(%rsp,%rdx,4) #60.5
  incq %rdx #59.3
  cmpq $48, %rdx #59.3
  jb ..B1.13 # Prob 97% #59.3
..B1.14: # Preds ..B1.13
  movq $0, 64(%rsp) #71.3[spill]
  movl %r14d, %edi #62.3
  movl %esi, 24(%rsp) #71.3[spill]
  movl %esi, %r8d #63.3
  movl 32(%rsp), %r12d #64.3[spill]
  movl %r13d, %r11d #65.3
  movl 48(%rsp), %r10d #66.3[spill]
  movl %r15d, %ecx #67.3
  movq 64(%rsp), %rsi #71.3[spill]
  movl %ebx, %edx #68.3
  movl %r15d, 56(%rsp) #71.3[spill]
  movl %eax, %r9d #69.3
  movl %r13d, 40(%rsp) #71.3[spill]
  movl %r14d, 16(%rsp) #71.3[spill]
..B1.15: # Preds ..B1.15 ..B1.14
  movl %r10d, %r13d #72.14
  movl %r10d, %r14d #72.14
  shldl $26, %r10d, %r13d #72.14
  shldl $21, %r10d, %r14d #72.14
  movl %r10d, %r15d #72.14
  xorl %r14d, %r13d #72.14
  shldl $7, %r10d, %r15d #72.14
  movl %r10d, %r14d #72.23
  xorl %r15d, %r13d #72.14
  movl %r10d, %r15d #72.23
  notl %r14d #72.23
  andl %ecx, %r15d #72.23
  andl %edx, %r14d #72.23
  xorl %r14d, %r15d #72.23
  movl %edi, %r14d #73.10
  addl %r15d, %r13d #72.14
  movl %edx, %r15d #74.5
  addl %r9d, %r13d #72.23
  movl %ecx, %r9d #75.5
  movl K(,%rsi,8), %ecx #72.37
  movl %r10d, %edx #76.5
  addl 96(%rsp,%rsi,8), %ecx #72.44
  movl %edi, %r10d #73.10
  addl %ecx, %r13d #72.37
  shldl $30, %edi, %r14d #73.10
  shldl $19, %edi, %r10d #73.10
  xorl %r10d, %r14d #73.10
  lea (%r11,%r13), %ecx #77.13
  movl %edi, %r11d #73.10
  movl %r8d, %r10d #73.19
  shldl $10, %edi, %r11d #73.10
  xorl %r11d, %r14d #73.10
  movl %r8d, %r11d #73.19
  xorl %r12d, %r11d #73.19
  andl %r12d, %r10d #73.19
  andl %edi, %r11d #73.19
  xorl %r10d, %r11d #73.19
  movl %r12d, %r10d #78.5
  addl %r11d, %r14d #73.19
  movl %r8d, %r11d #79.5
  movl %edi, %r12d #80.5
  movl %ecx, %edi #72.14
  shldl $26, %ecx, %edi #72.14
  lea (%r13,%r14), %r8d #81.14
  movl %ecx, %r13d #72.14
  shldl $21, %ecx, %r13d #72.14
  movl %ecx, %r14d #72.14
  xorl %r13d, %edi #72.14
  shldl $7, %ecx, %r14d #72.14
  movl %ecx, %r13d #72.23
  xorl %r14d, %edi #72.14
  movl %edx, %r14d #72.23
  notl %r13d #72.23
  andl %ecx, %r14d #72.23
  andl %r9d, %r13d #72.23
  xorl %r13d, %r14d #72.23
  movl %r8d, %r13d #73.10
  addl %r14d, %edi #72.14
  movl %r8d, %r14d #73.10
  addl %r15d, %edi #72.23
  movl 4+K(,%rsi,8), %r15d #72.37
  addl 100(%rsp,%rsi,8), %r15d #72.44
  incq %rsi #71.3
  shldl $30, %r8d, %r13d #73.10
  shldl $19, %r8d, %r14d #73.10
  addl %r15d, %edi #72.37
  movl %r8d, %r15d #73.10
  shldl $10, %r8d, %r15d #73.10
  xorl %r14d, %r13d #73.10
  movl %r12d, %r14d #73.19
  xorl %r15d, %r13d #73.10
  movl %r12d, %r15d #73.19
  xorl %r11d, %r15d #73.19
  andl %r11d, %r14d #73.19
  andl %r8d, %r15d #73.19
  addl %edi, %r10d #77.13
  xorl %r14d, %r15d #73.19
  addl %r15d, %r13d #73.19
  addl %r13d, %edi #81.14
  cmpq $32, %rsi #71.3
  jb ..B1.15 # Prob 96% #71.3
..B1.16: # Preds ..B1.15
  movl 16(%rsp), %r14d #[spill]
  addl %r9d, %eax #91.3
  addl %edi, %r14d #84.3
  addl %edx, %ebx #90.3
  movl 56(%rsp), %r15d #[spill]
  movl 40(%rsp), %r13d #[spill]
  addl %ecx, %r15d #89.3
  movl %r14d, %ecx #141.25
  addl %r11d, %r13d #87.3
  shlq $32, %rcx #141.37
  movl %r13d, %edx #141.60
  shlq $16, %rdx #141.72
  xorq %rax, %rcx #141.43
  movl 8(%rsp), %edi #137.5[spill]
  xorq %rdx, %rcx #141.72
  incl %edi #137.5
  movl 24(%rsp), %esi #[spill]
  addl %r12d, 32(%rsp) #86.3[spill]
  addl %r8d, %esi #85.3
  addl %r10d, 48(%rsp) #88.3[spill]
  addl $1664525, 72(%rsp) #137.5[spill]
  addq %rcx, 88(%rsp) #141.7[spill]
  movl %edi, 8(%rsp) #137.5[spill]
  cmpl $131072, %edi #137.5
  jb ..B1.12 # Prob 99% #137.5
..B1.17: # Preds ..B1.16
  movl (%rsp), %edx #[spill]
  incl %edx #127.3
  cmpl $8, %edx #127.3
  jb ..B1.11 # Prob 88% #127.3
..B1.18: # Preds ..B1.17
  movl $.L_2__STRING.0, %edi #145.3
  xorl %eax, %eax #145.3
  movq 88(%rsp), %rsi #145.3[spill]
  call printf #145.3
..B1.19: # Preds ..B1.18
  xorl %eax, %eax #146.10
  addq $472, %rsp #146.10
  popq %rbx #146.10
  popq %r15 #146.10
  popq %r14 #146.10
  popq %r13 #146.10
  popq %r12 #146.10
  movq %rbp, %rsp #146.10
  popq %rbp #146.10
  ret #146.10
..B1.20: # Preds ..B1.7 ..B1.9 ..B1.8
  movl $2, %eax #125.12
  addq $472, %rsp #125.12
  popq %rbx #125.12
  popq %r15 #125.12
  popq %r14 #125.12
  popq %r13 #125.12
  popq %r12 #125.12
  movq %rbp, %rsp #125.12
  popq %rbp #125.12
  ret #125.12
data.130.0.0.2:
state.130.0.0.2:
K:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
.L_2il0floatpacket.6:
.L_2il0floatpacket.7:
.L_2__STRING.0:
