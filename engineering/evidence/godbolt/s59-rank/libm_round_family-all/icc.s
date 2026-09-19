main:
..B1.1: # Preds ..B1.0
  pushq %rbp #88.1
  movq %rsp, %rbp #88.1
  andq $-128, %rsp #88.1
  pushq %r12 #88.1
  pushq %r13 #88.1
  subq $112, %rsp #88.1
  movl $3, %edi #88.1
  xorl %esi, %esi #88.1
  call __intel_new_feature_proc_init #88.1
..B1.10: # Preds ..B1.1
  stmxcsr (%rsp) #88.1
  xorl %eax, %eax #91.3
  orl $32832, (%rsp) #88.1
  xorl %edx, %edx #91.3
  ldmxcsr (%rsp) #88.1
  movdqu .L_2il0floatpacket.0(%rip), %xmm5 #92.19
  movdqu .L_2il0floatpacket.1(%rip), %xmm4 #92.19
  movdqu .L_2il0floatpacket.2(%rip), %xmm3 #92.5
  movups .L_2il0floatpacket.3(%rip), %xmm2 #92.28
  movups .L_2il0floatpacket.4(%rip), %xmm1 #92.49
  movdqu .L_2il0floatpacket.5(%rip), %xmm0 #92.44
..B1.2: # Preds ..B1.2 ..B1.10
  movdqa %xmm0, %xmm7 #92.44
  movdqa %xmm4, %xmm6 #92.19
  pand %xmm3, %xmm7 #92.44
  paddd %xmm5, %xmm3 #92.5
  punpckhqdq %xmm4, %xmm6 #92.19
  cvtdq2pd %xmm7, %xmm8 #92.44
  cvtdq2pd %xmm4, %xmm10 #92.19
  cvtdq2pd %xmm6, %xmm11 #92.19
  mulpd %xmm1, %xmm8 #92.49
  mulpd %xmm2, %xmm10 #92.28
  mulpd %xmm2, %xmm11 #92.28
  addpd %xmm8, %xmm10 #92.49
  punpckhqdq %xmm7, %xmm7 #92.44
  paddd %xmm5, %xmm4 #92.19
  cvtdq2pd %xmm7, %xmm9 #92.44
  mulpd %xmm1, %xmm9 #92.49
  addpd %xmm9, %xmm11 #92.49
  movups %xmm10, buf(,%rax,8) #92.5
  movups %xmm11, 16+buf(,%rax,8) #92.5
  addq $4, %rax #91.3
  cmpq $4096, %rax #91.3
  jb ..B1.2 # Prob 99% #91.3
..B1.3: # Preds ..B1.2
  xorl %eax, %eax #95.36
  movq %rdx, %r13 #95.36
  pxor %xmm1, %xmm1 #94.16
  movl %eax, %r12d #95.36
..B1.4: # Preds ..B1.5 ..B1.3
  movsd %xmm1, (%rsp) #97.16[spill]
  call round_family_pass #97.16
..B1.11: # Preds ..B1.4
  movsd (%rsp), %xmm1 #[spill]
..B1.5: # Preds ..B1.11
  andq $4095, %r13 #99.29
  incl %r12d #95.36
  addsd %xmm0, %xmm1 #97.7
  movsd buf(,%r13,8), %xmm2 #99.21
  xorps .L_2il0floatpacket.6(%rip), %xmm2 #99.21
  movsd %xmm2, buf(,%r13,8) #99.7
  movl %r12d, %r13d #95.36
  cmpl $20000, %r12d #95.28
  jb ..B1.4 # Prob 99% #95.28
..B1.6: # Preds ..B1.5
  movl $.L_2__STRING.0, %edi #101.3
  movl $1, %eax #101.3
  movaps %xmm1, %xmm0 #101.3
  call printf #101.3
..B1.7: # Preds ..B1.6
  xorl %eax, %eax #102.10
  addq $112, %rsp #102.10
  popq %r13 #102.10
  popq %r12 #102.10
  movq %rbp, %rsp #102.10
  popq %rbp #102.10
  ret #102.10
round_family_pass:
..B2.1: # Preds ..B2.0
  pushq %r12 #64.1
  subq $64, %rsp #64.1
  xorl %eax, %eax #66.14
  pxor %xmm0, %xmm0 #65.14
  movq %rax, %r12 #66.14
  movsd %xmm0, 32(%rsp) #66.14[spill]
..B2.2: # Preds ..B2.9 ..B2.1
  movsd buf(,%r12,8), %xmm0 #68.18
  movsd %xmm0, 56(%rsp) #68.18[spill]
  call trunc #74.18
..B2.8: # Preds ..B2.2
  movsd %xmm0, 40(%rsp) #74.18[spill]
  movsd 56(%rsp), %xmm0 #77.18[spill]
  call __builtin_roundeven #77.18
..B2.7: # Preds ..B2.8
  movsd %xmm0, 8(%rsp) #77.18[spill]
..B2.3: # Preds ..B2.7
  movsd 56(%rsp), %xmm0 #75.18[spill]
  call rint #75.18
..B2.13: # Preds ..B2.3
  movsd %xmm0, 48(%rsp) #75.18[spill]
  movsd 56(%rsp), %xmm0 #76.18[spill]
  call nearbyint #76.18
..B2.12: # Preds ..B2.13
  movsd %xmm0, 16(%rsp) #76.18[spill]
  movsd .L_2il0floatpacket.8(%rip), %xmm2 #78.18
  movsd .L_2il0floatpacket.8(%rip), %xmm3 #78.18
  movsd 56(%rsp), %xmm0 #78.18[spill]
  movsd 40(%rsp), %xmm1 #78.18[spill]
  andps %xmm0, %xmm2 #78.18
  andnps %xmm1, %xmm3 #78.18
  orps %xmm2, %xmm3 #78.18
  movsd %xmm3, 24(%rsp) #78.18[spill]
  call floor #72.18
..B2.11: # Preds ..B2.12
  movsd %xmm0, (%rsp) #72.18[spill]
  movsd 56(%rsp), %xmm0 #73.18[spill]
  call ceil #73.18
..B2.10: # Preds ..B2.11
  movsd .L_2il0floatpacket.7(%rip), %xmm1 #79.18
  movaps %xmm0, %xmm2 #73.18
  movsd (%rsp), %xmm0 #79.18[spill]
  call fma #79.18
..B2.9: # Preds ..B2.10
  movsd 48(%rsp), %xmm1 #80.20[spill]
  movsd 32(%rsp), %xmm2 #81.7[spill]
  addsd 16(%rsp), %xmm1 #80.20[spill]
  addsd 8(%rsp), %xmm1 #80.24[spill]
  addsd 24(%rsp), %xmm1 #80.28[spill]
  addsd %xmm0, %xmm1 #80.32
  movsd %xmm1, out(,%r12,8) #80.7
  incq %r12 #66.26
  subsd 40(%rsp), %xmm1 #81.23[spill]
  addsd %xmm1, %xmm2 #81.7
  movsd %xmm2, 32(%rsp) #81.7[spill]
  cmpq $4096, %r12 #66.23
  jl ..B2.2 # Prob 99% #66.23
..B2.4: # Preds ..B2.9
  movaps %xmm2, %xmm0 #
  addq $64, %rsp #83.10
  popq %r12 #83.10
  ret #83.10
rint:
..B3.1: # Preds ..B3.0
  movaps %xmm0, %xmm2 #31.44
  pxor %xmm1, %xmm1 #33.33
  subsd .L_2il0floatpacket.9(%rip), %xmm2 #31.44
  comisd %xmm1, %xmm0 #33.33
  addsd .L_2il0floatpacket.9(%rip), %xmm2 #31.51
  jae ..L68 # Prob 50% #33.33
  movaps %xmm2, %xmm0 #33.33
..L68: #
  ret #33.33
nearbyint:
..B4.1: # Preds ..B4.0
  movaps %xmm0, %xmm2 #31.44
  pxor %xmm1, %xmm1 #34.38
  subsd .L_2il0floatpacket.9(%rip), %xmm2 #31.44
  comisd %xmm1, %xmm0 #34.38
  addsd .L_2il0floatpacket.9(%rip), %xmm2 #31.51
  jae ..L76 # Prob 50% #34.38
  movaps %xmm2, %xmm0 #34.38
..L76: #
  ret #34.38
roundeven:
..B5.1: # Preds ..B5.0
  movaps %xmm0, %xmm2 #31.44
  pxor %xmm1, %xmm1 #35.38
  subsd .L_2il0floatpacket.9(%rip), %xmm2 #31.44
  comisd %xmm1, %xmm0 #35.38
  addsd .L_2il0floatpacket.9(%rip), %xmm2 #31.51
  jae ..L84 # Prob 50% #35.38
  movaps %xmm2, %xmm0 #35.38
..L84: #
  ret #35.38
floor:
..B6.1: # Preds ..B6.0
  movaps %xmm0, %xmm2 #31.44
  pxor %xmm1, %xmm1 #31.15
  subsd .L_2il0floatpacket.9(%rip), %xmm2 #31.44
  comisd %xmm1, %xmm0 #31.15
  addsd .L_2il0floatpacket.9(%rip), %xmm2 #31.51
  jb ..L92 # Prob 50% #31.15
  movaps %xmm0, %xmm2 #31.15
..L92: #
  comisd %xmm0, %xmm2 #36.58
  movaps %xmm2, %xmm3 #36.66
  subsd .L_2il0floatpacket.10(%rip), %xmm3 #36.66
  ja ..L93 # Prob 50% #36.58
  movaps %xmm2, %xmm3 #36.58
..L93: #
  movaps %xmm3, %xmm0 #36.58
  ret #36.58
ceil:
..B7.1: # Preds ..B7.0
  movaps %xmm0, %xmm2 #31.44
  pxor %xmm1, %xmm1 #31.15
  subsd .L_2il0floatpacket.9(%rip), %xmm2 #31.44
  comisd %xmm1, %xmm0 #31.15
  addsd .L_2il0floatpacket.9(%rip), %xmm2 #31.51
  jb ..L101 # Prob 50% #31.15
  movaps %xmm0, %xmm2 #31.15
..L101: #
  comisd %xmm2, %xmm0 #37.57
  movsd .L_2il0floatpacket.10(%rip), %xmm3 #37.65
  addsd %xmm2, %xmm3 #37.65
  ja ..L102 # Prob 50% #37.57
  movaps %xmm2, %xmm3 #37.57
..L102: #
  movaps %xmm3, %xmm0 #37.57
  ret #37.57
trunc:
..B8.1: # Preds ..B8.0
  movaps %xmm0, %xmm2 #38.25
  movaps %xmm2, %xmm3 #31.44
  pxor %xmm1, %xmm1 #31.15
  subsd .L_2il0floatpacket.9(%rip), %xmm3 #31.44
  comisd %xmm1, %xmm2 #31.15
  addsd .L_2il0floatpacket.9(%rip), %xmm3 #31.51
  jb ..L110 # Prob 50% #31.15
  movaps %xmm2, %xmm3 #31.15
..L110: #
  jb ..B8.3 # Prob 50% #39.12
..B8.2: # Preds ..B8.1
  comisd %xmm2, %xmm3 #39.28
  movaps %xmm3, %xmm0 #39.36
  subsd .L_2il0floatpacket.10(%rip), %xmm0 #39.36
  ja ..L111 # Prob 50% #39.28
  movaps %xmm3, %xmm0 #39.28
..L111: #
  ret #39.28
..B8.3: # Preds ..B8.1
  comisd %xmm3, %xmm2 #40.14
  movsd .L_2il0floatpacket.10(%rip), %xmm0 #40.22
  addsd %xmm3, %xmm0 #40.22
  ja ..L113 # Prob 50% #40.14
  movaps %xmm3, %xmm0 #40.14
..L113: #
  ret #40.14
copysign:
..B9.1: # Preds ..B9.0
  movq $0x8000000000000000, %rax #44.48
  movq %xmm0, %rcx #44.10
  movq %xmm1, %rdx #44.42
  btrq $63, %rcx #44.16
  andq %rax, %rdx #44.48
  orq %rdx, %rcx #44.48
  movq %rcx, -8(%rsp) #44.3
  movsd -8(%rsp), %xmm0 #45.10
  ret #45.10
fma:
..B10.1: # Preds ..B10.0
  mulsd %xmm1, %xmm0 #50.56
  addsd %xmm2, %xmm0 #50.60
  ret #50.60
buf:
out:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
.L_2il0floatpacket.6:
.L_2il0floatpacket.7:
.L_2il0floatpacket.8:
.L_2il0floatpacket.9:
.L_2il0floatpacket.10:
.L_2__STRING.0:
