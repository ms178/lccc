main:
..B1.1: # Preds ..B1.0
  pushq %rbp #35.16
  movq %rsp, %rbp #35.16
  andq $-128, %rsp #35.16
  subq $64000, %rsp #35.16
  movl $3, %edi #35.16
  xorl %esi, %esi #35.16
  call __intel_new_feature_proc_init #35.16
..B1.27: # Preds ..B1.1
  stmxcsr (%rsp) #35.16
  xorl %eax, %eax #38.5
  orl $32832, (%rsp) #35.16
  ldmxcsr (%rsp) #35.16
  movups .L_2il0floatpacket.0(%rip), %xmm0 #38.40
..B1.2: # Preds ..B1.2 ..B1.27
  movups %xmm0, 48000(%rsp,%rax,8) #38.33
  movups %xmm0, 48016(%rsp,%rax,8) #38.33
  movups %xmm0, 48032(%rsp,%rax,8) #38.33
  movups %xmm0, 48048(%rsp,%rax,8) #38.33
  addq $8, %rax #38.5
  cmpq $2000, %rax #38.5
  jb ..B1.2 # Prob 99% #38.5
..B1.3: # Preds ..B1.2
  movq $0x100000000, %rax #24.13
  xorb %dl, %dl #40.5
  pxor %xmm1, %xmm1 #13.20
  pxor %xmm4, %xmm4 #8.24
  movdqu .L_2il0floatpacket.1(%rip), %xmm6 #8.24
  movaps %xmm1, %xmm3 #13.20
  movdqu .L_2il0floatpacket.2(%rip), %xmm5 #8.38
  movdqu .L_2il0floatpacket.3(%rip), %xmm2 #8.43
  movq %rax, %xmm0 #24.13
  xorl %eax, %eax #24.13
..B1.4: # Preds ..B1.20 ..B1.3
  movl %eax, %esi #12.5
  xorl %ecx, %ecx #12.5
..B1.5: # Preds ..B1.7 ..B1.4
  movd %esi, %xmm11 #8.24
  lea 1(%rsi), %edi #8.24
  addl $2, %esi #8.38
  movdqa %xmm11, %xmm9 #8.24
  movaps %xmm3, %xmm10 #13.20
  movd %edi, %xmm8 #8.24
  movd %esi, %xmm7 #8.38
  xorl %esi, %esi #14.9
  punpckldq %xmm8, %xmm9 #8.24
  punpckldq %xmm7, %xmm8 #8.38
  punpcklqdq %xmm4, %xmm9 #8.24
  punpcklqdq %xmm4, %xmm8 #8.38
  pshufd $0, %xmm11, %xmm7 #16.9
..B1.6: # Preds ..B1.6 ..B1.5
  movdqa %xmm9, %xmm12 #8.38
  movdqa %xmm8, %xmm11 #8.38
  movdqa %xmm9, %xmm14 #8.38
  psrlq $32, %xmm12 #8.38
  psrlq $32, %xmm11 #8.38
  movdqa %xmm4, %xmm13 #8.43
  pmuludq %xmm8, %xmm14 #8.38
  paddd %xmm6, %xmm9 #8.24
  pmuludq %xmm11, %xmm12 #8.38
  pand %xmm5, %xmm14 #8.38
  psllq $32, %xmm12 #8.38
  por %xmm12, %xmm14 #8.38
  paddd %xmm6, %xmm8 #8.38
  pcmpgtd %xmm14, %xmm13 #8.43
  pand %xmm2, %xmm13 #8.51
  paddd %xmm13, %xmm14 #8.51
  psrad $1, %xmm14 #8.51
  paddd %xmm7, %xmm14 #8.51
  paddd %xmm2, %xmm14 #8.51
  cvtdq2pd %xmm14, %xmm15 #8.51
  movups 48000(%rsp,%rsi,8), %xmm11 #41.21
  addq $2, %rsi #14.9
  divpd %xmm15, %xmm11 #15.30
  addpd %xmm11, %xmm10 #15.13
  cmpq $2000, %rsi #14.9
  jb ..B1.6 # Prob 82% #14.9
..B1.7: # Preds ..B1.6
  movaps %xmm10, %xmm7 #13.20
  movl %edi, %esi #12.5
  unpckhpd %xmm10, %xmm7 #13.20
  addsd %xmm7, %xmm10 #13.20
  movsd %xmm10, (%rsp,%rcx,8) #31.18
  incq %rcx #8.24
  cmpl $2000, %edi #12.5
  jb ..B1.5 # Prob 91% #12.5
..B1.8: # Preds ..B1.7
  movl %eax, %esi #21.5
  xorl %ecx, %ecx #21.5
..B1.9: # Preds ..B1.11 ..B1.8
  movd %esi, %xmm8 #8.24
  lea 1(%rsi), %edi #8.24
  addl $2, %esi #8.38
  movaps %xmm3, %xmm9 #22.20
  movd %edi, %xmm7 #8.24
  movd %esi, %xmm10 #8.38
  xorl %esi, %esi #23.9
  punpckldq %xmm7, %xmm8 #8.24
  punpckldq %xmm10, %xmm7 #8.38
  movdqa %xmm0, %xmm10 #24.13
  punpcklqdq %xmm4, %xmm8 #8.24
  punpcklqdq %xmm4, %xmm7 #8.38
..B1.10: # Preds ..B1.10 ..B1.9
  movdqa %xmm8, %xmm12 #8.38
  movdqa %xmm7, %xmm11 #8.38
  movdqa %xmm8, %xmm14 #8.38
  psrlq $32, %xmm12 #8.38
  psrlq $32, %xmm11 #8.38
  movdqa %xmm4, %xmm13 #8.43
  pmuludq %xmm7, %xmm14 #8.38
  paddd %xmm6, %xmm8 #8.24
  pmuludq %xmm11, %xmm12 #8.38
  pand %xmm5, %xmm14 #8.38
  psllq $32, %xmm12 #8.38
  por %xmm12, %xmm14 #8.38
  paddd %xmm6, %xmm7 #8.38
  pcmpgtd %xmm14, %xmm13 #8.43
  pand %xmm2, %xmm13 #8.51
  paddd %xmm13, %xmm14 #8.51
  psrad $1, %xmm14 #8.51
  paddd %xmm10, %xmm14 #8.51
  paddd %xmm6, %xmm10 #24.13
  paddd %xmm2, %xmm14 #8.51
  cvtdq2pd %xmm14, %xmm15 #8.51
  movups (%rsp,%rsi,8), %xmm11 #32.16
  addq $2, %rsi #23.9
  divpd %xmm15, %xmm11 #24.30
  addpd %xmm11, %xmm9 #24.13
  cmpq $2000, %rsi #23.9
  jb ..B1.10 # Prob 82% #23.9
..B1.11: # Preds ..B1.10
  movaps %xmm9, %xmm7 #22.20
  movl %edi, %esi #21.5
  unpckhpd %xmm9, %xmm7 #22.20
  addsd %xmm7, %xmm9 #22.20
  movsd %xmm9, 32000(%rsp,%rcx,8) #41.24
  incq %rcx #8.24
  cmpl $2000, %edi #21.5
  jb ..B1.9 # Prob 91% #21.5
..B1.12: # Preds ..B1.11
  movl %eax, %esi #12.5
  xorl %ecx, %ecx #12.5
..B1.13: # Preds ..B1.15 ..B1.12
  movd %esi, %xmm11 #8.24
  lea 1(%rsi), %edi #8.24
  addl $2, %esi #8.38
  movdqa %xmm11, %xmm9 #8.24
  movaps %xmm3, %xmm10 #13.20
  movd %edi, %xmm8 #8.24
  movd %esi, %xmm7 #8.38
  xorl %esi, %esi #14.9
  punpckldq %xmm8, %xmm9 #8.24
  punpckldq %xmm7, %xmm8 #8.38
  punpcklqdq %xmm4, %xmm9 #8.24
  punpcklqdq %xmm4, %xmm8 #8.38
  pshufd $0, %xmm11, %xmm7 #16.9
..B1.14: # Preds ..B1.14 ..B1.13
  movdqa %xmm9, %xmm12 #8.38
  movdqa %xmm8, %xmm11 #8.38
  movdqa %xmm9, %xmm14 #8.38
  psrlq $32, %xmm12 #8.38
  psrlq $32, %xmm11 #8.38
  movdqa %xmm4, %xmm13 #8.43
  pmuludq %xmm8, %xmm14 #8.38
  paddd %xmm6, %xmm9 #8.24
  pmuludq %xmm11, %xmm12 #8.38
  pand %xmm5, %xmm14 #8.38
  psllq $32, %xmm12 #8.38
  por %xmm12, %xmm14 #8.38
  paddd %xmm6, %xmm8 #8.38
  pcmpgtd %xmm14, %xmm13 #8.43
  pand %xmm2, %xmm13 #8.51
  paddd %xmm13, %xmm14 #8.51
  psrad $1, %xmm14 #8.51
  paddd %xmm7, %xmm14 #8.51
  paddd %xmm2, %xmm14 #8.51
  cvtdq2pd %xmm14, %xmm15 #8.51
  movups 32000(%rsp,%rsi,8), %xmm11 #42.21
  addq $2, %rsi #14.9
  divpd %xmm15, %xmm11 #15.30
  addpd %xmm11, %xmm10 #15.13
  cmpq $2000, %rsi #14.9
  jb ..B1.14 # Prob 82% #14.9
..B1.15: # Preds ..B1.14
  movaps %xmm10, %xmm7 #13.20
  movl %edi, %esi #12.5
  unpckhpd %xmm10, %xmm7 #13.20
  addsd %xmm7, %xmm10 #13.20
  movsd %xmm10, 16000(%rsp,%rcx,8) #31.18
  incq %rcx #8.24
  cmpl $2000, %edi #12.5
  jb ..B1.13 # Prob 91% #12.5
..B1.16: # Preds ..B1.15
  movl %eax, %esi #21.5
  xorl %ecx, %ecx #21.5
..B1.17: # Preds ..B1.19 ..B1.16
  movd %esi, %xmm8 #8.24
  lea 1(%rsi), %edi #8.24
  addl $2, %esi #8.38
  movaps %xmm3, %xmm9 #22.20
  movd %edi, %xmm7 #8.24
  movd %esi, %xmm10 #8.38
  xorl %esi, %esi #23.9
  punpckldq %xmm7, %xmm8 #8.24
  punpckldq %xmm10, %xmm7 #8.38
  movdqa %xmm0, %xmm10 #24.13
  punpcklqdq %xmm4, %xmm8 #8.24
  punpcklqdq %xmm4, %xmm7 #8.38
..B1.18: # Preds ..B1.18 ..B1.17
  movdqa %xmm8, %xmm12 #8.38
  movdqa %xmm7, %xmm11 #8.38
  movdqa %xmm8, %xmm14 #8.38
  psrlq $32, %xmm12 #8.38
  psrlq $32, %xmm11 #8.38
  movdqa %xmm4, %xmm13 #8.43
  pmuludq %xmm7, %xmm14 #8.38
  paddd %xmm6, %xmm8 #8.24
  pmuludq %xmm11, %xmm12 #8.38
  pand %xmm5, %xmm14 #8.38
  psllq $32, %xmm12 #8.38
  por %xmm12, %xmm14 #8.38
  paddd %xmm6, %xmm7 #8.38
  pcmpgtd %xmm14, %xmm13 #8.43
  pand %xmm2, %xmm13 #8.51
  paddd %xmm13, %xmm14 #8.51
  psrad $1, %xmm14 #8.51
  paddd %xmm10, %xmm14 #8.51
  paddd %xmm6, %xmm10 #24.13
  paddd %xmm2, %xmm14 #8.51
  cvtdq2pd %xmm14, %xmm15 #8.51
  movups 16000(%rsp,%rsi,8), %xmm11 #32.16
  addq $2, %rsi #23.9
  divpd %xmm15, %xmm11 #24.30
  addpd %xmm11, %xmm9 #24.13
  cmpq $2000, %rsi #23.9
  jb ..B1.18 # Prob 82% #23.9
..B1.19: # Preds ..B1.18
  movaps %xmm9, %xmm7 #22.20
  movl %edi, %esi #21.5
  unpckhpd %xmm9, %xmm7 #22.20
  addsd %xmm7, %xmm9 #22.20
  movsd %xmm9, 48000(%rsp,%rcx,8) #42.24
  incq %rcx #8.24
  cmpl $2000, %edi #21.5
  jb ..B1.17 # Prob 91% #21.5
..B1.20: # Preds ..B1.19
  incb %dl #40.5
  cmpb $10, %dl #40.5
  jb ..B1.4 # Prob 99% #40.5
..B1.21: # Preds ..B1.20
  movaps %xmm3, %xmm0 #45.16
  xorl %eax, %eax #46.5
..B1.22: # Preds ..B1.22 ..B1.21
  movups 48000(%rsp,%rax,8), %xmm2 #47.16
  movups 32000(%rsp,%rax,8), %xmm4 #47.23
  mulpd %xmm4, %xmm2 #47.23
  mulpd %xmm4, %xmm4 #48.22
  addpd %xmm2, %xmm1 #47.9
  addpd %xmm4, %xmm3 #48.9
  movups 48016(%rsp,%rax,8), %xmm5 #47.16
  movups 32016(%rsp,%rax,8), %xmm6 #47.23
  mulpd %xmm6, %xmm5 #47.23
  mulpd %xmm6, %xmm6 #48.22
  addpd %xmm5, %xmm0 #47.9
  addpd %xmm3, %xmm6 #48.9
  movups 48032(%rsp,%rax,8), %xmm3 #47.16
  movups 32032(%rsp,%rax,8), %xmm8 #47.23
  mulpd %xmm8, %xmm3 #47.23
  mulpd %xmm8, %xmm8 #48.22
  addpd %xmm3, %xmm1 #47.9
  addpd %xmm6, %xmm8 #48.9
  movups 48048(%rsp,%rax,8), %xmm7 #47.16
  movups 32048(%rsp,%rax,8), %xmm3 #47.23
  addq $8, %rax #46.5
  mulpd %xmm3, %xmm7 #47.23
  mulpd %xmm3, %xmm3 #48.22
  addpd %xmm7, %xmm0 #47.9
  addpd %xmm8, %xmm3 #48.9
  cmpq $2000, %rax #46.5
  jb ..B1.22 # Prob 99% #46.5
..B1.23: # Preds ..B1.22
  addpd %xmm0, %xmm1 #45.16
  movaps %xmm1, %xmm0 #45.16
  movaps %xmm3, %xmm2 #45.24
  unpckhpd %xmm1, %xmm0 #45.16
  movl $.L_2__STRING.0, %edi #50.5
  unpckhpd %xmm3, %xmm2 #45.24
  movl $1, %eax #50.5
  addsd %xmm0, %xmm1 #45.16
  addsd %xmm2, %xmm3 #45.24
  divsd %xmm3, %xmm1 #50.5
  sqrtsd %xmm1, %xmm1 #50.5
  movaps %xmm1, %xmm0 #50.5
  call printf #50.5
..B1.24: # Preds ..B1.23
  xorl %eax, %eax #51.12
  movq %rbp, %rsp #51.12
  popq %rbp #51.12
  ret #51.12
.L_2il0floatpacket.0:
  .long 0x00000000,0x3ff00000,0x00000000,0x3ff00000
.L_2il0floatpacket.1:
  .long 0x00000002,0x00000002,0x00000002,0x00000002
.L_2il0floatpacket.2:
  .long 0xffffffff,0x00000000,0xffffffff,0x00000000
.L_2il0floatpacket.3:
  .long 0x00000001,0x00000001,0x00000001,0x00000001
.L_2__STRING.0:
  .long 1715023397
  .word 10
