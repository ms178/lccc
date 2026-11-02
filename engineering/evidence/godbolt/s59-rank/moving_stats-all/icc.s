main:
..B1.1: # Preds ..B1.0
  pushq %rbp #23.16
  movq %rsp, %rbp #23.16
  andq $-128, %rsp #23.16
  subq $128, %rsp #23.16
  movl $3, %edi #23.16
  xorl %esi, %esi #23.16
  call __intel_new_feature_proc_init #23.16
..B1.13: # Preds ..B1.1
  stmxcsr 8(%rsp) #23.16
  xorl %eax, %eax #26.5
  orl $32832, 8(%rsp) #23.16
  ldmxcsr 8(%rsp) #23.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm12 #27.36
  movdqu .L_2il0floatpacket.1(%rip), %xmm11 #27.36
  movdqu .L_2il0floatpacket.2(%rip), %xmm10 #27.36
  movdqu .L_2il0floatpacket.3(%rip), %xmm9 #27.42
  movdqu .L_2il0floatpacket.4(%rip), %xmm8 #27.50
  movq %r12, (%rsp) #27.50[spill]
  movq %rax, %r12 #27.50
..B1.2: # Preds ..B1.14 ..B1.13
  movdqa %xmm11, %xmm0 #27.42
  movdqa %xmm9, %xmm1 #27.42
  call *__svml_irem4@GOTPCREL(%rip) #27.42
..B1.15: # Preds ..B1.2
  movdqa %xmm0, %xmm13 #27.42
  movdqa %xmm10, %xmm0 #27.42
  pslld $16, %xmm13 #27.42
  movdqa %xmm9, %xmm1 #27.42
  psrad $16, %xmm13 #27.42
  call *__svml_irem4@GOTPCREL(%rip) #27.42
..B1.14: # Preds ..B1.15
  pslld $16, %xmm0 #27.42
  paddd %xmm12, %xmm11 #27.36
  psrad $16, %xmm0 #27.42
  paddd %xmm12, %xmm10 #27.36
  packssdw %xmm0, %xmm13 #27.42
  paddw %xmm8, %xmm13 #27.50
  movdqu %xmm13, a.132.0.2(,%r12,2) #27.9
  addq $8, %r12 #26.5
  cmpq $1024, %r12 #26.5
  jb ..B1.2 # Prob 99% #26.5
..B1.3: # Preds ..B1.14
  movq (%rsp), %r12 #[spill]
  xorl %eax, %eax #8.5
..B1.4: # Preds ..B1.6 ..B1.3
  movswl a.132.0.2(,%rax,2), %edx #28.11
  xorl %ecx, %ecx #11.9
  pxor %xmm2, %xmm2 #9.15
  movd %edx, %xmm1 #10.18
  lea (%rax,%rax), %rdx #28.11
  punpcklwd %xmm1, %xmm1 #10.18
  punpckldq %xmm1, %xmm1 #10.18
  punpcklqdq %xmm1, %xmm1 #10.18
  movdqa %xmm1, %xmm0 #10.29
..B1.5: # Preds ..B1.5 ..B1.4
  movq a.132.0.2(%rdx,%rcx,2), %xmm4 #28.11
  addq $4, %rcx #11.9
  movdqa %xmm4, %xmm3 #13.18
  pminsw %xmm4, %xmm1 #14.13
  punpcklwd %xmm4, %xmm3 #13.18
  pmaxsw %xmm4, %xmm0 #15.13
  psrad $16, %xmm3 #13.18
  paddd %xmm3, %xmm2 #13.13
  cmpq $16, %rcx #11.9
  jb ..B1.5 # Prob 93% #11.9
..B1.6: # Preds ..B1.5
  movdqa %xmm0, %xmm3 #10.29
  psrldq $4, %xmm3 #10.29
  pmaxsw %xmm3, %xmm0 #10.29
  movdqa %xmm0, %xmm4 #10.29
  psrldq $2, %xmm4 #10.29
  pmaxsw %xmm4, %xmm0 #10.29
  movd %xmm0, %ecx #10.29
  movdqa %xmm1, %xmm0 #10.18
  psrldq $4, %xmm0 #10.18
  pminsw %xmm0, %xmm1 #10.18
  movdqa %xmm1, %xmm5 #10.18
  psrldq $2, %xmm5 #10.18
  pminsw %xmm5, %xmm1 #10.18
  movd %xmm1, %edx #10.18
  movdqa %xmm2, %xmm1 #9.15
  psrldq $8, %xmm1 #9.15
  paddd %xmm1, %xmm2 #9.15
  movdqa %xmm2, %xmm6 #9.15
  psrlq $32, %xmm6 #9.15
  paddd %xmm6, %xmm2 #9.15
  movd %xmm2, sums.132.0.2(,%rax,4) #28.14
  movw %dx, mns.132.0.2(,%rax,2) #28.20
  movw %cx, mxs.132.0.2(,%rax,2) #28.25
  incq %rax #8.5
  cmpq $1009, %rax #8.5
  jb ..B1.4 # Prob 81% #8.5
..B1.7: # Preds ..B1.6
  xorl %edx, %edx #29.26
  xorl %eax, %eax #30.5
..B1.8: # Preds ..B1.8 ..B1.7
  movq %rdx, %rcx #31.17
  shlq $5, %rcx #31.17
  subq %rdx, %rcx #31.17
  movl sums.132.0.2(,%rax,8), %edx #31.32
  addq %rdx, %rcx #31.32
  movq %rcx, %rdi #32.17
  shlq $5, %rdi #32.17
  movswl mns.132.0.2(,%rax,4), %esi #32.33
  subq %rcx, %rdi #32.17
  addl $1000, %esi #32.42
  addq %rsi, %rdi #32.42
  movq %rdi, %r9 #33.17
  shlq $5, %r9 #33.17
  movswl mxs.132.0.2(,%rax,4), %r8d #33.33
  subq %rdi, %r9 #33.17
  addl $1000, %r8d #33.42
  addq %r8, %r9 #33.42
  movq %r9, %r11 #31.17
  shlq $5, %r11 #31.17
  subq %r9, %r11 #31.17
  movl 4+sums.132.0.2(,%rax,8), %r10d #31.32
  addq %r10, %r11 #31.32
  movq %r11, %rcx #32.17
  shlq $5, %rcx #32.17
  movswl 2+mns.132.0.2(,%rax,4), %edx #32.33
  subq %r11, %rcx #32.17
  addl $1000, %edx #32.42
  addq %rdx, %rcx #32.42
  movq %rcx, %rdx #33.17
  shlq $5, %rdx #33.17
  movswl 2+mxs.132.0.2(,%rax,4), %esi #33.33
  subq %rcx, %rdx #33.17
  addl $1000, %esi #33.42
  incq %rax #30.5
  addq %rsi, %rdx #33.42
  cmpq $504, %rax #30.5
  jb ..B1.8 # Prob 64% #30.5
..B1.9: # Preds ..B1.8
  movl 4032+sums.132.0.2(%rip), %ecx #35.5
  movl $.L_2__STRING.0, %edi #35.5
  imulq $29791, %rdx, %rsi #35.5
  imulq $961, %rcx, %r8 #35.5
  movswl 2016+mns.132.0.2(%rip), %edx #35.5
  addl $1000, %edx #35.5
  movq %rdx, %r9 #35.5
  shlq $5, %r9 #35.5
  movswl 2016+mxs.132.0.2(%rip), %eax #35.5
  subq %rdx, %r9 #35.5
  addl $1000, %eax #35.5
  addq %r8, %r9 #35.5
  addq %rax, %rsi #35.5
  addq %r9, %rsi #35.5
  xorl %eax, %eax #35.5
  call printf #35.5
..B1.10: # Preds ..B1.9
  xorl %eax, %eax #36.12
  movq %rbp, %rsp #36.12
  popq %rbp #36.12
  ret #36.12
a.132.0.2:
sums.132.0.2:
mns.132.0.2:
mxs.132.0.2:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2__STRING.0:
