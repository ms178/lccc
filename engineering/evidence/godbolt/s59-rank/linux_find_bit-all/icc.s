main:
..B1.1: # Preds ..B1.0
  pushq %rbp #133.1
  movq %rsp, %rbp #133.1
  andq $-128, %rsp #133.1
  pushq %r15 #133.1
  pushq %rbx #133.1
  subq $112, %rsp #133.1
  movl $3, %edi #133.1
  xorl %esi, %esi #133.1
  call __intel_new_feature_proc_init #133.1
..B1.44: # Preds ..B1.1
  stmxcsr 48(%rsp) #133.1
  xorl %eax, %eax #135.26
  movq $-1, %rdi #119.3
  xorl %esi, %esi #135.26
  orl $32832, 48(%rsp) #133.1
  xorl %r9d, %r9d #78.3
  ldmxcsr 48(%rsp) #133.1
  movq $32, (%rsp) #115.3
  xorl %edx, %edx #78.3
  movq $4, 8(%rsp) #116.3
  movq %rax, 16(%rsp) #117.3
  movq %rax, 24(%rsp) #118.3
  movq %rdi, 32(%rsp) #119.3
  movq %rdi, 40(%rsp) #120.3
..B1.2: # Preds ..B1.3 ..B1.44
  lea 64(%rdx), %rcx #82.5
  cmpq $192, %rcx #82.42
  jae ..B1.41 # Prob 20% #82.42
..B1.3: # Preds ..B1.2
  incq %r9 #84.5
  addq $64, %rdx #84.5
  movq 24(%rsp,%r9,8), %rcx #125.41
  notq %rcx #85.29
  andq (%rsp,%r9,8), %rcx #85.29
  je ..B1.2 # Prob 82% #81.11
..B1.4: # Preds ..B1.3
  movl %ecx, %edx #38.15
  movq %rcx, %r10 #40.5
  shrq $32, %r10 #40.5
  xorl %r8d, %r8d #39.5
  testq %rdx, %rdx #39.5
  movl $32, %edx #39.5
  cmove %r10, %rcx #40.5
  cmovne %r8d, %edx #39.5
  movzwl %cx, %r11d #42.15
  testq %r11, %r11 #42.28
  jne ..B1.6 # Prob 50% #42.28
..B1.5: # Preds ..B1.4
  shrq $16, %rcx #44.5
  addl $16, %edx #43.5
..B1.6: # Preds ..B1.5 ..B1.4
  movl %ecx, %r10d #46.26
  testb %r10b, %r10b #46.26
  jne ..B1.8 # Prob 50% #46.26
..B1.7: # Preds ..B1.6
  shrq $8, %rcx #48.5
  addl $8, %edx #47.5
..B1.8: # Preds ..B1.7 ..B1.6
  testq $15, %rcx #50.15
  jne ..B1.10 # Prob 50% #50.25
..B1.9: # Preds ..B1.8
  shrq $4, %rcx #52.5
  addl $4, %edx #51.5
..B1.10: # Preds ..B1.9 ..B1.8
  testq $3, %rcx #54.15
  jne ..B1.12 # Prob 50% #54.25
..B1.11: # Preds ..B1.10
  shrq $2, %rcx #56.5
  addl $2, %edx #55.5
..B1.12: # Preds ..B1.11 ..B1.10
  testq $1, %rcx #58.15
  lea 1(%rdx), %r11d #59.5
  cmove %r11d, %edx #89.19
  shlq $6, %r9 #88.20
  addq %rdx, %r9 #88.36
  cmpq $192, %r9 #89.19
  ja ..B1.15 # Prob 50% #89.19
..B1.13: # Preds ..B1.12
  je ..B1.15 # Prob 50% #126.10
..B1.14: # Preds ..B1.13
  movl $2, %eax #138.12
  addq $112, %rsp #138.12
  popq %rbx #138.12
  popq %r15 #138.12
  movq %rbp, %rsp #138.12
  popq %rbp #138.12
  ret #138.12
..B1.15: # Preds ..B1.12 ..B1.13 ..B1.41
  movdqu .L_2il0floatpacket.8(%rip), %xmm4 #103.34
  movq %rax, %rdx #97.3
  movdqu .L_2il0floatpacket.6(%rip), %xmm0 #103.27
  movdqa %xmm4, %xmm12 #103.34
  movdqa %xmm4, %xmm1 #103.34
  pcmpeqd %xmm8, %xmm8 #99.25
  movdqu .L_2il0floatpacket.0(%rip), %xmm3 #97.31
  pand %xmm0, %xmm12 #103.34
  movdqu .L_2il0floatpacket.1(%rip), %xmm10 #97.31
  pandn %xmm0, %xmm1 #103.34
  movdqu .L_2il0floatpacket.2(%rip), %xmm11 #102.39
  movdqu .L_2il0floatpacket.3(%rip), %xmm9 #102.39
  movdqu .L_2il0floatpacket.4(%rip), %xmm7 #101.23
  movdqu .L_2il0floatpacket.5(%rip), %xmm6 #101.14
  movdqu .L_2il0floatpacket.7(%rip), %xmm5 #103.34
..B1.16: # Preds ..B1.16 ..B1.15
  movdqa %xmm6, %xmm2 #101.23
  movdqa %xmm6, %xmm15 #102.46
  pand %xmm10, %xmm2 #101.23
  pand %xmm9, %xmm15 #102.46
  pcmpeqd %xmm7, %xmm2 #101.23
  pand %xmm5, %xmm15 #103.34
  pshufd $177, %xmm2, %xmm13 #101.23
  movdqa %xmm4, %xmm14 #103.34
  pand %xmm13, %xmm2 #101.23
  pand %xmm15, %xmm14 #103.34
  pshufd $14, %xmm15, %xmm13 #103.34
  movdqa %xmm12, %xmm0 #103.34
  psllq %xmm14, %xmm0 #103.34
  pand %xmm4, %xmm13 #103.34
  movdqa %xmm1, %xmm14 #103.34
  psllq %xmm13, %xmm14 #103.34
  por %xmm14, %xmm0 #103.34
  pand %xmm2, %xmm0 #103.34
  pandn %xmm8, %xmm2 #104.7
  movdqu %xmm0, linux_bitmap_a(,%rdx,8) #103.7
  movdqu %xmm2, linux_bitmap_b(,%rdx,8) #104.7
  addq $2, %rdx #97.3
  paddq %xmm3, %xmm10 #97.31
  paddq %xmm11, %xmm9 #102.39
  cmpq $16384, %rdx #97.3
  jb ..B1.16 # Prob 99% #97.3
..B1.17: # Preds ..B1.16
  movl %r8d, %edx #141.33
..B1.18: # Preds ..B1.37 ..B1.17
  movl %edx, %r10d #148.43
  movq %rax, %rcx #142.51
  shlq $19, %r10 #148.51
  andq $63, %rcx #142.51
  jmp ..B1.19 # Prob 100% #142.51
..B1.34: # Preds ..B1.33
  lea (%rbx,%r10), %rcx #148.51
  xorq %rcx, %rsi #148.9
  lea 1(%rbx), %rcx #149.24
..B1.19: # Preds ..B1.34 ..B1.18
  cmpq $1048576, %rcx #74.16
  jae ..B1.37 # Prob 28% #74.16
..B1.20: # Preds ..B1.19
  movq %rcx, %rbx #78.19
  movq %rdi, %r11 #77.27
  shrq $6, %rbx #78.19
  shlq %cl, %r11 #77.27
  movq linux_bitmap_b(,%rbx,8), %r9 #145.56
  notq %r9 #79.28
  andq linux_bitmap_a(,%rbx,8), %r9 #79.28
  andq %r11, %r9 #79.44
  jne ..B1.25 # Prob 1% #81.11
..B1.22: # Preds ..B1.20 ..B1.23
  movq %rbx, %rcx #82.25
  shlq $6, %rcx #82.25
  addq $64, %rcx #82.25
  cmpq $1048576, %rcx #82.42
  jae ..B1.37 # Prob 20% #82.42
..B1.23: # Preds ..B1.22
  incq %rbx #84.5
  movq linux_bitmap_b(,%rbx,8), %r9 #145.56
  notq %r9 #85.29
  andq linux_bitmap_a(,%rbx,8), %r9 #85.29
  je ..B1.22 # Prob 82% #81.11
..B1.25: # Preds ..B1.23 ..B1.20
  movl %r9d, %r11d #38.15
  movl $32, %ecx #39.5
  testq %r11, %r11 #39.5
  movq %r9, %r15 #40.5
  cmovne %r8d, %ecx #39.5
  shrq $32, %r15 #40.5
  testq %r11, %r11 #40.5
  cmove %r15, %r9 #40.5
  movzwl %r9w, %r11d #42.15
  testq %r11, %r11 #42.28
  jne ..B1.27 # Prob 50% #42.28
..B1.26: # Preds ..B1.25
  shrq $16, %r9 #44.5
  addl $16, %ecx #43.5
..B1.27: # Preds ..B1.26 ..B1.25
  movl %r9d, %r11d #46.26
  testb %r11b, %r11b #46.26
  jne ..B1.29 # Prob 50% #46.26
..B1.28: # Preds ..B1.27
  shrq $8, %r9 #48.5
  addl $8, %ecx #47.5
..B1.29: # Preds ..B1.28 ..B1.27
  testq $15, %r9 #50.15
  jne ..B1.31 # Prob 50% #50.25
..B1.30: # Preds ..B1.29
  shrq $4, %r9 #52.5
  addl $4, %ecx #51.5
..B1.31: # Preds ..B1.30 ..B1.29
  testq $3, %r9 #54.15
  jne ..B1.33 # Prob 50% #54.25
..B1.32: # Preds ..B1.31
  shrq $2, %r9 #56.5
  addl $2, %ecx #55.5
..B1.33: # Preds ..B1.32 ..B1.31
  testq $1, %r9 #58.15
  lea 1(%rcx), %r15d #59.5
  cmove %r15d, %ecx #89.19
  movl $1048576, %r9d #89.19
  shlq $6, %rbx #88.20
  addq %rcx, %rbx #88.36
  cmpq $1048576, %rbx #89.19
  cmova %r9, %rbx #89.19
  cmpq $1048576, %rbx #147.17
  jb ..B1.34 # Prob 50% #147.17
..B1.37: # Preds ..B1.22 ..B1.33 ..B1.19
  incl %edx #141.33
  lea (%rax,%rax,8), %rcx #155.52
  lea (%rcx,%rax,4), %rbx #155.52
  movzbl %bl, %r11d #155.60
  lea (,%rax,8), %r9 #156.62
  shlq $9, %r11 #155.69
  subq %rax, %r9 #156.62
  movl %edx, %eax #141.33
  movq 40+linux_bitmap_b(%r11), %r10 #156.7
  btcq %r9, %r10 #156.7
  movq %r10, 40+linux_bitmap_b(%r11) #156.7
  cmpl $1024, %edx #141.25
  jb ..B1.18 # Prob 99% #141.25
..B1.38: # Preds ..B1.37
  movl $.L_2__STRING.0, %edi #161.3
  xorl %eax, %eax #161.3
  call printf #161.3
..B1.39: # Preds ..B1.38
  xorl %eax, %eax #162.10
  addq $112, %rsp #162.10
  popq %rbx #162.10
  popq %r15 #162.10
  movq %rbp, %rsp #162.10
  popq %rbp #162.10
  ret #162.10
..B1.41: # Preds ..B1.2
  xorl %r8d, %r8d #39.5
  jmp ..B1.15 # Prob 100% #39.5
linux_bitmap_a:
linux_bitmap_b:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
.L_2il0floatpacket.6:
.L_2il0floatpacket.7:
.L_2il0floatpacket.8:
.L_2__STRING.0:
