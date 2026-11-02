main:
..B1.1: # Preds ..B1.0
  pushq %rbp #23.16
  movq %rsp, %rbp #23.16
  andq $-128, %rsp #23.16
  subq $128, %rsp #23.16
  movl $3, %edi #23.16
  xorl %esi, %esi #23.16
  call __intel_new_feature_proc_init #23.16
..B1.15: # Preds ..B1.1
  stmxcsr (%rsp) #23.16
  xorl %eax, %eax #24.5
  xorl %edx, %edx #24.5
  orl $32832, (%rsp) #23.16
  xorl %ecx, %ecx #24.5
  ldmxcsr (%rsp) #23.16
  movdqu .L_2il0floatpacket.1(%rip), %xmm10 #26.13
  movdqu .L_2il0floatpacket.2(%rip), %xmm12 #26.13
  movdqu .L_2il0floatpacket.3(%rip), %xmm13 #26.13
  movdqu .L_2il0floatpacket.4(%rip), %xmm14 #26.13
  movdqu .L_2il0floatpacket.5(%rip), %xmm11 #26.13
  movdqu .L_2il0floatpacket.6(%rip), %xmm9 #26.62
..B1.2: # Preds ..B1.4 ..B1.15
  movd %ecx, %xmm0 #26.54
  lea 3(%rcx), %esi #26.54
  lea 6(%rcx), %edi #26.54
  lea 9(%rcx), %r8d #26.54
  lea 12(%rcx), %r9d #26.54
  movd %esi, %xmm1 #26.54
  lea 15(%rcx), %r10d #26.54
  movd %edi, %xmm3 #26.54
  lea 18(%rcx), %r11d #26.54
  movd %r8d, %xmm2 #26.54
  lea 21(%rcx), %esi #26.54
  punpckldq %xmm1, %xmm0 #26.54
  lea 24(%rcx), %edi #26.54
  movd %r9d, %xmm1 #26.54
  lea 27(%rcx), %r8d #26.54
  movd %r10d, %xmm4 #26.54
  movd %r11d, %xmm6 #26.54
  punpckldq %xmm2, %xmm3 #26.54
  movd %esi, %xmm5 #26.54
  movd %edi, %xmm2 #26.54
  lea 39(%rcx), %esi #26.54
  movd %r8d, %xmm7 #26.54
  lea 42(%rcx), %edi #26.54
  punpckldq %xmm4, %xmm1 #26.54
  lea 30(%rcx), %r9d #26.54
  punpckldq %xmm5, %xmm6 #26.54
  lea 33(%rcx), %r10d #26.54
  punpckldq %xmm7, %xmm2 #26.54
  lea 36(%rcx), %r11d #26.54
  punpcklqdq %xmm3, %xmm0 #26.54
  lea 45(%rcx), %r8d #26.54
  movd %r9d, %xmm15 #26.54
  movd %esi, %xmm4 #26.54
  movd %r10d, %xmm8 #26.54
  movdqa %xmm11, %xmm7 #26.13
  punpckldq %xmm8, %xmm15 #26.54
  movd %eax, %xmm8 #24.28
  punpcklqdq %xmm6, %xmm1 #26.54
  movd %r11d, %xmm3 #26.54
  punpcklqdq %xmm15, %xmm2 #26.54
  movd %edi, %xmm6 #26.54
  pshufd $0, %xmm8, %xmm15 #24.28
  movd %r8d, %xmm5 #26.54
  punpckldq %xmm4, %xmm3 #26.54
  movdqa %xmm15, %xmm8 #26.62
  punpckldq %xmm5, %xmm6 #26.54
  movdqa %xmm12, %xmm4 #26.13
  punpcklqdq %xmm6, %xmm3 #26.54
  movdqa %xmm13, %xmm5 #26.13
  movdqu %xmm15, (%rsp) #26.13[spill]
  movdqa %xmm14, %xmm6 #26.13
  xorb %dil, %dil #25.9
  movq %rdx, %rsi #26.13
  psrlq $32, %xmm8 #26.62
..B1.3: # Preds ..B1.3 ..B1.2
  movdqu (%rsp), %xmm13 #26.62[spill]
  movdqa %xmm4, %xmm15 #26.62
  movdqa %xmm5, %xmm11 #26.62
  movdqa %xmm13, %xmm14 #26.62
  psrlq $32, %xmm15 #26.62
  movdqa %xmm13, %xmm12 #26.62
  psrlq $32, %xmm11 #26.62
  addb $16, %dil #25.9
  pmuludq %xmm4, %xmm14 #26.62
  paddd %xmm10, %xmm4 #26.13
  pmuludq %xmm8, %xmm15 #26.62
  pmuludq %xmm5, %xmm12 #26.62
  pmuludq %xmm8, %xmm11 #26.62
  pand %xmm9, %xmm14 #26.62
  psllq $32, %xmm15 #26.62
  pand %xmm9, %xmm12 #26.62
  psllq $32, %xmm11 #26.62
  por %xmm15, %xmm14 #26.62
  por %xmm11, %xmm12 #26.62
  paddd %xmm0, %xmm14 #26.62
  paddd %xmm1, %xmm12 #26.62
  pslld $16, %xmm14 #26.62
  pslld $16, %xmm12 #26.62
  psrad $16, %xmm14 #26.62
  psrad $16, %xmm12 #26.62
  movdqa %xmm6, %xmm15 #26.62
  paddd %xmm10, %xmm5 #26.13
  packssdw %xmm12, %xmm14 #26.62
  movdqa %xmm13, %xmm12 #26.62
  psrlq $32, %xmm15 #26.62
  pmuludq %xmm6, %xmm12 #26.62
  paddd %xmm10, %xmm6 #26.13
  pmuludq %xmm8, %xmm15 #26.62
  pmuludq %xmm7, %xmm13 #26.62
  pand %xmm9, %xmm12 #26.62
  psllq $32, %xmm15 #26.62
  por %xmm15, %xmm12 #26.62
  movdqa %xmm7, %xmm15 #26.62
  psrlq $32, %xmm15 #26.62
  pand %xmm9, %xmm13 #26.62
  pmuludq %xmm8, %xmm15 #26.62
  paddd %xmm2, %xmm12 #26.62
  psllq $32, %xmm15 #26.62
  pslld $16, %xmm12 #26.62
  por %xmm15, %xmm13 #26.62
  psrad $16, %xmm12 #26.62
  paddd %xmm3, %xmm13 #26.62
  paddd %xmm10, %xmm7 #26.13
  pslld $16, %xmm13 #26.62
  psrad $16, %xmm13 #26.62
  movdqu .L_2il0floatpacket.7(%rip), %xmm11 #26.62
  packssdw %xmm13, %xmm12 #26.62
  pand %xmm11, %xmm14 #26.62
  pand %xmm11, %xmm12 #26.62
  packuswb %xmm12, %xmm14 #26.62
  movdqu %xmm14, img(%rsi) #26.13
  addq $16, %rsi #25.9
  movdqu .L_2il0floatpacket.0(%rip), %xmm14 #26.54
  paddd %xmm14, %xmm0 #26.54
  paddd %xmm14, %xmm1 #26.54
  paddd %xmm14, %xmm2 #26.54
  paddd %xmm14, %xmm3 #26.54
  cmpb $64, %dil #25.9
  jb ..B1.3 # Prob 98% #25.9
..B1.4: # Preds ..B1.3
  incl %eax #24.5
  addl $5, %ecx #24.5
  addq $64, %rdx #24.5
  movdqu .L_2il0floatpacket.5(%rip), %xmm11 #
  movdqu .L_2il0floatpacket.4(%rip), %xmm14 #
  movdqu .L_2il0floatpacket.3(%rip), %xmm13 #
  movdqu .L_2il0floatpacket.2(%rip), %xmm12 #
  cmpl $64, %eax #24.5
  jb ..B1.2 # Prob 98% #24.5
..B1.5: # Preds ..B1.4
  movdqu .L_2il0floatpacket.11(%rip), %xmm1 #15.61
  xorl %esi, %esi #10.5
  pcmpeqd %xmm7, %xmm7 #15.61
  psrlq $32, %xmm1 #15.61
  movdqu .L_2il0floatpacket.13(%rip), %xmm2 #15.61
  movdqa %xmm7, %xmm3 #15.61
  movdqu .L_2il0floatpacket.12(%rip), %xmm14 #15.61
  psrlq $32, %xmm3 #15.61
  psrlq $32, %xmm2 #15.61
  pxor %xmm13, %xmm13 #15.31
  psrlq $32, %xmm14 #15.61
  movl $255, %ecx #18.13
  movdqu .L_2il0floatpacket.14(%rip), %xmm12 #17.21
  movdqu %xmm2, 16(%rsp) #18.13[spill]
  movdqu %xmm1, 32(%rsp) #18.13[spill]
  movdqu %xmm3, 48(%rsp) #18.13[spill]
  movq %r12, (%rsp) #18.13[spill]
..B1.6: # Preds ..B1.8 ..B1.5
  movq %rsi, %r11 #15.31
  xorb %dl, %dl #11.9
  shlq $6, %r11 #15.31
  movq %r11, %rax #11.9
..B1.7: # Preds ..B1.7 ..B1.6
  movd img(%rax), %xmm7 #15.31
  addb $4, %dl #11.9
  movd 1+img(%rax), %xmm10 #15.31
  pcmpeqd %xmm2, %xmm2 #15.61
  movd 2+img(%rax), %xmm11 #15.31
  movdqa %xmm2, %xmm4 #15.61
  punpcklbw %xmm13, %xmm7 #15.31
  punpcklbw %xmm13, %xmm10 #15.31
  punpcklwd %xmm13, %xmm7 #15.31
  punpcklwd %xmm13, %xmm10 #15.31
  punpcklbw %xmm13, %xmm11 #15.31
  movdqu .L_2il0floatpacket.8(%rip), %xmm8 #15.61
  punpcklwd %xmm13, %xmm11 #15.31
  pmuludq %xmm7, %xmm4 #15.61
  psrlq $32, %xmm7 #15.61
  pmuludq %xmm10, %xmm8 #15.61
  pmuludq %xmm11, %xmm2 #15.61
  movdqu 48(%rsp), %xmm15 #15.61[spill]
  psrlq $32, %xmm10 #15.61
  pmuludq 32(%rsp), %xmm10 #15.61[spill]
  psrlq $32, %xmm11 #15.61
  pmuludq %xmm15, %xmm7 #15.61
  pmuludq %xmm15, %xmm11 #15.61
  movd 128+img(%rax), %xmm6 #15.31
  pand %xmm9, %xmm4 #15.61
  movd 129+img(%rax), %xmm3 #15.31
  psllq $32, %xmm7 #15.61
  punpcklbw %xmm13, %xmm6 #15.31
  pand %xmm9, %xmm8 #15.61
  movdqu .L_2il0floatpacket.9(%rip), %xmm15 #15.61
  psllq $32, %xmm10 #15.61
  punpcklwd %xmm13, %xmm6 #15.31
  movdqa %xmm15, %xmm0 #15.61
  punpcklbw %xmm13, %xmm3 #15.31
  por %xmm7, %xmm4 #15.61
  movd 130+img(%rax), %xmm5 #15.31
  por %xmm10, %xmm8 #15.61
  punpcklwd %xmm13, %xmm3 #15.31
  pand %xmm9, %xmm2 #15.61
  punpcklbw %xmm13, %xmm5 #15.31
  psllq $32, %xmm11 #15.61
  pmuludq %xmm6, %xmm0 #15.61
  psrlq $32, %xmm6 #15.61
  pmuludq %xmm14, %xmm6 #15.61
  movdqu .L_2il0floatpacket.10(%rip), %xmm1 #15.61
  paddd %xmm8, %xmm4 #15.21
  punpcklwd %xmm13, %xmm5 #15.31
  por %xmm11, %xmm2 #15.61
  pmuludq %xmm3, %xmm1 #15.61
  psrlq $32, %xmm3 #15.61
  pmuludq 16(%rsp), %xmm3 #15.61[spill]
  pmuludq %xmm5, %xmm15 #15.61
  psrlq $32, %xmm5 #15.61
  paddd %xmm2, %xmm4 #15.21
  pmuludq %xmm14, %xmm5 #15.61
  paddd %xmm13, %xmm4 #15.21
  paddd %xmm13, %xmm4 #15.21
  pand %xmm9, %xmm0 #15.61
  psllq $32, %xmm6 #15.61
  paddd %xmm13, %xmm4 #15.21
  por %xmm6, %xmm0 #15.61
  pand %xmm9, %xmm1 #15.61
  psllq $32, %xmm3 #15.61
  paddd %xmm0, %xmm4 #15.21
  por %xmm3, %xmm1 #15.61
  pand %xmm9, %xmm15 #15.61
  psllq $32, %xmm5 #15.61
  paddd %xmm1, %xmm4 #15.21
  por %xmm5, %xmm15 #15.61
  movdqa %xmm13, %xmm0 #16.13
  paddd %xmm15, %xmm4 #15.21
  movdqa %xmm12, %xmm1 #17.13
  pcmpgtd %xmm4, %xmm0 #16.13
  pxor %xmm0, %xmm4 #16.13
  psubd %xmm0, %xmm4 #16.13
  pcmpgtd %xmm4, %xmm1 #17.13
  pxor %xmm12, %xmm4 #18.40
  pand %xmm4, %xmm1 #18.40
  pxor %xmm12, %xmm1 #18.40
  pslld $16, %xmm1 #18.40
  psrad $16, %xmm1 #18.40
  packssdw %xmm13, %xmm1 #18.40
  pand .L_2il0floatpacket.7(%rip), %xmm1 #18.40
  packuswb %xmm13, %xmm1 #18.40
  movd %xmm1, 65+out(%rax) #18.13
  addq $4, %rax #11.9
  cmpb $60, %dl #11.9
  jb ..B1.7 # Prob 98% #11.9
..B1.8: # Preds ..B1.7
  movzbl 61+img(%r11), %edi #15.31
  incq %rsi #10.5
  movzbl 188+img(%r11), %r12d #15.31
  movzbl 189+img(%r11), %r8d #15.31
  movzbl 62+img(%r11), %r10d #15.31
  lea (%rdi,%rdi), %edx #15.61
  negl %edx #15.61
  addl %r12d, %edx #15.21
  movzbl 60+img(%r11), %eax #15.31
  movzbl 190+img(%r11), %r9d #15.31
  addl %r10d, %eax #15.21
  negl %eax #15.21
  addl %r10d, %r10d #15.61
  negl %r10d #15.61
  lea (%rdx,%r8,2), %r12d #15.21
  addl %r9d, %r12d #15.21
  addl %r8d, %r10d #15.21
  addl %r12d, %eax #15.21
  cltd #16.13
  xorl %edx, %eax #16.13
  lea (%r10,%r9,2), %r9d #15.21
  subl %edx, %eax #16.13
  movzbl 63+img(%r11), %edx #15.31
  addl %edx, %edi #15.21
  movzbl 191+img(%r11), %r8d #15.31
  cmpl $255, %eax #18.13
  cmovg %ecx, %eax #18.13
  negl %edi #15.21
  addl %r8d, %r9d #15.21
  addl %r9d, %edi #15.21
  movb %al, 125+out(%r11) #18.13
  movl %edi, %eax #16.13
  cltd #16.13
  xorl %edx, %edi #16.13
  subl %edx, %edi #16.13
  cmpl $255, %edi #18.13
  cmovg %ecx, %edi #18.13
  movb %dil, 126+out(%r11) #18.13
  cmpq $62, %rsi #10.5
  jb ..B1.6 # Prob 98% #10.5
..B1.9: # Preds ..B1.8
  xorl %esi, %esi #28.26
  movq (%rsp), %r12 #[spill]
  xorl %eax, %eax #29.5
..B1.10: # Preds ..B1.10 ..B1.9
  movq %rsi, %rdx #30.45
  shlq $5, %rdx #30.45
  subq %rsi, %rdx #30.45
  movzbl out(,%rax,2), %esi #30.50
  addq %rsi, %rdx #30.50
  movq %rdx, %rsi #30.45
  shlq $5, %rsi #30.45
  subq %rdx, %rsi #30.45
  movzbl 1+out(,%rax,2), %ecx #30.50
  incq %rax #29.5
  addq %rcx, %rsi #30.50
  cmpq $2048, %rax #29.5
  jb ..B1.10 # Prob 99% #29.5
..B1.11: # Preds ..B1.10
  movl $.L_2__STRING.0, %edi #31.5
  xorl %eax, %eax #31.5
  call printf #31.5
..B1.12: # Preds ..B1.11
  xorl %eax, %eax #32.12
  movq %rbp, %rsp #32.12
  popq %rbp #32.12
  ret #32.12
img:
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
.L_2il0floatpacket.11:
.L_2il0floatpacket.12:
.L_2il0floatpacket.13:
.L_2il0floatpacket.14:
.L_2__STRING.0:
