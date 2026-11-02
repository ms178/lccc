main:
..B1.1: # Preds ..B1.0
  pushq %rbp #57.33
  movq %rsp, %rbp #57.33
  andq $-128, %rsp #57.33
  pushq %r12 #57.33
  pushq %r13 #57.33
  subq $496, %rsp #57.33
  movq %rsi, %r13 #57.33
  movl %edi, %r12d #57.33
  movl $3, %edi #57.33
  xorl %esi, %esi #57.33
  call __intel_new_feature_proc_init #57.33
..B1.18: # Preds ..B1.1
  stmxcsr (%rsp) #57.33
  orl $32832, (%rsp) #57.33
  ldmxcsr (%rsp) #57.33
  cmpl $1, %r12d #58.24
  jg ..B1.3 # Prob 22% #58.24
..B1.2: # Preds ..B1.18
  movl $20000000, %eax #58.58
  jmp ..B1.5 # Prob 100% #58.58
..B1.3: # Preds ..B1.18
  xorl %esi, %esi #58.33
  xorl %edx, %edx #58.33
  movq 8(%r13), %rdi #58.33
  call strtoul #58.33
..B1.5: # Preds ..B1.3 ..B1.2
  movq 400+ks.378.0.0.19(%rip), %rdx #59.30
  movq %rdx, 400(%rsp) #59.30
  movl $400, %edx #59.30
..B1.17: # Preds ..B1.17 ..B1.5
  movups -16+ks.378.0.0.19(%rdx), %xmm0 #59.30
  movups -32+ks.378.0.0.19(%rdx), %xmm1 #59.30
  movups -48+ks.378.0.0.19(%rdx), %xmm2 #59.30
  movups -64+ks.378.0.0.19(%rdx), %xmm3 #59.30
  movups -80+ks.378.0.0.19(%rdx), %xmm4 #59.30
  movups %xmm0, -16(%rsp,%rdx) #59.30
  movups %xmm1, -32(%rsp,%rdx) #59.30
  movups %xmm2, -48(%rsp,%rdx) #59.30
  movups %xmm3, -64(%rsp,%rdx) #59.30
  movups %xmm4, -80(%rsp,%rdx) #59.30
  subq $80, %rdx #59.30
  jne ..B1.17 # Prob 80% #59.30
..B1.6: # Preds ..B1.17
  movl %eax, %ecx #71.48
  xorl %r13d, %r13d #69.15
  shrl $4, %ecx #71.48
  xorl %r12d, %r12d #70.21
  movl %ecx, 472(%rsp) #71.29[spill]
  movl %eax, 464(%rsp) #71.29[spill]
  movq %r14, 424(%rsp) #71.29[spill]
  movq %r15, 416(%rsp) #71.29[spill]
  movq %rbx, 408(%rsp) #71.29[spill]
..B1.7: # Preds ..B1.12 ..B1.6
  movl $k_digits, %eax #71.29
  lea (%r12,%r12,2), %r15 #71.17
  cmpq 8(%rsp,%r15,8), %rax #71.29
  movl 464(%rsp), %r14d #71.29[spill]
  lea 432(%rsp), %rsi #27.5
  cmove 40(%rsi), %r14d #71.29[spill]
  movl $1, %edi #71.29
  call clock_gettime #27.5
..B1.8: # Preds ..B1.7
  pxor %xmm1, %xmm1 #28.12
  pxor %xmm0, %xmm0 #28.30
  cvtsi2sdq 432(%rsp), %xmm1 #28.12
  cvtsi2sdq 440(%rsp), %xmm0 #28.30
  mulsd .L_2il0floatpacket.0(%rip), %xmm1 #28.24
  movl 16(%rsp,%r15,8), %edi #73.17
  movl %r14d, %esi #73.17
  addsd %xmm0, %xmm1 #28.30
  movsd %xmm1, 480(%rsp) #28.30[spill]
  call *8(%rsp,%r15,8) #73.17
..B1.20: # Preds ..B1.8
  movl %eax, %ebx #73.17
..B1.9: # Preds ..B1.20
  movl $1, %edi #27.5
  lea 448(%rsp), %rsi #27.5
  call clock_gettime #27.5
..B1.10: # Preds ..B1.9
  pxor %xmm0, %xmm0 #80.9
  pxor %xmm1, %xmm1 #80.9
  cvtsi2sdq 448(%rsp), %xmm0 #80.9
  cvtsi2sdq 456(%rsp), %xmm1 #80.9
  mulsd .L_2il0floatpacket.0(%rip), %xmm0 #80.9
  pxor %xmm2, %xmm2 #80.9
  cvtsi2sdq %r14, %xmm2 #80.9
  addsd %xmm1, %xmm0 #80.9
  movl %r13d, %ecx #75.25
  movl $.L_2__STRING.17, %esi #80.9
  shll $5, %ecx #75.25
  movl $1, %eax #80.9
  subl %r13d, %ecx #75.25
  subsd 480(%rsp), %xmm0 #80.9[spill]
  divsd %xmm2, %xmm0 #80.9
  movq stderr(%rip), %rdi #80.9
  lea (%rbx,%rcx), %r13d #75.31
  movq (%rsp,%r15,8), %rdx #80.9
  call fprintf #80.9
..B1.11: # Preds ..B1.10
  movl $.L_2__STRING.18, %edi #81.9
  movl %ebx, %edx #81.9
  xorl %eax, %eax #81.9
  movq (%rsp,%r15,8), %rsi #81.9
  call printf #81.9
..B1.12: # Preds ..B1.11
  incl %r12d #70.56
  cmpq $17, %r12 #70.30
  jb ..B1.7 # Prob 82% #70.30
..B1.13: # Preds ..B1.12
  movl $.L_2__STRING.19, %edi #83.5
  movl %r13d, %esi #83.5
  xorl %eax, %eax #83.5
  movq 424(%rsp), %r14 #[spill]
  movq 416(%rsp), %r15 #[spill]
  movq 408(%rsp), %rbx #[spill]
  call printf #83.5
..B1.14: # Preds ..B1.13
  xorl %eax, %eax #84.12
  addq $496, %rsp #84.12
  popq %r13 #84.12
  popq %r12 #84.12
  movq %rbp, %rsp #84.12
  popq %rbp #84.12
  ret #84.12
ks.378.0.0.19:
k_urem7:
..B2.1: # Preds ..B2.0
  xorl %ecx, %ecx #32.43
  testl %esi, %esi #32.52
  jbe ..B2.5 # Prob 10% #32.52
..B2.3: # Preds ..B2.1 ..B2.3
  movl $613566757, %eax #32.69
  movl %edi, %r8d #32.69
  mull %edi #32.69
  subl %edx, %r8d #32.69
  shrl $1, %r8d #32.69
  addl %edx, %r8d #32.69
  shrl $2, %r8d #32.69
  lea (,%r8,8), %r9d #32.69
  subl %r8d, %r9d #32.69
  subl %r9d, %edi #32.69
  addl %ecx, %edi #32.32
  incl %ecx #32.55
  cmpl %esi, %ecx #32.52
  jb ..B2.3 # Prob 82% #32.52
..B2.5: # Preds ..B2.3 ..B2.1
  movl %edi, %eax #32.85
  ret #32.85
k_urem10:
..B3.1: # Preds ..B3.0
  xorl %ecx, %ecx #33.44
  testl %esi, %esi #33.53
  jbe ..B3.5 # Prob 10% #33.53
..B3.3: # Preds ..B3.1 ..B3.3
  movl $-858993459, %eax #33.70
  mull %edi #33.70
  shrl $3, %edx #33.70
  lea (%rdx,%rdx,4), %r8d #33.70
  addl %r8d, %r8d #33.70
  negl %r8d #33.70
  addl %edi, %r8d #33.70
  xorl %ecx, %edi #33.82
  incl %ecx #33.56
  addl %r8d, %edi #33.82
  cmpl %esi, %ecx #33.53
  jb ..B3.3 # Prob 82% #33.53
..B3.5: # Preds ..B3.3 ..B3.1
  movl %edi, %eax #33.93
  ret #33.93
k_udiv7:
..B4.1: # Preds ..B4.0
  movl %esi, %r8d #34.30
  xorl %esi, %esi #34.43
  xorl %ecx, %ecx #34.43
  testl %r8d, %r8d #34.52
  jbe ..B4.5 # Prob 10% #34.52
..B4.3: # Preds ..B4.1 ..B4.3
  movl $613566757, %eax #34.69
  incl %esi #34.55
  mull %edi #34.69
  subl %edx, %edi #34.69
  shrl $1, %edi #34.69
  addl %edx, %edi #34.69
  shrl $2, %edi #34.69
  addl %ecx, %edi #34.80
  addl $3, %ecx #34.55
  cmpl %r8d, %esi #34.52
  jb ..B4.3 # Prob 82% #34.52
..B4.5: # Preds ..B4.3 ..B4.1
  movl %edi, %eax #34.92
  ret #34.92
k_udr10:
..B5.1: # Preds ..B5.0
  xorl %ecx, %ecx #35.43
  testl %esi, %esi #35.52
  jbe ..B5.5 # Prob 10% #35.52
..B5.3: # Preds ..B5.1 ..B5.3
  movl $-858993459, %eax #35.74
  mull %edi #35.74
  shrl $3, %edx #35.74
  lea (%rdx,%rdx,4), %r8d #35.74
  addl %r8d, %r8d #35.74
  subl %r8d, %edi #35.74
  lea (%rdi,%rdi,2), %edi #35.104
  addl %edi, %edx #35.74
  lea (%rcx,%rdx), %edi #35.32
  incl %ecx #35.55
  cmpl %esi, %ecx #35.52
  jb ..B5.3 # Prob 82% #35.52
..B5.5: # Preds ..B5.3 ..B5.1
  movl %edi, %eax #35.121
  ret #35.121
k_sdr7:
..B6.1: # Preds ..B6.0
  movl %esi, %r8d #36.29
  xorl %esi, %esi #36.42
  xorl %ecx, %ecx #36.42
  testl %r8d, %r8d #36.51
  jbe ..B6.5 # Prob 10% #36.51
..B6.3: # Preds ..B6.1 ..B6.3
  movl $-1840700269, %eax #36.73
  movl %edi, %r9d #36.73
  imull %edi #36.73
  sarl $31, %r9d #36.73
  incl %esi #36.54
  addl %edi, %edx #36.73
  sarl $2, %edx #36.73
  subl %r9d, %edx #36.73
  lea (,%rdx,8), %r10d #36.73
  subl %edx, %r10d #36.73
  subl %r10d, %edi #36.73
  lea (%rdi,%rdi,4), %edi #36.99
  addl %edi, %edx #36.73
  lea (%rcx,%rdx), %edi #36.73
  decl %ecx #36.54
  cmpl %r8d, %esi #36.51
  jb ..B6.3 # Prob 82% #36.51
..B6.5: # Preds ..B6.3 ..B6.1
  movl %edi, %eax #36.125
  ret #36.125
k_srem7:
..B7.1: # Preds ..B7.0
  xorl %ecx, %ecx #37.43
  testl %esi, %esi #37.52
  jbe ..B7.5 # Prob 10% #37.52
..B7.3: # Preds ..B7.1 ..B7.3
  movl $-1840700269, %eax #37.69
  movl %edi, %r8d #37.69
  imull %edi #37.69
  sarl $31, %r8d #37.69
  addl %edi, %edx #37.69
  sarl $2, %edx #37.69
  subl %r8d, %edx #37.69
  movzwl %cx, %r10d #37.84
  incl %ecx #37.55
  lea (,%rdx,8), %r9d #37.69
  subl %edx, %r9d #37.69
  subl %r9d, %edi #37.69
  lea -30000(%rdi,%r10), %edi #37.94
  cmpl %esi, %ecx #37.52
  jb ..B7.3 # Prob 82% #37.52
..B7.5: # Preds ..B7.3 ..B7.1
  movl %edi, %eax #37.113
  ret #37.113
k_srem16:
..B8.1: # Preds ..B8.0
  xorl %edx, %edx #38.44
  testl %esi, %esi #38.53
  jbe ..B8.5 # Prob 10% #38.53
..B8.3: # Preds ..B8.1 ..B8.8
  andl $-2147483633, %edi #38.70
  jge ..B8.8 # Prob 50% #38.70
..B8.9: # Preds ..B8.3
  subl $1, %edi #38.70
  orl $-16, %edi #38.70
  incl %edi #38.70
..B8.8: # Preds ..B8.3 ..B8.9
  movzbl %dl, %ecx #38.90
  incl %edx #38.56
  lea (%rdi,%rdi,2), %eax #38.76
  lea -100(%rax,%rcx), %edi #38.98
  cmpl %esi, %edx #38.53
  jb ..B8.3 # Prob 82% #38.53
..B8.5: # Preds ..B8.8 ..B8.1
  movl %edi, %eax #38.115
  ret #38.115
k_sdr16:
..B9.1: # Preds ..B9.0
  xorl %edx, %edx #39.43
  testl %esi, %esi #39.52
  jbe ..B9.5 # Prob 10% #39.52
..B9.3: # Preds ..B9.1 ..B9.8
  movl %edi, %ecx #39.74
  sarl $3, %ecx #39.74
  shrl $28, %ecx #39.74
  addl %edi, %ecx #39.74
  sarl $4, %ecx #39.74
  andl $-2147483633, %edi #39.86
  jge ..B9.8 # Prob 50% #39.86
..B9.9: # Preds ..B9.3
  subl $1, %edi #39.86
  orl $-16, %edi #39.86
  incl %edi #39.86
..B9.8: # Preds ..B9.3 ..B9.9
  shll $3, %edi #39.104
  xorl %edi, %ecx #39.104
  movl %edx, %edi #39.114
  incl %edx #39.55
  xorl %ecx, %edi #39.114
  cmpl %esi, %edx #39.52
  jb ..B9.3 # Prob 82% #39.52
..B9.5: # Preds ..B9.8 ..B9.1
  movl %edi, %eax #39.131
  ret #39.131
k_mul7:
..B10.1: # Preds ..B10.0
  xorl %edx, %edx #40.42
  testl %esi, %esi #40.51
  jbe ..B10.5 # Prob 10% #40.51
..B10.3: # Preds ..B10.1 ..B10.3
  lea (,%rdi,8), %ecx #40.68
  subl %edi, %ecx #40.68
  movl %edx, %edi #40.74
  incl %edx #40.54
  xorl %ecx, %edi #40.74
  cmpl %esi, %edx #40.51
  jb ..B10.3 # Prob 82% #40.51
..B10.5: # Preds ..B10.3 ..B10.1
  movl %edi, %eax #40.84
  ret #40.84
k_mul11:
..B11.1: # Preds ..B11.0
  xorl %edx, %edx #41.43
  testl %esi, %esi #41.52
  jbe ..B11.5 # Prob 10% #41.52
..B11.3: # Preds ..B11.1 ..B11.3
  lea (%rdi,%rdi,8), %ecx #41.69
  lea (%rcx,%rdi,2), %edi #41.69
  xorl %edx, %edi #41.76
  incl %edx #41.55
  cmpl %esi, %edx #41.52
  jb ..B11.3 # Prob 82% #41.52
..B11.5: # Preds ..B11.3 ..B11.1
  movl %edi, %eax #41.86
  ret #41.86
k_mul17:
..B12.1: # Preds ..B12.0
  xorl %edx, %edx #42.43
  testl %esi, %esi #42.52
  jbe ..B12.5 # Prob 10% #42.52
..B12.3: # Preds ..B12.1 ..B12.3
  movl %edi, %ecx #42.69
  shll $4, %ecx #42.69
  addl %edi, %ecx #42.69
  movl %edx, %edi #42.76
  incl %edx #42.55
  xorl %ecx, %edi #42.76
  cmpl %esi, %edx #42.52
  jb ..B12.3 # Prob 82% #42.52
..B12.5: # Preds ..B12.3 ..B12.1
  movl %edi, %eax #42.86
  ret #42.86
k_mul24:
..B13.1: # Preds ..B13.0
  xorl %edx, %edx #43.43
  testl %esi, %esi #43.52
  jbe ..B13.5 # Prob 10% #43.52
..B13.3: # Preds ..B13.1 ..B13.3
  lea (%rdi,%rdi,2), %edi #43.69
  shll $3, %edi #43.69
  xorl %edx, %edi #43.76
  incl %edx #43.55
  cmpl %esi, %edx #43.52
  jb ..B13.3 # Prob 82% #43.52
..B13.5: # Preds ..B13.3 ..B13.1
  movl %edi, %eax #43.86
  ret #43.86
k_mul45:
..B14.1: # Preds ..B14.0
  xorl %edx, %edx #44.43
  testl %esi, %esi #44.52
  jbe ..B14.5 # Prob 10% #44.52
..B14.3: # Preds ..B14.1 ..B14.3
  imull $45, %edi, %edi #44.69
  xorl %edx, %edi #44.76
  incl %edx #44.55
  cmpl %esi, %edx #44.52
  jb ..B14.3 # Prob 82% #44.52
..B14.5: # Preds ..B14.3 ..B14.1
  movl %edi, %eax #44.86
  ret #44.86
k_mulm3:
..B15.1: # Preds ..B15.0
  xorl %edx, %edx #45.43
  testl %esi, %esi #45.52
  jbe ..B15.5 # Prob 10% #45.52
..B15.3: # Preds ..B15.1 ..B15.3
  lea (%rdi,%rdi,2), %edi #45.69
  negl %edi #45.69
  xorl %edx, %edi #45.80
  incl %edx #45.55
  cmpl %esi, %edx #45.52
  jb ..B15.3 # Prob 82% #45.52
..B15.5: # Preds ..B15.3 ..B15.1
  movl %edi, %eax #45.90
  ret #45.90
k_mul1000:
..B16.1: # Preds ..B16.0
  xorl %edx, %edx #46.45
  testl %esi, %esi #46.54
  jbe ..B16.5 # Prob 10% #46.54
..B16.3: # Preds ..B16.1 ..B16.3
  imull $1000, %edi, %edi #46.71
  xorl %edx, %edi #46.80
  incl %edx #46.57
  cmpl %esi, %edx #46.54
  jb ..B16.3 # Prob 82% #46.54
..B16.5: # Preds ..B16.3 ..B16.1
  movl %edi, %eax #46.90
  ret #46.90
k_fnv:
..B17.1: # Preds ..B17.0
  xorl %edx, %edx #47.41
  testl %esi, %esi #47.50
  jbe ..B17.5 # Prob 10% #47.50
..B17.3: # Preds ..B17.1 ..B17.3
  movzbl %dl, %ecx #47.69
  incl %edx #47.53
  xorl %ecx, %edi #47.60
  imull $16777619, %edi, %edi #47.75
  cmpl %esi, %edx #47.50
  jb ..B17.3 # Prob 82% #47.50
..B17.5: # Preds ..B17.3 ..B17.1
  movl %edi, %eax #47.100
  ret #47.100
k_digits:
..B18.1: # Preds ..B18.0
  movl %edi, %r8d #48.31
  xorl %edi, %edi #49.13
  xorl %ecx, %ecx #50.16
  testl %esi, %esi #50.25
  jbe ..B18.9 # Prob 10% #50.25
..B18.3: # Preds ..B18.1 ..B18.7
  movl %r8d, %r9d #50.47
  addl %ecx, %r9d #50.47
  je ..B18.7 # Prob 10% #50.57
..B18.5: # Preds ..B18.3 ..B18.5
  movl $-858993459, %eax #50.73
  mull %r9d #50.73
  shrl $3, %edx #50.73
  lea (%rdx,%rdx,4), %r10d #50.73
  addl %r10d, %r10d #50.73
  subl %r10d, %r9d #50.73
  addl %r9d, %edi #50.62
  movl %edx, %r9d #50.78
  testl %r9d, %r9d #50.57
  jne ..B18.5 # Prob 82% #50.57
..B18.7: # Preds ..B18.5 ..B18.3
  incl %ecx #50.28
  cmpl %esi, %ecx #50.25
  jb ..B18.3 # Prob 82% #50.25
..B18.9: # Preds ..B18.7 ..B18.1
  movl %edi, %eax #51.12
  ret #51.12
.L_2il0floatpacket.0:
.L_2__STRING.0:
.L_2__STRING.1:
.L_2__STRING.2:
.L_2__STRING.3:
.L_2__STRING.4:
.L_2__STRING.5:
.L_2__STRING.6:
.L_2__STRING.7:
.L_2__STRING.8:
.L_2__STRING.9:
.L_2__STRING.10:
.L_2__STRING.11:
.L_2__STRING.12:
.L_2__STRING.13:
.L_2__STRING.14:
.L_2__STRING.15:
.L_2__STRING.16:
.L_2__STRING.17:
.L_2__STRING.18:
.L_2__STRING.19:
