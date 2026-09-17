main:
..B1.1: # Preds ..B1.0
  pushq %rbp #54.16
  movq %rsp, %rbp #54.16
  andq $-128, %rsp #54.16
  pushq %r13 #54.16
  pushq %r14 #54.16
  pushq %r15 #54.16
  pushq %rbx #54.16
  subq $96, %rsp #54.16
  movl $3, %edi #54.16
  xorl %esi, %esi #54.16
  call __intel_new_feature_proc_init #54.16
..B1.22: # Preds ..B1.1
  stmxcsr (%rsp) #54.16
  movl $42, %edx #55.23
  xorl %eax, %eax #56.5
  orl $32832, (%rsp) #54.16
  ldmxcsr (%rsp) #54.16
..B1.2: # Preds ..B1.2 ..B1.22
  imull $1664525, %edx, %ecx #57.23
  addl $1013904223, %ecx #57.34
  movl %ecx, %edx #58.34
  shrl $1, %edx #58.34
  addl $-1000000000, %edx #58.39
  movl %edx, array(,%rax,8) #58.9
  imull $1664525, %ecx, %edx #57.23
  addl $1013904223, %edx #57.34
  movl %edx, %ebx #58.34
  shrl $1, %ebx #58.34
  addl $-1000000000, %ebx #58.39
  movl %ebx, 4+array(,%rax,8) #58.9
  incq %rax #56.5
  cmpq $5000000, %rax #56.5
  jb ..B1.2 # Prob 99% #56.5
..B1.3: # Preds ..B1.2
  movdqu .L_2il0floatpacket.0(%rip), %xmm5 #14.14
  xorl %eax, %eax #13.5
  pxor %xmm6, %xmm6 #12.12
  movdqa %xmm6, %xmm1 #12.12
  movdqa %xmm1, %xmm2 #12.12
  movdqa %xmm2, %xmm3 #20.12
  movdqu .L_2il0floatpacket.1(%rip), %xmm4 #14.14
  movdqa %xmm3, %xmm0 #20.12
..B1.4: # Preds ..B1.4 ..B1.3
  movdqu array(,%rax,4), %xmm9 #61.25
  addq $4, %rax #13.5
  movdqa %xmm9, %xmm8 #14.14
  movdqa %xmm9, %xmm7 #14.14
  psrldq $8, %xmm8 #14.14
  punpckldq %xmm9, %xmm7 #14.14
  pcmpgtd %xmm6, %xmm9 #22.22
  punpckldq %xmm8, %xmm8 #14.14
  movdqa %xmm7, %xmm10 #14.14
  movdqa %xmm8, %xmm12 #14.14
  psrad $31, %xmm10 #14.14
  psrad $31, %xmm12 #14.14
  movdqa %xmm9, %xmm11 #22.22
  psrldq $8, %xmm11 #22.22
  pand %xmm5, %xmm10 #14.14
  pand %xmm4, %xmm7 #14.14
  pand %xmm5, %xmm12 #14.14
  pand %xmm4, %xmm8 #14.14
  por %xmm7, %xmm10 #14.14
  punpckldq %xmm9, %xmm9 #22.22
  por %xmm8, %xmm12 #14.14
  punpckldq %xmm11, %xmm11 #22.22
  paddq %xmm10, %xmm1 #14.9
  paddq %xmm12, %xmm2 #14.9
  pand %xmm9, %xmm10 #22.30
  pand %xmm11, %xmm12 #22.30
  paddq %xmm10, %xmm3 #22.25
  paddq %xmm12, %xmm0 #22.25
  cmpq $10000000, %rax #13.5
  jb ..B1.4 # Prob 82% #13.5
..B1.5: # Preds ..B1.4
  movl array(%rip), %ebx #63.23
  movl 4+array(%rip), %eax #63.23
  cmpl %ebx, %eax #28.12
  movl 8+array(%rip), %edx #63.23
  cmovg %eax, %ebx #28.12
  movl $3, %eax #29.5
  cmpl %ebx, %edx #28.12
  movl 12+array(%rip), %ecx #63.23
  cmovg %edx, %ebx #28.12
  cmpl %ebx, %ecx #28.12
  cmovg %ecx, %ebx #28.12
  movd %ebx, %xmm4 #28.12
  pshufd $0, %xmm4, %xmm4 #28.12
..B1.6: # Preds ..B1.6 ..B1.5
  movdqu 4+array(,%rax,4), %xmm5 #63.23
  movdqa %xmm5, %xmm6 #30.9
  pxor %xmm4, %xmm5 #30.9
  pcmpgtd %xmm4, %xmm6 #30.9
  pand %xmm5, %xmm6 #30.9
  pxor %xmm4, %xmm6 #30.9
  movdqu 20+array(,%rax,4), %xmm4 #63.23
  movdqa %xmm4, %xmm8 #30.9
  pxor %xmm6, %xmm4 #30.9
  pcmpgtd %xmm6, %xmm8 #30.9
  movdqu 36+array(,%rax,4), %xmm7 #63.23
  pand %xmm4, %xmm8 #30.9
  pxor %xmm6, %xmm8 #30.9
  movdqa %xmm7, %xmm10 #30.9
  pcmpgtd %xmm8, %xmm10 #30.9
  pxor %xmm8, %xmm7 #30.9
  movdqu 52+array(,%rax,4), %xmm9 #63.23
  pand %xmm7, %xmm10 #30.9
  pxor %xmm8, %xmm10 #30.9
  movdqa %xmm9, %xmm4 #30.9
  pcmpgtd %xmm10, %xmm4 #30.9
  pxor %xmm10, %xmm9 #30.9
  addq $16, %rax #29.5
  pand %xmm9, %xmm4 #30.9
  pxor %xmm10, %xmm4 #30.9
  cmpq $9999987, %rax #29.5
  jb ..B1.6 # Prob 82% #29.5
..B1.7: # Preds ..B1.6
  movdqa %xmm4, %xmm5 #28.12
  movdqa %xmm4, %xmm6 #28.12
  psrldq $8, %xmm5 #28.12
  movabsq $array+39999996, %r15 #63.23
  pcmpgtd %xmm5, %xmm6 #28.12
  pxor %xmm5, %xmm4 #28.12
  pand %xmm4, %xmm6 #28.12
  pxor %xmm5, %xmm6 #28.12
  movdqa %xmm6, %xmm4 #28.12
  movdqa %xmm6, %xmm7 #28.12
  psrldq $4, %xmm4 #28.12
  pcmpgtd %xmm4, %xmm7 #28.12
  pxor %xmm4, %xmm6 #28.12
  pand %xmm6, %xmm7 #28.12
  pxor %xmm4, %xmm7 #28.12
  movd %xmm7, %ecx #28.12
  movdqu .L_2il0floatpacket.2(%rip), %xmm4 #66.36
  movl (%r15), %r14d #63.23
  movl -4(%r15), %r13d #63.23
  movl -8(%r15), %r11d #63.23
  movl -12(%r15), %r10d #63.23
  movl -16(%r15), %r9d #63.23
  movl -20(%r15), %r8d #63.23
  movl -24(%r15), %edi #63.23
  movl -28(%r15), %esi #63.23
  movl -32(%r15), %ebx #63.23
  movl -36(%r15), %edx #63.23
  movl -40(%r15), %eax #63.23
  movl -44(%r15), %r15d #63.23
  cmpl %ecx, %r15d #30.9
  cmovg %r15d, %ecx #30.9
  cmpl %ecx, %eax #30.9
  cmovg %eax, %ecx #30.9
  cmpl %ecx, %edx #30.9
  cmovg %edx, %ecx #30.9
  cmpl %ecx, %ebx #30.9
  cmovg %ebx, %ecx #30.9
  cmpl %ecx, %esi #30.9
  cmovg %esi, %ecx #30.9
  cmpl %ecx, %edi #30.9
  cmovg %edi, %ecx #30.9
  cmpl %ecx, %r8d #30.9
  cmovg %r8d, %ecx #30.9
  xorl %r8d, %r8d #12.12
  cmpl %ecx, %r9d #30.9
  cmovg %r9d, %ecx #30.9
  xorl %eax, %eax #30.9
  cmpl %ecx, %r10d #30.9
  cmovg %r10d, %ecx #30.9
  cmpl %ecx, %r11d #30.9
  cmovg %r11d, %ecx #30.9
  cmpl %ecx, %r13d #30.9
  cmovg %r13d, %ecx #30.9
  cmpl %ecx, %r14d #30.9
  cmovg %r14d, %ecx #30.9
..B1.8: # Preds ..B1.8 ..B1.7
  movdqu array(,%rax,4), %xmm5 #66.20
  movdqa %xmm5, %xmm6 #37.27
  paddd %xmm5, %xmm6 #37.27
  paddd %xmm5, %xmm6 #37.27
  paddd %xmm4, %xmm6 #37.35
  movntdq %xmm6, buf.145.0.7(,%rax,4) #66.15
  addq $4, %rax #36.5
  cmpq $10000000, %rax #36.5
  jb ..B1.8 # Prob 82% #36.5
..B1.9: # Preds ..B1.8
  mfence #36.5
..B1.10: # Preds ..B1.9
  xorl %eax, %eax #36.5
  xorl %edi, %edi #14.9
..B1.11: # Preds ..B1.11 ..B1.10
  lea (%rax,%rax), %edx #14.14
  incl %eax #36.5
  movslq buf.145.0.7(,%rdx,4), %rbx #67.25
  addq %rbx, %r8 #14.9
  movslq 4+buf.145.0.7(,%rdx,4), %rsi #67.25
  addq %rsi, %rdi #14.9
  cmpl $5000000, %eax #36.5
  jb ..B1.11 # Prob 64% #36.5
..B1.12: # Preds ..B1.11
  xorl %r9d, %r9d #42.12
  xorl %eax, %eax #43.5
  xorl %ebx, %ebx #44.9
..B1.13: # Preds ..B1.13 ..B1.12
  lea (%rax,%rax), %edx #44.20
  incl %eax #43.5
  movslq buf.145.0.7(,%rdx,4), %rsi #69.34
  movslq 4+buf.145.0.7(,%rdx,4), %r11 #69.34
  movslq array(,%rdx,4), %r10 #69.27
  movslq 4+array(,%rdx,4), %r13 #69.27
  imulq %rsi, %r10 #44.27
  imulq %r11, %r13 #44.27
  addq %r10, %r9 #44.9
  addq %r13, %rbx #44.9
  cmpl $500000, %eax #43.5
  jb ..B1.13 # Prob 64% #43.5
..B1.14: # Preds ..B1.13
  movl $42, %r10d #72.5
  xorl %esi, %esi #73.5
..B1.15: # Preds ..B1.15 ..B1.14
  imull $1664525, %r10d, %r13d #74.23
  movl $1374389535, %eax #75.33
  lea 1013904223(%r13), %r10d #74.34
  mull %r10d #75.33
  shrl $5, %edx #75.33
  imull $-100, %edx, %r11d #75.33
  lea 1013904223(%r11,%r13), %r14d #75.33
  movl %r14d, array(,%rsi,4) #75.9
  incq %rsi #73.5
  cmpq $10000, %rsi #73.5
  jb ..B1.15 # Prob 99% #73.5
..B1.16: # Preds ..B1.15
  movl array(%rip), %eax #77.16
  xorl %edx, %edx #50.5
..B1.17: # Preds ..B1.17 ..B1.16
  lea (%rdx,%rdx), %esi #51.9
  incl %edx #50.5
  addl 4+array(,%rsi,4), %eax #51.9
  movl 8+array(,%rsi,4), %r10d #77.16
  movl %eax, 4+array(,%rsi,4) #77.16
  addl %r10d, %eax #51.9
  movl %eax, 8+array(,%rsi,4) #77.16
  cmpl $4999, %edx #50.5
  jb ..B1.17 # Prob 64% #50.5
..B1.18: # Preds ..B1.17
  paddq %xmm0, %xmm3 #20.12
  paddq %xmm2, %xmm1 #12.12
  movdqa %xmm3, %xmm0 #20.12
  movdqa %xmm1, %xmm2 #12.12
  psrldq $8, %xmm0 #20.12
  addq $-16, %rsp #79.5
  psrldq $8, %xmm2 #12.12
  addq %rdi, %r8 #79.5
  paddq %xmm0, %xmm3 #20.12
  paddq %xmm2, %xmm1 #12.12
  movq %xmm3, %rdx #20.12
  movq %xmm1, %rsi #12.12
  movl 39996+array(%rip), %eax #77.16
  addq %rbx, %r9 #79.5
  addl 39992+array(%rip), %eax #51.9
  movl $.L_2__STRING.0, %edi #79.5
  movl %eax, 39996+array(%rip) #77.16
  movl %eax, (%rsp) #79.5
  xorl %eax, %eax #79.5
  call printf #79.5
..B1.23: # Preds ..B1.18
  addq $16, %rsp #79.5
..B1.19: # Preds ..B1.23
  xorl %eax, %eax #81.12
  addq $96, %rsp #81.12
  popq %rbx #81.12
  popq %r15 #81.12
  popq %r14 #81.12
  popq %r13 #81.12
  movq %rbp, %rsp #81.12
  popq %rbp #81.12
  ret #81.12
buf.145.0.7:
array:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2__STRING.0:
