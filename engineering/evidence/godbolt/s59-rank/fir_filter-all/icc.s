main:
..B1.1: # Preds ..B1.0
  pushq %rbp #12.16
  movq %rsp, %rbp #12.16
  andq $-128, %rsp #12.16
  subq $256, %rsp #12.16
  movl $3, %edi #12.16
  xorl %esi, %esi #12.16
  call __intel_new_feature_proc_init #12.16
..B1.11: # Preds ..B1.1
  stmxcsr (%rsp) #12.16
  xorl %eax, %eax #15.5
  orl $32832, (%rsp) #12.16
  ldmxcsr (%rsp) #12.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm3 #15.61
  movdqu .L_2il0floatpacket.1(%rip), %xmm2 #15.61
  movdqu .L_2il0floatpacket.2(%rip), %xmm1 #15.67
  movdqu .L_2il0floatpacket.3(%rip), %xmm0 #15.75
..B1.2: # Preds ..B1.2 ..B1.11
  movdqa %xmm1, %xmm4 #15.67
  pand %xmm2, %xmm4 #15.67
  paddw %xmm3, %xmm2 #15.61
  paddw %xmm0, %xmm4 #15.75
  movdqu %xmm4, a.128.0.2(,%rax,2) #15.35
  addq $8, %rax #15.5
  cmpq $264, %rax #15.5
  jb ..B1.2 # Prob 99% #15.5
..B1.3: # Preds ..B1.2
  movdqu .L_2il0floatpacket.4(%rip), %xmm0 #16.33
  xorl %eax, %eax #5.5
  pmullw %xmm0, %xmm0 #16.52
  psubw .L_2il0floatpacket.5(%rip), %xmm0 #16.52
  paddw .L_2il0floatpacket.6(%rip), %xmm0 #16.64
  movdqu %xmm0, c.128.0.2(%rip) #16.33
  movswl c.128.0.2(%rip), %edx #17.12
  movswl 2+c.128.0.2(%rip), %ecx #17.12
  movswl 4+c.128.0.2(%rip), %esi #17.12
  movswl 6+c.128.0.2(%rip), %edi #17.12
  movd %edx, %xmm8 #7.58
  movswl 8+c.128.0.2(%rip), %r8d #17.12
  movd %ecx, %xmm7 #7.58
  movswl 10+c.128.0.2(%rip), %r9d #17.12
  movd %esi, %xmm6 #7.58
  movswl 12+c.128.0.2(%rip), %r10d #17.12
  movd %edi, %xmm5 #7.58
  movswl 14+c.128.0.2(%rip), %r11d #17.12
  movd %r8d, %xmm4 #7.58
  movd %r9d, %xmm3 #7.58
  movd %r10d, %xmm2 #7.58
  movd %r11d, %xmm1 #7.58
  punpcklwd %xmm8, %xmm8 #7.58
  punpcklwd %xmm7, %xmm7 #7.58
  punpcklwd %xmm6, %xmm6 #7.58
  punpcklwd %xmm5, %xmm5 #7.58
  punpcklwd %xmm4, %xmm4 #7.58
  punpcklwd %xmm3, %xmm3 #7.58
  punpcklwd %xmm2, %xmm2 #7.58
  punpcklwd %xmm1, %xmm1 #7.58
  punpckldq %xmm8, %xmm8 #7.58
  punpckldq %xmm7, %xmm7 #7.58
  punpckldq %xmm6, %xmm6 #7.58
  punpckldq %xmm5, %xmm5 #7.58
  punpckldq %xmm4, %xmm4 #7.58
  punpckldq %xmm3, %xmm3 #7.58
  punpckldq %xmm2, %xmm2 #7.58
  punpckldq %xmm1, %xmm1 #7.58
  punpcklqdq %xmm8, %xmm8 #7.58
  punpcklqdq %xmm7, %xmm7 #7.58
  punpcklqdq %xmm6, %xmm6 #7.58
  punpcklqdq %xmm5, %xmm5 #7.58
  punpcklqdq %xmm4, %xmm4 #7.58
  punpcklqdq %xmm3, %xmm3 #7.58
  punpcklqdq %xmm2, %xmm2 #7.58
  punpcklqdq %xmm1, %xmm1 #7.58
  movdqu %xmm1, 112(%rsp) #6.15[spill]
  movdqu %xmm2, 64(%rsp) #6.15[spill]
  movdqu %xmm3, 96(%rsp) #6.15[spill]
  movdqu %xmm4, 48(%rsp) #6.15[spill]
  movdqu %xmm5, 32(%rsp) #6.15[spill]
  movdqu %xmm6, 80(%rsp) #6.15[spill]
  movdqu %xmm7, 16(%rsp) #6.15[spill]
  movdqu %xmm8, (%rsp) #6.15[spill]
..B1.4: # Preds ..B1.4 ..B1.3
  movdqu a.128.0.2(,%rax,2), %xmm9 #17.9
  movdqu 2+a.128.0.2(,%rax,2), %xmm10 #17.9
  movdqu 4+a.128.0.2(,%rax,2), %xmm11 #17.9
  movdqu 6+a.128.0.2(,%rax,2), %xmm12 #17.9
  movdqu 8+a.128.0.2(,%rax,2), %xmm13 #17.9
  movdqu 10+a.128.0.2(,%rax,2), %xmm0 #17.9
  movdqu 12+a.128.0.2(,%rax,2), %xmm1 #17.9
  movdqu 14+a.128.0.2(,%rax,2), %xmm3 #17.9
  movdqu (%rsp), %xmm15 #7.58[spill]
  movdqa %xmm15, %xmm8 #7.58
  pmullw %xmm9, %xmm8 #7.58
  pmulhw %xmm15, %xmm9 #7.58
  movdqu 16(%rsp), %xmm14 #7.58[spill]
  movdqa %xmm8, %xmm15 #7.58
  punpcklwd %xmm9, %xmm15 #7.58
  punpckhwd %xmm9, %xmm8 #7.58
  movdqa %xmm14, %xmm9 #7.58
  pmullw %xmm10, %xmm9 #7.58
  pmulhw %xmm14, %xmm10 #7.58
  movdqu 80(%rsp), %xmm4 #7.58[spill]
  movdqa %xmm9, %xmm14 #7.58
  punpcklwd %xmm10, %xmm14 #7.58
  punpckhwd %xmm10, %xmm9 #7.58
  movdqa %xmm4, %xmm10 #7.58
  pmullw %xmm11, %xmm10 #7.58
  pmulhw %xmm4, %xmm11 #7.58
  movdqu 32(%rsp), %xmm5 #7.58[spill]
  movdqa %xmm10, %xmm4 #7.58
  punpcklwd %xmm11, %xmm4 #7.58
  punpckhwd %xmm11, %xmm10 #7.58
  movdqa %xmm5, %xmm11 #7.58
  pmullw %xmm12, %xmm11 #7.58
  pmulhw %xmm5, %xmm12 #7.58
  movdqu 48(%rsp), %xmm6 #7.58[spill]
  movdqa %xmm11, %xmm5 #7.58
  punpcklwd %xmm12, %xmm5 #7.58
  punpckhwd %xmm12, %xmm11 #7.58
  movdqa %xmm6, %xmm12 #7.58
  pmullw %xmm13, %xmm12 #7.58
  pmulhw %xmm6, %xmm13 #7.58
  movdqu 96(%rsp), %xmm7 #7.58[spill]
  movdqa %xmm12, %xmm6 #7.58
  punpcklwd %xmm13, %xmm6 #7.58
  punpckhwd %xmm13, %xmm12 #7.58
  movdqa %xmm7, %xmm13 #7.58
  pmullw %xmm0, %xmm13 #7.58
  pmulhw %xmm7, %xmm0 #7.58
  movdqu 64(%rsp), %xmm2 #7.58[spill]
  movdqa %xmm13, %xmm7 #7.58
  punpcklwd %xmm0, %xmm7 #7.58
  punpckhwd %xmm0, %xmm13 #7.58
  movdqa %xmm2, %xmm0 #7.58
  pmullw %xmm1, %xmm0 #7.58
  pmulhw %xmm2, %xmm1 #7.58
  movdqa %xmm0, %xmm2 #7.58
  punpcklwd %xmm1, %xmm2 #7.58
  punpckhwd %xmm1, %xmm0 #7.58
  movdqu 112(%rsp), %xmm1 #7.58[spill]
  movdqu %xmm0, 128(%rsp) #7.58[spill]
  movdqa %xmm1, %xmm0 #7.58
  pmullw %xmm3, %xmm0 #7.58
  pmulhw %xmm1, %xmm3 #7.58
  movdqa %xmm0, %xmm1 #7.58
  punpcklwd %xmm3, %xmm1 #7.58
  punpckhwd %xmm3, %xmm0 #7.58
  pxor %xmm3, %xmm3 #7.37
  paddd %xmm3, %xmm15 #7.37
  paddd %xmm3, %xmm8 #7.37
  paddd %xmm14, %xmm15 #7.37
  paddd %xmm9, %xmm8 #7.37
  paddd %xmm4, %xmm15 #7.37
  paddd %xmm10, %xmm8 #7.37
  paddd %xmm5, %xmm15 #7.37
  paddd %xmm11, %xmm8 #7.37
  paddd %xmm6, %xmm15 #7.37
  paddd %xmm12, %xmm8 #7.37
  paddd %xmm7, %xmm15 #7.37
  paddd %xmm13, %xmm8 #7.37
  paddd 128(%rsp), %xmm8 #7.37[spill]
  paddd %xmm2, %xmm15 #7.37
  paddd %xmm1, %xmm15 #7.37
  paddd %xmm0, %xmm8 #7.37
  movdqu %xmm15, d.128.0.2(,%rax,4) #17.15
  movdqu %xmm8, 16+d.128.0.2(,%rax,4) #17.15
  addq $8, %rax #5.5
  cmpq $256, %rax #5.5
  jb ..B1.4 # Prob 81% #5.5
..B1.5: # Preds ..B1.4
  movl $146959, %esi #18.26
  xorl %eax, %eax #19.5
  movq $0x100000001b3, %rdx #23.9
..B1.6: # Preds ..B1.6 ..B1.5
  movl d.128.0.2(,%rax,8), %ecx #20.25
  movzwl %cx, %edi #20.32
  xorq %rdi, %rsi #20.9
  imulq %rdx, %rsi #21.9
  shrl $16, %ecx #22.34
  xorq %rcx, %rsi #22.9
  imulq %rdx, %rsi #23.9
  movl 4+d.128.0.2(,%rax,8), %r8d #20.25
  incq %rax #19.5
  movzwl %r8w, %r9d #20.32
  xorq %r9, %rsi #20.9
  imulq %rdx, %rsi #21.9
  shrl $16, %r8d #22.34
  xorq %r8, %rsi #22.9
  imulq %rdx, %rsi #23.9
  cmpq $128, %rax #19.5
  jb ..B1.6 # Prob 99% #19.5
..B1.7: # Preds ..B1.6
  movl $.L_2__STRING.0, %edi #25.5
  xorl %eax, %eax #25.5
  call printf #25.5
..B1.8: # Preds ..B1.7
  xorl %eax, %eax #26.12
  movq %rbp, %rsp #26.12
  popq %rbp #26.12
  ret #26.12
a.128.0.2:
d.128.0.2:
c.128.0.2:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
.L_2il0floatpacket.6:
.L_2__STRING.0:
