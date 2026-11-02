main:
..B1.1: # Preds ..B1.0
  pushq %rbp #16.16
  movq %rsp, %rbp #16.16
  andq $-128, %rsp #16.16
  subq $128, %rsp #16.16
  movl $3, %edi #16.16
  xorl %esi, %esi #16.16
  call __intel_new_feature_proc_init #16.16
..B1.13: # Preds ..B1.1
  stmxcsr (%rsp) #16.16
  movl $1, %edx #17.19
  xorl %eax, %eax #18.5
  orl $32832, (%rsp) #16.16
  ldmxcsr (%rsp) #16.16
..B1.2: # Preds ..B1.2 ..B1.13
  imull $1664525, %edx, %ecx #19.23
  addl $1013904223, %ecx #19.34
  imull $1664525, %ecx, %edi #21.23
  movl %ecx, %edx #20.31
  addl $1013904223, %edi #21.34
  imull $1664525, %edi, %r9d #23.23
  movl %edi, %esi #22.31
  addl $1013904223, %r9d #23.34
  imull $1664525, %r9d, %r11d #19.23
  movl %r9d, %r8d #24.31
  addl $1013904223, %r11d #19.34
  imull $1664525, %r11d, %ecx #21.23
  movl %r11d, %r10d #20.31
  shrl $16, %edx #20.31
  addl $1013904223, %ecx #21.34
  andl $15, %edx #20.37
  addl $-7, %edx #20.44
  movl %edx, a(,%rax,8) #20.9
  movl %ecx, %edx #22.31
  shrl $16, %edx #22.31
  andl $15, %edx #22.37
  addl $-7, %edx #22.44
  movl %edx, 4+b(,%rax,8) #22.9
  imull $1664525, %ecx, %edx #23.23
  shrl $16, %esi #22.31
  addl $1013904223, %edx #23.34
  andl $15, %esi #22.37
  addl $-7, %esi #22.44
  movl %esi, b(,%rax,8) #22.9
  movl %edx, %esi #24.31
  shrl $16, %r8d #24.31
  shrl $16, %r10d #20.31
  andl $15, %r8d #24.37
  shrl $16, %esi #24.31
  andl $15, %r10d #20.37
  andl $15, %esi #24.37
  addl $-7, %r8d #24.44
  addl $-7, %r10d #20.44
  addl $-7, %esi #24.44
  movl %r8d, c(,%rax,8) #24.9
  movl %r10d, 4+a(,%rax,8) #20.9
  movl %esi, 4+c(,%rax,8) #24.9
  incq %rax #18.5
  cmpq $524288, %rax #18.5
  jb ..B1.2 # Prob 99% #18.5
..B1.3: # Preds ..B1.2
  movdqu .L_2il0floatpacket.0(%rip), %xmm3 #31.28
  xorl %esi, %esi #27.19
  xorl %eax, %eax #28.5
  pxor %xmm2, %xmm2 #31.28
..B1.4: # Preds ..B1.8 ..B1.3
  movdqa %xmm2, %xmm0 #29.18
  xorl %edx, %edx #30.9
  movdqa %xmm0, %xmm1 #29.28
..B1.5: # Preds ..B1.5 ..B1.4
  movdqu a(,%rdx,4), %xmm11 #31.21
  movdqu 16+a(,%rdx,4), %xmm13 #31.21
  movdqa %xmm11, %xmm5 #31.28
  movdqu b(,%rdx,4), %xmm4 #31.28
  movdqa %xmm11, %xmm10 #31.28
  movdqu c(,%rdx,4), %xmm8 #32.28
  psrlq $32, %xmm10 #31.28
  movdqu 16+c(,%rdx,4), %xmm9 #32.28
  movdqa %xmm13, %xmm12 #31.28
  pmuludq %xmm4, %xmm5 #31.28
  psrlq $32, %xmm4 #31.28
  pmuludq %xmm8, %xmm11 #32.28
  pmuludq %xmm10, %xmm4 #31.28
  movdqu 16+b(,%rdx,4), %xmm6 #31.28
  psrlq $32, %xmm8 #32.28
  movdqa %xmm13, %xmm7 #31.28
  psrlq $32, %xmm12 #31.28
  pmuludq %xmm6, %xmm7 #31.28
  psrlq $32, %xmm6 #31.28
  pmuludq %xmm8, %xmm10 #32.28
  pmuludq %xmm9, %xmm13 #32.28
  pmuludq %xmm12, %xmm6 #31.28
  psrlq $32, %xmm9 #32.28
  pand %xmm3, %xmm5 #31.28
  pmuludq %xmm9, %xmm12 #32.28
  psllq $32, %xmm4 #31.28
  pand %xmm3, %xmm11 #32.28
  psllq $32, %xmm10 #32.28
  por %xmm4, %xmm5 #31.28
  pand %xmm3, %xmm7 #31.28
  psllq $32, %xmm6 #31.28
  por %xmm10, %xmm11 #32.28
  pand %xmm3, %xmm13 #32.28
  psllq $32, %xmm12 #32.28
  addq $8, %rdx #30.9
  paddd %xmm5, %xmm0 #31.13
  por %xmm6, %xmm7 #31.28
  paddd %xmm11, %xmm1 #32.13
  por %xmm12, %xmm13 #32.28
  paddd %xmm7, %xmm0 #31.13
  paddd %xmm13, %xmm1 #32.13
  cmpq $1048576, %rdx #30.9
  jb ..B1.5 # Prob 99% #30.9
..B1.6: # Preds ..B1.5
  movdqa %xmm2, %xmm5 #36.17
  xorl %edx, %edx #37.9
  movdqa %xmm5, %xmm4 #36.26
..B1.7: # Preds ..B1.7 ..B1.6
  paddd a(,%rdx,4), %xmm5 #38.13
  paddd b(,%rdx,4), %xmm4 #39.13
  paddd 16+b(,%rdx,4), %xmm4 #39.13
  paddd 16+a(,%rdx,4), %xmm5 #38.13
  paddd 32+a(,%rdx,4), %xmm5 #38.13
  paddd 32+b(,%rdx,4), %xmm4 #39.13
  paddd 48+b(,%rdx,4), %xmm4 #39.13
  paddd 48+a(,%rdx,4), %xmm5 #38.13
  addq $16, %rdx #37.9
  cmpq $1048576, %rdx #37.9
  jb ..B1.7 # Prob 99% #37.9
..B1.8: # Preds ..B1.7
  movdqa %xmm0, %xmm6 #29.18
  incl %eax #28.5
  psrldq $8, %xmm6 #29.18
  paddd %xmm6, %xmm0 #29.18
  movdqa %xmm0, %xmm7 #29.18
  psrlq $32, %xmm7 #29.18
  paddd %xmm7, %xmm0 #29.18
  movd %xmm0, %edx #29.18
  movdqa %xmm1, %xmm0 #29.28
  psrldq $8, %xmm0 #29.28
  paddd %xmm0, %xmm1 #29.28
  movdqa %xmm1, %xmm8 #29.28
  psrlq $32, %xmm8 #29.28
  paddd %xmm8, %xmm1 #29.28
  movd %xmm1, %ecx #29.28
  movdqa %xmm5, %xmm1 #36.17
  psrldq $8, %xmm1 #36.17
  paddd %xmm1, %xmm5 #36.17
  movdqa %xmm5, %xmm9 #36.17
  psrlq $32, %xmm9 #36.17
  paddd %xmm9, %xmm5 #36.17
  movd %xmm5, %edi #36.17
  movdqa %xmm4, %xmm5 #36.26
  psrldq $8, %xmm5 #36.26
  paddd %xmm5, %xmm4 #36.26
  movdqa %xmm4, %xmm10 #36.26
  psrlq $32, %xmm10 #36.26
  paddd %xmm10, %xmm4 #36.26
  movd %xmm4, %r8d #36.26
  movslq %edx, %rdx #34.25
  movslq %ecx, %rcx #34.25
  addq %rcx, %rdx #34.25
  movslq %r8d, %r8 #36.26
  addq %rsi, %rdx #34.9
  movslq %edi, %rsi #36.17
  addq %r8, %rsi #41.24
  addq %rdx, %rsi #41.9
  cmpl $128, %eax #28.5
  jb ..B1.4 # Prob 90% #28.5
..B1.9: # Preds ..B1.8
  movl $.L_2__STRING.0, %edi #44.5
  xorl %eax, %eax #44.5
  call printf #44.5
..B1.10: # Preds ..B1.9
  xorl %eax, %eax #45.12
  movq %rbp, %rsp #45.12
  popq %rbp #45.12
  ret #45.12
a:
b:
c:
.L_2il0floatpacket.0:
.L_2__STRING.0:
