main:
..B1.1: # Preds ..B1.0
  pushq %rbp #34.16
  movq %rsp, %rbp #34.16
  andq $-128, %rsp #34.16
  subq $128, %rsp #34.16
  movl $3, %edi #34.16
  xorl %esi, %esi #34.16
  call __intel_new_feature_proc_init #34.16
..B1.12: # Preds ..B1.1
  stmxcsr (%rsp) #34.16
  xorl %eax, %eax #16.5
  orl $32832, (%rsp) #34.16
  ldmxcsr (%rsp) #34.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm4 #17.41
  movdqu .L_2il0floatpacket.1(%rip), %xmm11 #17.41
  movdqu .L_2il0floatpacket.2(%rip), %xmm10 #17.41
  movdqu .L_2il0floatpacket.3(%rip), %xmm9 #17.41
  movdqu .L_2il0floatpacket.4(%rip), %xmm7 #17.41
  movdqu .L_2il0floatpacket.6(%rip), %xmm6 #18.40
  movdqu .L_2il0floatpacket.7(%rip), %xmm5 #18.47
  movups .L_2il0floatpacket.8(%rip), %xmm8 #18.40
  movups .L_2il0floatpacket.10(%rip), %xmm2 #18.40
..B1.2: # Preds ..B1.2 ..B1.12
  movdqa %xmm11, %xmm3 #18.40
  movaps %xmm8, %xmm14 #18.40
  psrld $16, %xmm3 #18.40
  movaps %xmm2, %xmm15 #18.40
  pshufd $49, %xmm3, %xmm12 #18.40
  movdqa %xmm3, %xmm13 #18.40
  pmuludq %xmm3, %xmm14 #18.40
  paddd %xmm4, %xmm11 #17.41
  pmuludq %xmm8, %xmm12 #18.40
  pand .L_2il0floatpacket.9(%rip), %xmm12 #18.40
  psrlq $32, %xmm14 #18.40
  por %xmm12, %xmm14 #18.40
  psubd %xmm14, %xmm13 #18.40
  psrld $1, %xmm13 #18.40
  paddd %xmm13, %xmm14 #18.40
  movaps %xmm8, %xmm13 #18.40
  psrld $6, %xmm14 #18.40
  pshufd $49, %xmm14, %xmm1 #18.40
  pmuludq %xmm14, %xmm15 #18.40
  movaps %xmm2, %xmm14 #18.40
  pmuludq %xmm2, %xmm1 #18.40
  pand .L_2il0floatpacket.11(%rip), %xmm15 #18.40
  psllq $32, %xmm1 #18.40
  por %xmm15, %xmm1 #18.40
  movdqa %xmm10, %xmm15 #18.40
  psrld $16, %xmm15 #18.40
  psubd %xmm1, %xmm3 #18.40
  pshufd $49, %xmm15, %xmm0 #18.40
  movdqa %xmm15, %xmm12 #18.40
  pmuludq %xmm15, %xmm13 #18.40
  pslld $16, %xmm3 #18.40
  pmuludq %xmm8, %xmm0 #18.40
  pand .L_2il0floatpacket.9(%rip), %xmm0 #18.40
  psrlq $32, %xmm13 #18.40
  por %xmm0, %xmm13 #18.40
  psrad $16, %xmm3 #18.40
  psubd %xmm13, %xmm12 #18.40
  paddd %xmm4, %xmm10 #17.41
  psrld $1, %xmm12 #18.40
  paddd %xmm12, %xmm13 #18.40
  psrld $6, %xmm13 #18.40
  pshufd $49, %xmm13, %xmm1 #18.40
  pmuludq %xmm13, %xmm14 #18.40
  movaps %xmm8, %xmm13 #18.40
  pmuludq %xmm2, %xmm1 #18.40
  pand .L_2il0floatpacket.11(%rip), %xmm14 #18.40
  psllq $32, %xmm1 #18.40
  por %xmm14, %xmm1 #18.40
  psubd %xmm1, %xmm15 #18.40
  movdqa %xmm9, %xmm1 #18.40
  psrld $16, %xmm1 #18.40
  pslld $16, %xmm15 #18.40
  pshufd $49, %xmm1, %xmm0 #18.40
  movdqa %xmm1, %xmm12 #18.40
  pmuludq %xmm1, %xmm13 #18.40
  psrad $16, %xmm15 #18.40
  pmuludq %xmm8, %xmm0 #18.40
  pand .L_2il0floatpacket.9(%rip), %xmm0 #18.40
  psrlq $32, %xmm13 #18.40
  por %xmm0, %xmm13 #18.40
  movaps %xmm2, %xmm0 #18.40
  psubd %xmm13, %xmm12 #18.40
  paddd %xmm4, %xmm9 #17.41
  psrld $1, %xmm12 #18.40
  paddd %xmm12, %xmm13 #18.40
  psrld $6, %xmm13 #18.40
  pshufd $49, %xmm13, %xmm12 #18.40
  pmuludq %xmm13, %xmm0 #18.40
  movaps %xmm2, %xmm13 #18.40
  pmuludq %xmm2, %xmm12 #18.40
  pand .L_2il0floatpacket.11(%rip), %xmm0 #18.40
  psllq $32, %xmm12 #18.40
  por %xmm0, %xmm12 #18.40
  movdqa %xmm7, %xmm0 #18.40
  psrld $16, %xmm0 #18.40
  psubd %xmm12, %xmm1 #18.40
  pshufd $49, %xmm0, %xmm14 #18.40
  movaps %xmm8, %xmm12 #18.40
  pmuludq %xmm0, %xmm12 #18.40
  pslld $16, %xmm1 #18.40
  pmuludq %xmm8, %xmm14 #18.40
  pand .L_2il0floatpacket.9(%rip), %xmm14 #18.40
  psrlq $32, %xmm12 #18.40
  packssdw %xmm15, %xmm3 #18.40
  por %xmm14, %xmm12 #18.40
  movdqa %xmm0, %xmm15 #18.40
  psrad $16, %xmm1 #18.40
  psubd %xmm12, %xmm15 #18.40
  pand %xmm6, %xmm3 #18.40
  psrld $1, %xmm15 #18.40
  paddd %xmm4, %xmm7 #17.41
  paddd %xmm15, %xmm12 #18.40
  psrld $6, %xmm12 #18.40
  pshufd $49, %xmm12, %xmm14 #18.40
  pmuludq %xmm12, %xmm13 #18.40
  pmuludq %xmm2, %xmm14 #18.40
  pand .L_2il0floatpacket.11(%rip), %xmm13 #18.40
  psllq $32, %xmm14 #18.40
  por %xmm13, %xmm14 #18.40
  psubd %xmm14, %xmm0 #18.40
  pslld $16, %xmm0 #18.40
  psrad $16, %xmm0 #18.40
  packssdw %xmm0, %xmm1 #18.40
  pand %xmm6, %xmm1 #18.40
  packuswb %xmm1, %xmm3 #18.40
  paddb %xmm5, %xmm3 #18.47
  movdqu %xmm3, data(%rax) #18.9
  addl $16, %eax #16.5
  cmpl $65536, %eax #16.5
  jb ..B1.2 # Prob 99% #16.5
..B1.3: # Preds ..B1.2
  xorl %edx, %edx #23.18
  xorl %eax, %eax #24.5
..B1.4: # Preds ..B1.4 ..B1.3
  movzbl data(,%rax,2), %edi #25.22
  movl %edx, %r9d #29.23
  movzbl 1+data(,%rax,2), %r10d #25.22
  shll $5, %r9d #29.23
  addl %edx, %r9d #29.28
  lea -65(%rdi), %ecx #26.18
  cmpl $25, %ecx #27.13
  lea 32(%rdi), %esi #27.13
  lea -65(%r10), %edx #26.18
  cmovbe %esi, %edi #27.13
  cmpl $25, %edx #27.13
  movb %dil, folded(,%rax,2) #28.9
  lea 32(%r10), %r8d #27.13
  cmovbe %r8d, %r10d #27.13
  addl %edi, %r9d #29.34
  movl %r9d, %edx #29.23
  shll $5, %edx #29.23
  addl %r9d, %edx #29.28
  movb %r10b, 1+folded(,%rax,2) #28.9
  incq %rax #24.5
  addl %r10d, %edx #29.34
  cmpq $32768, %rax #24.5
  jb ..B1.4 # Prob 99% #24.5
..B1.5: # Preds ..B1.4
  cmpb $32, folded(%rip) #37.22
  jne ..B1.8 # Prob 67% #37.22
..B1.6: # Preds ..B1.5
  cmpb $55, 1+folded(%rip) #37.42
  jne ..B1.8 # Prob 67% #37.42
..B1.7: # Preds ..B1.6
  cmpb $110, 2+folded(%rip) #37.62
  je ..B1.9 # Prob 32% #37.62
..B1.8: # Preds ..B1.5 ..B1.6 ..B1.7
  movl $1, %eax #38.16
  movq %rbp, %rsp #38.16
  popq %rbp #38.16
  ret #38.16
..B1.9: # Preds ..B1.7
  movl $2, %ecx #39.18
  xorl %eax, %eax #39.18
  cmpl $-1723667183, %edx #39.18
  cmovne %ecx, %eax #39.18
  movq %rbp, %rsp #39.18
  popq %rbp #39.18
  ret #39.18
data:
folded:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2il0floatpacket.6:
.L_2il0floatpacket.7:
.L_2il0floatpacket.8:
.L_2il0floatpacket.9:
.L_2il0floatpacket.10:
.L_2il0floatpacket.11:
