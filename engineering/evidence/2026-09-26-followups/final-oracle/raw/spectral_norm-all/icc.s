main:
..B1.1: # Preds ..B1.0
  pushq %rbp #37.16
  movq %rsp, %rbp #37.16
  andq $-128, %rsp #37.16
  subq $64000, %rsp #37.16
  movl $3, %edi #37.16
  xorl %esi, %esi #37.16
  call __intel_new_feature_proc_init #37.16
..B1.27: # Preds ..B1.1
  stmxcsr (%rsp) #37.16
  xorl %eax, %eax #40.5
  orl $32832, (%rsp) #37.16
  ldmxcsr (%rsp) #37.16
  movups .L_2il0floatpacket.0(%rip), %xmm0 #40.40
..B1.2: # Preds ..B1.2 ..B1.27
  movups %xmm0, 48000(%rsp,%rax,8) #40.33
  movups %xmm0, 48016(%rsp,%rax,8) #40.33
  movups %xmm0, 48032(%rsp,%rax,8) #40.33
  movups %xmm0, 48048(%rsp,%rax,8) #40.33
  addq $8, %rax #40.5
  cmpq $2000, %rax #40.5
  jb ..B1.2 # Prob 99% #40.5
..B1.3: # Preds ..B1.2
  movq $0x100000000, %rax #26.13
  xorb %dl, %dl #42.5
  pxor %xmm1, %xmm1 #15.20
  pxor %xmm4, %xmm4 #10.24
  movdqu .L_2il0floatpacket.1(%rip), %xmm6 #10.24
  movaps %xmm1, %xmm3 #15.20
  movdqu .L_2il0floatpacket.2(%rip), %xmm5 #10.38
  movdqu .L_2il0floatpacket.3(%rip), %xmm2 #10.43
  movq %rax, %xmm0 #26.13
  xorl %eax, %eax #26.13
..B1.4: # Preds ..B1.20 ..B1.3
  movl %eax, %esi #14.5
  xorl %ecx, %ecx #14.5
..B1.5: # Preds ..B1.7 ..B1.4
  movd %esi, %xmm11 #10.24
  lea 1(%rsi), %edi #10.24
  addl $2, %esi #10.38
  movdqa %xmm11, %xmm9 #10.24
  movaps %xmm3, %xmm10 #15.20
  movd %edi, %xmm8 #10.24
  movd %esi, %xmm7 #10.38
  xorl %esi, %esi #16.9
  punpckldq %xmm8, %xmm9 #10.24
  punpckldq %xmm7, %xmm8 #10.38
  punpcklqdq %xmm4, %xmm9 #10.24
  punpcklqdq %xmm4, %xmm8 #10.38
  pshufd $0, %xmm11, %xmm7 #18.9
..B1.6: # Preds ..B1.6 ..B1.5
  movdqa %xmm9, %xmm12 #10.38
  movdqa %xmm8, %xmm11 #10.38
  movdqa %xmm9, %xmm14 #10.38
  psrlq $32, %xmm12 #10.38
  psrlq $32, %xmm11 #10.38
  movdqa %xmm4, %xmm13 #10.43
  pmuludq %xmm8, %xmm14 #10.38
  paddd %xmm6, %xmm9 #10.24
  pmuludq %xmm11, %xmm12 #10.38
  pand %xmm5, %xmm14 #10.38
  psllq $32, %xmm12 #10.38
  por %xmm12, %xmm14 #10.38
  paddd %xmm6, %xmm8 #10.38
  pcmpgtd %xmm14, %xmm13 #10.43
  pand %xmm2, %xmm13 #10.51
  paddd %xmm13, %xmm14 #10.51
  psrad $1, %xmm14 #10.51
  paddd %xmm7, %xmm14 #10.51
  paddd %xmm2, %xmm14 #10.51
  cvtdq2pd %xmm14, %xmm15 #10.51
  movups 48000(%rsp,%rsi,8), %xmm11 #43.21
  addq $2, %rsi #16.9
  divpd %xmm15, %xmm11 #17.30
  addpd %xmm11, %xmm10 #17.13
  cmpq $2000, %rsi #16.9
  jb ..B1.6 # Prob 82% #16.9
..B1.7: # Preds ..B1.6
  movaps %xmm10, %xmm7 #15.20
  movl %edi, %esi #14.5
  unpckhpd %xmm10, %xmm7 #15.20
  addsd %xmm7, %xmm10 #15.20
  movsd %xmm10, (%rsp,%rcx,8) #33.18
  incq %rcx #10.24
  cmpl $2000, %edi #14.5
  jb ..B1.5 # Prob 91% #14.5
..B1.8: # Preds ..B1.7
  movl %eax, %esi #23.5
  xorl %ecx, %ecx #23.5
..B1.9: # Preds ..B1.11 ..B1.8
  movd %esi, %xmm8 #10.24
  lea 1(%rsi), %edi #10.24
  addl $2, %esi #10.38
  movaps %xmm3, %xmm9 #24.20
  movd %edi, %xmm7 #10.24
  movd %esi, %xmm10 #10.38
  xorl %esi, %esi #25.9
  punpckldq %xmm7, %xmm8 #10.24
  punpckldq %xmm10, %xmm7 #10.38
  movdqa %xmm0, %xmm10 #26.13
  punpcklqdq %xmm4, %xmm8 #10.24
  punpcklqdq %xmm4, %xmm7 #10.38
..B1.10: # Preds ..B1.10 ..B1.9
  movdqa %xmm8, %xmm12 #10.38
  movdqa %xmm7, %xmm11 #10.38
  movdqa %xmm8, %xmm14 #10.38
  psrlq $32, %xmm12 #10.38
  psrlq $32, %xmm11 #10.38
  movdqa %xmm4, %xmm13 #10.43
  pmuludq %xmm7, %xmm14 #10.38
  paddd %xmm6, %xmm8 #10.24
  pmuludq %xmm11, %xmm12 #10.38
  pand %xmm5, %xmm14 #10.38
  psllq $32, %xmm12 #10.38
  por %xmm12, %xmm14 #10.38
  paddd %xmm6, %xmm7 #10.38
  pcmpgtd %xmm14, %xmm13 #10.43
  pand %xmm2, %xmm13 #10.51
  paddd %xmm13, %xmm14 #10.51
  psrad $1, %xmm14 #10.51
  paddd %xmm10, %xmm14 #10.51
  paddd %xmm6, %xmm10 #26.13
  paddd %xmm2, %xmm14 #10.51
  cvtdq2pd %xmm14, %xmm15 #10.51
  movups (%rsp,%rsi,8), %xmm11 #34.16
  addq $2, %rsi #25.9
  divpd %xmm15, %xmm11 #26.30
  addpd %xmm11, %xmm9 #26.13
  cmpq $2000, %rsi #25.9
  jb ..B1.10 # Prob 82% #25.9
..B1.11: # Preds ..B1.10
  movaps %xmm9, %xmm7 #24.20
  movl %edi, %esi #23.5
  unpckhpd %xmm9, %xmm7 #24.20
  addsd %xmm7, %xmm9 #24.20
  movsd %xmm9, 32000(%rsp,%rcx,8) #43.24
  incq %rcx #10.24
  cmpl $2000, %edi #23.5
  jb ..B1.9 # Prob 91% #23.5
..B1.12: # Preds ..B1.11
  movl %eax, %esi #14.5
  xorl %ecx, %ecx #14.5
..B1.13: # Preds ..B1.15 ..B1.12
  movd %esi, %xmm11 #10.24
  lea 1(%rsi), %edi #10.24
  addl $2, %esi #10.38
  movdqa %xmm11, %xmm9 #10.24
  movaps %xmm3, %xmm10 #15.20
  movd %edi, %xmm8 #10.24
  movd %esi, %xmm7 #10.38
  xorl %esi, %esi #16.9
  punpckldq %xmm8, %xmm9 #10.24
  punpckldq %xmm7, %xmm8 #10.38
  punpcklqdq %xmm4, %xmm9 #10.24
  punpcklqdq %xmm4, %xmm8 #10.38
  pshufd $0, %xmm11, %xmm7 #18.9
..B1.14: # Preds ..B1.14 ..B1.13
  movdqa %xmm9, %xmm12 #10.38
  movdqa %xmm8, %xmm11 #10.38
  movdqa %xmm9, %xmm14 #10.38
  psrlq $32, %xmm12 #10.38
  psrlq $32, %xmm11 #10.38
  movdqa %xmm4, %xmm13 #10.43
  pmuludq %xmm8, %xmm14 #10.38
  paddd %xmm6, %xmm9 #10.24
  pmuludq %xmm11, %xmm12 #10.38
  pand %xmm5, %xmm14 #10.38
  psllq $32, %xmm12 #10.38
  por %xmm12, %xmm14 #10.38
  paddd %xmm6, %xmm8 #10.38
  pcmpgtd %xmm14, %xmm13 #10.43
  pand %xmm2, %xmm13 #10.51
  paddd %xmm13, %xmm14 #10.51
  psrad $1, %xmm14 #10.51
  paddd %xmm7, %xmm14 #10.51
  paddd %xmm2, %xmm14 #10.51
  cvtdq2pd %xmm14, %xmm15 #10.51
  movups 32000(%rsp,%rsi,8), %xmm11 #44.21
  addq $2, %rsi #16.9
  divpd %xmm15, %xmm11 #17.30
  addpd %xmm11, %xmm10 #17.13
  cmpq $2000, %rsi #16.9
  jb ..B1.14 # Prob 82% #16.9
..B1.15: # Preds ..B1.14
  movaps %xmm10, %xmm7 #15.20
  movl %edi, %esi #14.5
  unpckhpd %xmm10, %xmm7 #15.20
  addsd %xmm7, %xmm10 #15.20
  movsd %xmm10, 16000(%rsp,%rcx,8) #33.18
  incq %rcx #10.24
  cmpl $2000, %edi #14.5
  jb ..B1.13 # Prob 91% #14.5
..B1.16: # Preds ..B1.15
  movl %eax, %esi #23.5
  xorl %ecx, %ecx #23.5
..B1.17: # Preds ..B1.19 ..B1.16
  movd %esi, %xmm8 #10.24
  lea 1(%rsi), %edi #10.24
  addl $2, %esi #10.38
  movaps %xmm3, %xmm9 #24.20
  movd %edi, %xmm7 #10.24
  movd %esi, %xmm10 #10.38
  xorl %esi, %esi #25.9
  punpckldq %xmm7, %xmm8 #10.24
  punpckldq %xmm10, %xmm7 #10.38
  movdqa %xmm0, %xmm10 #26.13
  punpcklqdq %xmm4, %xmm8 #10.24
  punpcklqdq %xmm4, %xmm7 #10.38
..B1.18: # Preds ..B1.18 ..B1.17
  movdqa %xmm8, %xmm12 #10.38
  movdqa %xmm7, %xmm11 #10.38
  movdqa %xmm8, %xmm14 #10.38
  psrlq $32, %xmm12 #10.38
  psrlq $32, %xmm11 #10.38
  movdqa %xmm4, %xmm13 #10.43
  pmuludq %xmm7, %xmm14 #10.38
  paddd %xmm6, %xmm8 #10.24
  pmuludq %xmm11, %xmm12 #10.38
  pand %xmm5, %xmm14 #10.38
  psllq $32, %xmm12 #10.38
  por %xmm12, %xmm14 #10.38
  paddd %xmm6, %xmm7 #10.38
  pcmpgtd %xmm14, %xmm13 #10.43
  pand %xmm2, %xmm13 #10.51
  paddd %xmm13, %xmm14 #10.51
  psrad $1, %xmm14 #10.51
  paddd %xmm10, %xmm14 #10.51
  paddd %xmm6, %xmm10 #26.13
  paddd %xmm2, %xmm14 #10.51
  cvtdq2pd %xmm14, %xmm15 #10.51
  movups 16000(%rsp,%rsi,8), %xmm11 #34.16
  addq $2, %rsi #25.9
  divpd %xmm15, %xmm11 #26.30
  addpd %xmm11, %xmm9 #26.13
  cmpq $2000, %rsi #25.9
  jb ..B1.18 # Prob 82% #25.9
..B1.19: # Preds ..B1.18
  movaps %xmm9, %xmm7 #24.20
  movl %edi, %esi #23.5
  unpckhpd %xmm9, %xmm7 #24.20
  addsd %xmm7, %xmm9 #24.20
  movsd %xmm9, 48000(%rsp,%rcx,8) #44.24
  incq %rcx #10.24
  cmpl $2000, %edi #23.5
  jb ..B1.17 # Prob 91% #23.5
..B1.20: # Preds ..B1.19
  incb %dl #42.5
  cmpb $10, %dl #42.5
  jb ..B1.4 # Prob 99% #42.5
..B1.21: # Preds ..B1.20
  movaps %xmm3, %xmm0 #47.16
  xorl %eax, %eax #48.5
..B1.22: # Preds ..B1.22 ..B1.21
  movups 48000(%rsp,%rax,8), %xmm2 #49.16
  movups 32000(%rsp,%rax,8), %xmm4 #49.23
  mulpd %xmm4, %xmm2 #49.23
  mulpd %xmm4, %xmm4 #50.22
  addpd %xmm2, %xmm1 #49.9
  addpd %xmm4, %xmm3 #50.9
  movups 48016(%rsp,%rax,8), %xmm5 #49.16
  movups 32016(%rsp,%rax,8), %xmm6 #49.23
  mulpd %xmm6, %xmm5 #49.23
  mulpd %xmm6, %xmm6 #50.22
  addpd %xmm5, %xmm0 #49.9
  addpd %xmm3, %xmm6 #50.9
  movups 48032(%rsp,%rax,8), %xmm3 #49.16
  movups 32032(%rsp,%rax,8), %xmm8 #49.23
  mulpd %xmm8, %xmm3 #49.23
  mulpd %xmm8, %xmm8 #50.22
  addpd %xmm3, %xmm1 #49.9
  addpd %xmm6, %xmm8 #50.9
  movups 48048(%rsp,%rax,8), %xmm7 #49.16
  movups 32048(%rsp,%rax,8), %xmm3 #49.23
  addq $8, %rax #48.5
  mulpd %xmm3, %xmm7 #49.23
  mulpd %xmm3, %xmm3 #50.22
  addpd %xmm7, %xmm0 #49.9
  addpd %xmm8, %xmm3 #50.9
  cmpq $2000, %rax #48.5
  jb ..B1.22 # Prob 99% #48.5
..B1.23: # Preds ..B1.22
  addpd %xmm0, %xmm1 #47.16
  movaps %xmm1, %xmm0 #47.16
  movaps %xmm3, %xmm2 #47.24
  unpckhpd %xmm1, %xmm0 #47.16
  movl $.L_2__STRING.0, %edi #52.5
  unpckhpd %xmm3, %xmm2 #47.24
  movl $1, %eax #52.5
  addsd %xmm0, %xmm1 #47.16
  addsd %xmm2, %xmm3 #47.24
  divsd %xmm3, %xmm1 #52.5
  sqrtsd %xmm1, %xmm1 #52.5
  movaps %xmm1, %xmm0 #52.5
  call printf #52.5
..B1.24: # Preds ..B1.23
  xorl %eax, %eax #53.12
  movq %rbp, %rsp #53.12
  popq %rbp #53.12
  ret #53.12
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2__STRING.0:
