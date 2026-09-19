main:
..B1.1: # Preds ..B1.0
  pushq %rbp #54.16
  movq %rsp, %rbp #54.16
  andq $-128, %rsp #54.16
  pushq %r12 #54.16
  pushq %r13 #54.16
  pushq %r14 #54.16
  pushq %r15 #54.16
  pushq %rbx #54.16
  subq $88, %rsp #54.16
  movl $3, %edi #54.16
  xorl %esi, %esi #54.16
  call __intel_new_feature_proc_init #54.16
..B1.24: # Preds ..B1.1
  stmxcsr (%rsp) #54.16
  movl $42, %edx #55.23
  xorl %eax, %eax #56.5
  orl $32832, (%rsp) #54.16
  ldmxcsr (%rsp) #54.16
..B1.2: # Preds ..B1.2 ..B1.24
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
  xorl %esi, %esi #12.12
  xorl %eax, %eax #13.5
  movq %rsi, (%rsp) #14.9[spill]
  movq %rsi, %rdi #14.9
..B1.4: # Preds ..B1.4 ..B1.3
  lea (%rax,%rax), %edx #14.14
  incl %eax #13.5
  movslq array(,%rdx,4), %rcx #61.25
  addq %rcx, %rsi #14.9
  movslq 4+array(,%rdx,4), %rbx #61.25
  addq %rbx, %rdi #14.9
  cmpl $5000000, %eax #13.5
  jb ..B1.4 # Prob 64% #13.5
..B1.5: # Preds ..B1.4
  movq %rdi, (%rsp) #[spill]
  xorl %eax, %eax #21.5
  pxor %xmm2, %xmm2 #20.12
  movdqa %xmm2, %xmm1 #20.12
  movdqu .L_2il0floatpacket.0(%rip), %xmm3 #22.30
  movdqa %xmm1, %xmm4 #20.12
  movdqu .L_2il0floatpacket.1(%rip), %xmm0 #22.30
..B1.6: # Preds ..B1.6 ..B1.5
  movdqu array(,%rax,4), %xmm7 #62.28
  movdqa %xmm7, %xmm5 #22.30
  movdqa %xmm7, %xmm6 #22.22
  movdqu 16+array(,%rax,4), %xmm14 #62.28
  pcmpgtd %xmm2, %xmm6 #22.22
  punpckldq %xmm7, %xmm5 #22.30
  movdqa %xmm14, %xmm10 #22.30
  psrldq $8, %xmm7 #22.30
  movdqa %xmm5, %xmm9 #22.30
  punpckldq %xmm7, %xmm7 #22.30
  psrad $31, %xmm9 #22.30
  punpckldq %xmm14, %xmm10 #22.30
  movdqa %xmm14, %xmm11 #22.22
  psrldq $8, %xmm14 #22.30
  movdqa %xmm7, %xmm13 #22.30
  punpckldq %xmm14, %xmm14 #22.30
  pand %xmm3, %xmm9 #22.30
  pand %xmm0, %xmm5 #22.30
  movdqa %xmm6, %xmm8 #22.22
  psrad $31, %xmm13 #22.30
  por %xmm5, %xmm9 #22.30
  pcmpgtd %xmm2, %xmm11 #22.22
  movdqa %xmm10, %xmm12 #22.30
  movdqa %xmm14, %xmm5 #22.30
  pand %xmm3, %xmm13 #22.30
  psrldq $8, %xmm8 #22.22
  pand %xmm0, %xmm7 #22.30
  movdqa %xmm11, %xmm15 #22.22
  psrad $31, %xmm12 #22.30
  psrad $31, %xmm5 #22.30
  por %xmm7, %xmm13 #22.30
  punpckldq %xmm6, %xmm6 #22.22
  pand %xmm3, %xmm12 #22.30
  punpckldq %xmm8, %xmm8 #22.22
  pand %xmm0, %xmm10 #22.30
  psrldq $8, %xmm15 #22.22
  pand %xmm3, %xmm5 #22.30
  pand %xmm0, %xmm14 #22.30
  pand %xmm6, %xmm9 #22.30
  pand %xmm8, %xmm13 #22.30
  por %xmm10, %xmm12 #22.30
  punpckldq %xmm11, %xmm11 #22.22
  por %xmm14, %xmm5 #22.30
  punpckldq %xmm15, %xmm15 #22.22
  addq $8, %rax #21.5
  paddq %xmm9, %xmm1 #22.25
  paddq %xmm13, %xmm4 #22.25
  pand %xmm11, %xmm12 #22.30
  pand %xmm15, %xmm5 #22.30
  paddq %xmm12, %xmm1 #22.25
  paddq %xmm5, %xmm4 #22.25
  cmpq $10000000, %rax #21.5
  jb ..B1.6 # Prob 82% #21.5
..B1.7: # Preds ..B1.6
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
  movd %ebx, %xmm0 #28.12
  pshufd $0, %xmm0, %xmm0 #28.12
..B1.8: # Preds ..B1.8 ..B1.7
  movdqu 4+array(,%rax,4), %xmm2 #63.23
  movdqa %xmm2, %xmm3 #30.9
  pxor %xmm0, %xmm2 #30.9
  pcmpgtd %xmm0, %xmm3 #30.9
  pand %xmm2, %xmm3 #30.9
  pxor %xmm0, %xmm3 #30.9
  movdqu 20+array(,%rax,4), %xmm0 #63.23
  movdqa %xmm0, %xmm6 #30.9
  pxor %xmm3, %xmm0 #30.9
  pcmpgtd %xmm3, %xmm6 #30.9
  movdqu 36+array(,%rax,4), %xmm5 #63.23
  pand %xmm0, %xmm6 #30.9
  pxor %xmm3, %xmm6 #30.9
  movdqa %xmm5, %xmm8 #30.9
  pcmpgtd %xmm6, %xmm8 #30.9
  pxor %xmm6, %xmm5 #30.9
  movdqu 52+array(,%rax,4), %xmm7 #63.23
  pand %xmm5, %xmm8 #30.9
  pxor %xmm6, %xmm8 #30.9
  movdqa %xmm7, %xmm0 #30.9
  pcmpgtd %xmm8, %xmm0 #30.9
  pxor %xmm8, %xmm7 #30.9
  addq $16, %rax #29.5
  pand %xmm7, %xmm0 #30.9
  pxor %xmm8, %xmm0 #30.9
  cmpq $9999987, %rax #29.5
  jb ..B1.8 # Prob 82% #29.5
..B1.9: # Preds ..B1.8
  movdqa %xmm0, %xmm2 #28.12
  movdqa %xmm0, %xmm3 #28.12
  psrldq $8, %xmm2 #28.12
  movabsq $array+39999996, %r15 #63.23
  pcmpgtd %xmm2, %xmm3 #28.12
  pxor %xmm2, %xmm0 #28.12
  pand %xmm0, %xmm3 #28.12
  pxor %xmm2, %xmm3 #28.12
  movdqa %xmm3, %xmm0 #28.12
  movdqa %xmm3, %xmm5 #28.12
  psrldq $4, %xmm0 #28.12
  pcmpgtd %xmm0, %xmm5 #28.12
  pxor %xmm0, %xmm3 #28.12
  pand %xmm3, %xmm5 #28.12
  pxor %xmm0, %xmm5 #28.12
  movd %xmm5, %ecx #28.12
  movdqu .L_2il0floatpacket.2(%rip), %xmm0 #66.36
  movl (%r15), %r14d #63.23
  movl -4(%r15), %r13d #63.23
  movl -8(%r15), %r12d #63.23
  movl -12(%r15), %r11d #63.23
  movl -16(%r15), %r10d #63.23
  movl -20(%r15), %r9d #63.23
  movl -24(%r15), %r8d #63.23
  movl -28(%r15), %edi #63.23
  movl -32(%r15), %ebx #63.23
  movl -36(%r15), %edx #63.23
  movl -40(%r15), %eax #63.23
  movl -44(%r15), %r15d #63.23
  cmpl %ecx, %r15d #30.9
  cmovg %r15d, %ecx #30.9
  cmpl %ecx, %eax #30.9
  cmovg %eax, %ecx #30.9
  xorl %eax, %eax #36.5
  cmpl %ecx, %edx #30.9
  cmovg %edx, %ecx #30.9
  cmpl %ecx, %ebx #30.9
  cmovg %ebx, %ecx #30.9
  cmpl %ecx, %edi #30.9
  cmovg %edi, %ecx #30.9
  cmpl %ecx, %r8d #30.9
  cmovg %r8d, %ecx #30.9
  cmpl %ecx, %r9d #30.9
  cmovg %r9d, %ecx #30.9
  cmpl %ecx, %r10d #30.9
  cmovg %r10d, %ecx #30.9
  cmpl %ecx, %r11d #30.9
  cmovg %r11d, %ecx #30.9
  cmpl %ecx, %r12d #30.9
  cmovg %r12d, %ecx #30.9
  cmpl %ecx, %r13d #30.9
  cmovg %r13d, %ecx #30.9
  cmpl %ecx, %r14d #30.9
  cmovg %r14d, %ecx #30.9
..B1.10: # Preds ..B1.10 ..B1.9
  movdqu array(,%rax,4), %xmm2 #66.20
  movdqa %xmm2, %xmm3 #37.27
  paddd %xmm2, %xmm3 #37.27
  paddd %xmm2, %xmm3 #37.27
  paddd %xmm0, %xmm3 #37.35
  movntdq %xmm3, buf.145.0.7(,%rax,4) #66.15
  addq $4, %rax #36.5
  cmpq $10000000, %rax #36.5
  jb ..B1.10 # Prob 82% #36.5
..B1.11: # Preds ..B1.10
  mfence #36.5
..B1.12: # Preds ..B1.11
  xorl %r8d, %r8d #12.12
  xorl %eax, %eax #13.5
  xorl %edi, %edi #14.9
..B1.13: # Preds ..B1.13 ..B1.12
  lea (%rax,%rax), %edx #14.14
  incl %eax #13.5
  movslq buf.145.0.7(,%rdx,4), %rbx #67.25
  addq %rbx, %r8 #14.9
  movslq 4+buf.145.0.7(,%rdx,4), %r9 #67.25
  addq %r9, %rdi #14.9
  cmpl $5000000, %eax #13.5
  jb ..B1.13 # Prob 64% #13.5
..B1.14: # Preds ..B1.13
  xorl %r9d, %r9d #42.12
  xorl %eax, %eax #43.5
  xorl %ebx, %ebx #44.9
..B1.15: # Preds ..B1.15 ..B1.14
  lea (%rax,%rax), %edx #44.20
  incl %eax #43.5
  movslq buf.145.0.7(,%rdx,4), %r10 #69.34
  movslq 4+buf.145.0.7(,%rdx,4), %r12 #69.34
  movslq array(,%rdx,4), %r11 #69.27
  movslq 4+array(,%rdx,4), %r13 #69.27
  imulq %r10, %r11 #44.27
  imulq %r12, %r13 #44.27
  addq %r11, %r9 #44.9
  addq %r13, %rbx #44.9
  cmpl $500000, %eax #43.5
  jb ..B1.15 # Prob 64% #43.5
..B1.16: # Preds ..B1.15
  movl $42, %r11d #72.5
  xorl %r10d, %r10d #73.16
..B1.17: # Preds ..B1.17 ..B1.16
  imull $1664525, %r11d, %r13d #74.23
  movl $1374389535, %eax #75.33
  lea 1013904223(%r13), %r11d #74.34
  mull %r11d #75.33
  shrl $5, %edx #75.33
  imull $-100, %edx, %r12d #75.33
  lea 1013904223(%r12,%r13), %r14d #75.33
  movl %r14d, array(,%r10,4) #75.9
  incq %r10 #73.32
  cmpq $10000, %r10 #73.25
  jl ..B1.17 # Prob 99% #73.25
..B1.18: # Preds ..B1.17
  xorl %eax, %eax #50.5
..B1.19: # Preds ..B1.19 ..B1.18
  lea (%rax,%rax), %edx #51.9
  incl %eax #50.5
  movl 4+array(,%rdx,4), %r10d #77.16
  addl array(,%rdx,4), %r10d #51.9
  addl %r10d, 8+array(,%rdx,4) #51.9
  movl %r10d, 4+array(,%rdx,4) #77.16
  cmpl $4999, %eax #50.5
  jb ..B1.19 # Prob 64% #50.5
..B1.20: # Preds ..B1.19
  paddq %xmm4, %xmm1 #20.12
  movdqa %xmm1, %xmm0 #20.12
  addq $-16, %rsp #79.5
  psrldq $8, %xmm0 #20.12
  addq %rdi, %r8 #79.5
  paddq %xmm0, %xmm1 #20.12
  movl 39996+array(%rip), %eax #77.16
  addq %rbx, %r9 #79.5
  movq %xmm1, %rdx #20.12
  movl $.L_2__STRING.0, %edi #79.5
  addl 39992+array(%rip), %eax #51.9
  addq 16(%rsp), %rsi #79.5[spill]
  movl %eax, 39996+array(%rip) #77.16
  movl %eax, (%rsp) #79.5
  xorl %eax, %eax #79.5
  call printf #79.5
..B1.25: # Preds ..B1.20
  addq $16, %rsp #79.5
..B1.21: # Preds ..B1.25
  xorl %eax, %eax #81.12
  addq $88, %rsp #81.12
  popq %rbx #81.12
  popq %r15 #81.12
  popq %r14 #81.12
  popq %r13 #81.12
  popq %r12 #81.12
  movq %rbp, %rsp #81.12
  popq %rbp #81.12
  ret #81.12
buf.145.0.7:
array:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2__STRING.0:
