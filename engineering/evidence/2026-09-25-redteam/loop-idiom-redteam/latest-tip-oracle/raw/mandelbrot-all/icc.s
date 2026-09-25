main:
..B1.1: # Preds ..B1.0
  pushq %rbp #12.16
  movq %rsp, %rbp #12.16
  andq $-128, %rsp #12.16
  subq $128, %rsp #12.16
  movl $3, %edi #12.16
  xorl %esi, %esi #12.16
  call __intel_new_feature_proc_init #12.16
..B1.14: # Preds ..B1.1
  stmxcsr (%rsp) #12.16
  xorl %esi, %esi #13.15
  orl $32832, (%rsp) #12.16
  xorl %edx, %edx #13.15
  ldmxcsr (%rsp) #12.16
  movsd .L_2il0floatpacket.0(%rip), %xmm3 #15.31
  xorl %eax, %eax #14.16
  movsd .L_2il0floatpacket.3(%rip), %xmm2 #15.40
  movsd .L_2il0floatpacket.1(%rip), %xmm1 #17.43
  movsd .L_2il0floatpacket.2(%rip), %xmm0 #24.41
..B1.2: # Preds ..B1.8 ..B1.14
  pxor %xmm4, %xmm4 #15.27
  movl %edx, %ecx #16.20
  cvtsi2sd %eax, %xmm4 #15.27
  addsd %xmm4, %xmm4 #15.27
  divsd %xmm3, %xmm4 #15.31
  subsd %xmm2, %xmm4 #15.40
..B1.3: # Preds ..B1.7 ..B1.2
  pxor %xmm8, %xmm8 #17.31
  movl %edx, %edi #20.18
  cvtsi2sd %ecx, %xmm8 #17.31
  pxor %xmm7, %xmm7 #18.23
  addsd %xmm8, %xmm8 #17.31
  divsd %xmm3, %xmm8 #17.35
  movaps %xmm7, %xmm6 #18.33
  subsd %xmm1, %xmm8 #17.43
  movaps %xmm6, %xmm5 #21.44
..B1.4: # Preds ..B1.5 ..B1.3
  movaps %xmm7, %xmm9 #21.34
  mulsd %xmm7, %xmm9 #21.34
  addsd %xmm7, %xmm7 #22.28
  mulsd %xmm6, %xmm7 #22.33
  addsd %xmm8, %xmm9 #21.49
  movaps %xmm4, %xmm6 #22.38
  subsd %xmm5, %xmm9 #21.44
  addsd %xmm7, %xmm6 #22.38
  movaps %xmm6, %xmm5 #24.36
  movaps %xmm9, %xmm7 #23.17
  mulsd %xmm6, %xmm5 #24.36
  mulsd %xmm9, %xmm9 #24.26
  addsd %xmm5, %xmm9 #24.36
  comisd %xmm0, %xmm9 #24.41
  ja ..B1.7 # Prob 20% #24.41
..B1.5: # Preds ..B1.4
  incl %edi #20.39
  cmpl $50, %edi #20.29
  jl ..B1.4 # Prob 98% #20.29
..B1.7: # Preds ..B1.4 ..B1.5
  incl %ecx #16.36
  addl %edi, %esi #26.13
  cmpl $4000, %ecx #16.29
  jl ..B1.3 # Prob 99% #16.29
..B1.8: # Preds ..B1.7
  incl %eax #14.33
  cmpl $4000, %eax #14.25
  jl ..B1.2 # Prob 99% #14.25
..B1.9: # Preds ..B1.8
  movl $.L_2__STRING.0, %edi #29.5
  xorl %eax, %eax #29.5
  call printf #29.5
..B1.10: # Preds ..B1.9
  xorl %eax, %eax #30.12
  movq %rbp, %rsp #30.12
  popq %rbp #30.12
  ret #30.12
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2__STRING.0:
