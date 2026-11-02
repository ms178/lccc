main:
..B1.1: # Preds ..B1.0
  pushq %rbp #13.16
  movq %rsp, %rbp #13.16
  andq $-128, %rsp #13.16
  subq $128, %rsp #13.16
  movl $3, %edi #13.16
  xorl %esi, %esi #13.16
  call __intel_new_feature_proc_init #13.16
..B1.14: # Preds ..B1.1
  stmxcsr (%rsp) #13.16
  xorl %edx, %edx #14.5
  movl $16, %eax #16.9
  orl $32832, (%rsp) #13.16
  ldmxcsr (%rsp) #13.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm5 #15.27
  movdqu .L_2il0floatpacket.1(%rip), %xmm4 #15.27
  movdqu .L_2il0floatpacket.2(%rip), %xmm3 #15.27
  movdqu .L_2il0floatpacket.3(%rip), %xmm2 #15.27
  movdqu .L_2il0floatpacket.4(%rip), %xmm1 #15.27
  movdqu .L_2il0floatpacket.5(%rip), %xmm0 #16.35
..B1.2: # Preds ..B1.2 ..B1.14
  movdqa %xmm4, %xmm9 #16.35
  movdqa %xmm3, %xmm6 #16.35
  movdqa %xmm2, %xmm8 #16.35
  movdqa %xmm1, %xmm7 #16.35
  paddd %xmm5, %xmm4 #15.27
  paddd %xmm5, %xmm3 #15.27
  paddd %xmm5, %xmm2 #15.27
  paddd %xmm5, %xmm1 #15.27
  movdqa %xmm4, %xmm13 #16.35
  movdqa %xmm3, %xmm10 #16.35
  movdqa %xmm2, %xmm12 #16.35
  movdqa %xmm1, %xmm11 #16.35
  psrld $24, %xmm9 #16.35
  psrld $24, %xmm6 #16.35
  psrld $24, %xmm8 #16.35
  psrld $24, %xmm7 #16.35
  psrld $24, %xmm13 #16.35
  psrld $24, %xmm10 #16.35
  psrld $24, %xmm12 #16.35
  psrld $24, %xmm11 #16.35
  pslld $16, %xmm9 #16.35
  pslld $16, %xmm6 #16.35
  pslld $16, %xmm8 #16.35
  pslld $16, %xmm7 #16.35
  pslld $16, %xmm13 #16.35
  pslld $16, %xmm10 #16.35
  pslld $16, %xmm12 #16.35
  pslld $16, %xmm11 #16.35
  psrad $16, %xmm9 #16.35
  psrad $16, %xmm6 #16.35
  psrad $16, %xmm8 #16.35
  psrad $16, %xmm7 #16.35
  psrad $16, %xmm13 #16.35
  psrad $16, %xmm10 #16.35
  psrad $16, %xmm12 #16.35
  psrad $16, %xmm11 #16.35
  packssdw %xmm6, %xmm9 #16.35
  paddd %xmm5, %xmm4 #15.27
  packssdw %xmm7, %xmm8 #16.35
  pand %xmm0, %xmm9 #16.35
  packssdw %xmm10, %xmm13 #16.35
  pand %xmm0, %xmm8 #16.35
  packssdw %xmm11, %xmm12 #16.35
  pand %xmm0, %xmm13 #16.35
  pand %xmm0, %xmm12 #16.35
  paddd %xmm5, %xmm3 #15.27
  packuswb %xmm8, %xmm9 #16.35
  paddd %xmm5, %xmm2 #15.27
  movdqu %xmm9, bytes(%rdx) #16.9
  addl $32, %edx #14.5
  packuswb %xmm12, %xmm13 #16.35
  paddd %xmm5, %xmm1 #15.27
  movdqu %xmm13, bytes(%rax) #16.9
  addl $32, %eax #14.5
  cmpl $262144, %edx #14.5
  jb ..B1.2 # Prob 99% #14.5
..B1.3: # Preds ..B1.2
  xorl %eax, %eax #18.5
..B1.4: # Preds ..B1.4 ..B1.3
  lea (%rax,%rax), %edx #19.14
  movzbl bytes(%rdx), %ecx #19.14
  lea 1(%rax,%rax), %esi #19.14
  movzbl bytes(%rsi), %edi #19.14
  incl %eax #18.5
  incq bins(,%rcx,8) #19.9
  incq bins(,%rdi,8) #19.9
  cmpl $131072, %eax #18.5
  jb ..B1.4 # Prob 99% #18.5
..B1.5: # Preds ..B1.4
  xorl %eax, %eax #20.34
  xorl %edx, %edx #21.5
  pxor %xmm0, %xmm0 #20.20
..B1.6: # Preds ..B1.6 ..B1.5
  lea 2(%rdx), %ecx #22.18
  lea 4(%rdx), %esi #22.18
  lea 6(%rdx), %edi #22.18
  paddq bins(,%rdx,8), %xmm0 #22.9
  paddq bins(,%rcx,8), %xmm0 #22.9
  paddq bins(,%rsi,8), %xmm0 #22.9
  addl $8, %edx #21.5
  paddq bins(,%rdi,8), %xmm0 #22.9
  cmpl $256, %edx #21.5
  jb ..B1.6 # Prob 99% #21.5
..B1.7: # Preds ..B1.6
  xorl %edx, %edx #21.5
..B1.8: # Preds ..B1.8 ..B1.7
  imulq $131, %rax, %rax #23.31
  lea (%rdx,%rdx), %ecx #23.38
  addq bins(,%rcx,8), %rax #23.38
  lea 1(%rdx,%rdx), %esi #23.38
  imulq $131, %rax, %rax #23.31
  incl %edx #21.5
  addq bins(,%rsi,8), %rax #23.38
  cmpl $128, %edx #21.5
  jb ..B1.8 # Prob 99% #21.5
..B1.9: # Preds ..B1.8
  movdqa %xmm0, %xmm1 #20.20
  psrldq $8, %xmm1 #20.20
  paddq %xmm1, %xmm0 #20.20
  movq %xmm0, %rdx #20.20
  cmpq $262144, %rdx #25.18
  jne ..B1.11 # Prob 28% #25.18
..B1.10: # Preds ..B1.9
  movq $0x99779461c17bd852, %rdx #27.24
  movl $2, %ecx #27.24
  cmpq %rax, %rdx #27.24
  movl $0, %eax #27.24
  cmovne %ecx, %eax #27.24
  movq %rbp, %rsp #27.24
  popq %rbp #27.24
  ret #27.24
..B1.11: # Preds ..B1.9
  movl $1, %eax #26.16
  movq %rbp, %rsp #26.16
  popq %rbp #26.16
  ret #26.16
bytes:
bins:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.5:
