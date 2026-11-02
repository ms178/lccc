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
  vstmxcsr 48(%rsp) #133.1
  xorl %r9d, %r9d #135.26
  movq $-1, %rax #119.3
  xorl %esi, %esi #135.26
  orl $32832, 48(%rsp) #133.1
  xorl %r8d, %r8d #78.3
  vldmxcsr 48(%rsp) #133.1
  movq $32, (%rsp) #115.3
  xorl %edx, %edx #78.3
  movq $4, 8(%rsp) #116.3
  movq %r9, 16(%rsp) #117.3
  movq %r9, 24(%rsp) #118.3
  movq %rax, 32(%rsp) #119.3
  movq %rax, 40(%rsp) #120.3
..B1.2: # Preds ..B1.3 ..B1.44
  lea 64(%rdx), %rcx #82.5
  cmpq $192, %rcx #82.42
  jae ..B1.41 # Prob 20% #82.42
..B1.3: # Preds ..B1.2
  incq %r8 #84.5
  addq $64, %rdx #84.5
  movq 24(%rsp,%r8,8), %rcx #125.41
  andn (%rsp,%r8,8), %rcx, %rdi #85.29
  je ..B1.2 # Prob 82% #81.11
..B1.4: # Preds ..B1.3
  movl %edi, %ecx #38.15
  movq %rdi, %r10 #40.5
  shrq $32, %r10 #40.5
  xorl %edx, %edx #39.5
  testq %rcx, %rcx #39.5
  movl $32, %ecx #39.5
  cmove %r10, %rdi #40.5
  cmovne %edx, %ecx #39.5
  movzwl %di, %r11d #42.15
  testq %r11, %r11 #42.28
  jne ..B1.6 # Prob 50% #42.28
..B1.5: # Preds ..B1.4
  shrq $16, %rdi #44.5
  addl $16, %ecx #43.5
..B1.6: # Preds ..B1.5 ..B1.4
  movl %edi, %r10d #46.26
  testb %r10b, %r10b #46.26
  jne ..B1.8 # Prob 50% #46.26
..B1.7: # Preds ..B1.6
  shrq $8, %rdi #48.5
  addl $8, %ecx #47.5
..B1.8: # Preds ..B1.7 ..B1.6
  testq $15, %rdi #50.15
  jne ..B1.10 # Prob 50% #50.25
..B1.9: # Preds ..B1.8
  shrq $4, %rdi #52.5
  addl $4, %ecx #51.5
..B1.10: # Preds ..B1.9 ..B1.8
  testq $3, %rdi #54.15
  jne ..B1.12 # Prob 50% #54.25
..B1.11: # Preds ..B1.10
  shrq $2, %rdi #56.5
  addl $2, %ecx #55.5
..B1.12: # Preds ..B1.11 ..B1.10
  testq $1, %rdi #58.15
  lea 1(%rcx), %r11d #59.5
  cmove %r11d, %ecx #89.19
  shlq $6, %r8 #88.20
  addq %rcx, %r8 #88.36
  cmpq $192, %r8 #89.19
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
  vmovdqu .L_2il0floatpacket.0(%rip), %ymm9 #97.31
  movq %r9, %rcx #97.3
  vmovdqu .L_2il0floatpacket.6(%rip), %ymm7 #97.31
  vmovdqu .L_2il0floatpacket.1(%rip), %ymm8 #102.39
  vmovdqu .L_2il0floatpacket.7(%rip), %ymm5 #102.39
  vmovdqu .L_2il0floatpacket.2(%rip), %ymm2 #101.23
  vmovdqu .L_2il0floatpacket.3(%rip), %ymm1 #101.14
  vmovdqu .L_2il0floatpacket.4(%rip), %ymm6 #103.27
  vmovdqu .L_2il0floatpacket.5(%rip), %ymm0 #103.34
  vpxor %ymm3, %ymm3, %ymm3 #98.25
  vpcmpeqd %ymm4, %ymm4, %ymm4 #99.25
..B1.16: # Preds ..B1.16 ..B1.15
  vpand %ymm1, %ymm5, %ymm11 #103.7
  vpand %ymm1, %ymm7, %ymm10 #101.23
  vpaddq %ymm9, %ymm7, %ymm7 #97.31
  vpaddq %ymm8, %ymm5, %ymm5 #102.39
  vpand %ymm0, %ymm11, %ymm12 #103.7
  vpcmpeqq %ymm10, %ymm2, %ymm15 #101.23
  vpsllvq %ymm12, %ymm6, %ymm13 #103.7
  vpandn %ymm4, %ymm15, %ymm10 #104.7
  vpblendvb %ymm15, %ymm13, %ymm3, %ymm14 #103.7
  vmovdqu %ymm10, linux_bitmap_b(,%rcx,8) #104.7
  vmovdqu %ymm14, linux_bitmap_a(,%rcx,8) #103.7
  addq $4, %rcx #97.3
  cmpq $16384, %rcx #97.3
  jb ..B1.16 # Prob 99% #97.3
..B1.17: # Preds ..B1.16
  movl %edx, %r10d #141.33
..B1.18: # Preds ..B1.37 ..B1.17
  movl %r10d, %r8d #148.43
  movq %r9, %rcx #142.51
  shlq $19, %r8 #148.51
  andq $63, %rcx #142.51
  jmp ..B1.19 # Prob 100% #142.51
..B1.34: # Preds ..B1.33
  lea (%rdi,%r8), %rcx #148.51
  xorq %rcx, %rsi #148.9
  lea 1(%rdi), %rcx #149.24
..B1.19: # Preds ..B1.34 ..B1.18
  cmpq $1048576, %rcx #74.16
  jae ..B1.37 # Prob 28% #74.16
..B1.20: # Preds ..B1.19
  movq %rcx, %rdi #78.19
  shrq $6, %rdi #78.19
  movl %ecx, %ecx #77.27
  shlx %rcx, %rax, %r11 #77.27
  movq linux_bitmap_b(,%rdi,8), %rbx #145.56
  andn linux_bitmap_a(,%rdi,8), %rbx, %rbx #79.28
  andq %r11, %rbx #79.44
  jne ..B1.25 # Prob 1% #81.11
..B1.22: # Preds ..B1.20 ..B1.23
  movq %rdi, %rcx #82.25
  shlq $6, %rcx #82.25
  addq $64, %rcx #82.25
  cmpq $1048576, %rcx #82.42
  jae ..B1.37 # Prob 20% #82.42
..B1.23: # Preds ..B1.22
  incq %rdi #84.5
  movq linux_bitmap_b(,%rdi,8), %rcx #145.56
  andn linux_bitmap_a(,%rdi,8), %rcx, %rbx #85.29
  je ..B1.22 # Prob 82% #81.11
..B1.25: # Preds ..B1.23 ..B1.20
  movl %ebx, %r11d #38.15
  movl $32, %ecx #39.5
  testq %r11, %r11 #39.5
  movq %rbx, %r15 #40.5
  cmovne %edx, %ecx #39.5
  shrq $32, %r15 #40.5
  testq %r11, %r11 #40.5
  cmove %r15, %rbx #40.5
  movzwl %bx, %r11d #42.15
  testq %r11, %r11 #42.28
  jne ..B1.27 # Prob 50% #42.28
..B1.26: # Preds ..B1.25
  shrq $16, %rbx #44.5
  addl $16, %ecx #43.5
..B1.27: # Preds ..B1.26 ..B1.25
  movl %ebx, %r11d #46.26
  testb %r11b, %r11b #46.26
  jne ..B1.29 # Prob 50% #46.26
..B1.28: # Preds ..B1.27
  shrq $8, %rbx #48.5
  addl $8, %ecx #47.5
..B1.29: # Preds ..B1.28 ..B1.27
  testq $15, %rbx #50.15
  jne ..B1.31 # Prob 50% #50.25
..B1.30: # Preds ..B1.29
  shrq $4, %rbx #52.5
  addl $4, %ecx #51.5
..B1.31: # Preds ..B1.30 ..B1.29
  testq $3, %rbx #54.15
  jne ..B1.33 # Prob 50% #54.25
..B1.32: # Preds ..B1.31
  shrq $2, %rbx #56.5
  addl $2, %ecx #55.5
..B1.33: # Preds ..B1.32 ..B1.31
  testq $1, %rbx #58.15
  lea 1(%rcx), %r15d #59.5
  cmove %r15d, %ecx #89.19
  movl $1048576, %ebx #89.19
  shlq $6, %rdi #88.20
  addq %rcx, %rdi #88.36
  cmpq $1048576, %rdi #89.19
  cmova %rbx, %rdi #89.19
  cmpq $1048576, %rdi #147.17
  jb ..B1.34 # Prob 50% #147.17
..B1.37: # Preds ..B1.22 ..B1.33 ..B1.19
  incl %r10d #141.33
  lea (%r9,%r9,8), %rcx #155.52
  lea (%rcx,%r9,4), %rbx #155.52
  movzbl %bl, %r11d #155.60
  lea (,%r9,8), %rdi #156.62
  shlq $9, %r11 #155.69
  subq %r9, %rdi #156.62
  movl %r10d, %r9d #141.33
  movq 40+linux_bitmap_b(%r11), %r8 #156.7
  btcq %rdi, %r8 #156.7
  movq %r8, 40+linux_bitmap_b(%r11) #156.7
  cmpl $1024, %r10d #141.25
  jb ..B1.18 # Prob 99% #141.25
..B1.38: # Preds ..B1.37
  movl $.L_2__STRING.0, %edi #161.3
  xorl %eax, %eax #161.3
  vzeroupper #161.3
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
  xorl %edx, %edx #39.5
  jmp ..B1.15 # Prob 100% #39.5
linux_bitmap_b:
linux_bitmap_a:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
.L_2il0floatpacket.6:
.L_2il0floatpacket.7:
.L_2__STRING.0:
