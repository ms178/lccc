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
  vstmxcsr (%rsp) #13.16
  vmovdqu .L_2il0floatpacket.0(%rip), %ymm5 #15.27
  vmovdqu .L_2il0floatpacket.1(%rip), %ymm0 #15.27
  vmovdqu .L_2il0floatpacket.2(%rip), %ymm3 #15.27
  vmovdqu .L_2il0floatpacket.3(%rip), %ymm2 #15.27
  vmovdqu .L_2il0floatpacket.4(%rip), %ymm1 #15.27
  vmovdqu .L_2il0floatpacket.5(%rip), %ymm6 #16.35
  vmovdqu .L_2il0floatpacket.6(%rip), %ymm7 #16.35
  vmovdqu .L_2il0floatpacket.8(%rip), %ymm4 #16.35
  orl $32832, (%rsp) #13.16
  xorl %edx, %edx #14.5
  vldmxcsr (%rsp) #13.16
  movl $32, %eax #16.9
..B1.2: # Preds ..B1.2 ..B1.14
  vpsrld $24, %ymm0, %ymm9 #16.35
  vpsrld $24, %ymm3, %ymm13 #16.35
  vpsrld $24, %ymm2, %ymm12 #16.35
  vpsrld $24, %ymm1, %ymm11 #16.35
  vpaddd %ymm5, %ymm0, %ymm0 #15.27
  vpaddd %ymm5, %ymm3, %ymm3 #15.27
  vpaddd %ymm5, %ymm2, %ymm2 #15.27
  vpaddd %ymm5, %ymm1, %ymm1 #15.27
  vpshufb %ymm6, %ymm9, %ymm15 #16.35
  vpshufb %ymm7, %ymm13, %ymm8 #16.35
  vpshufb %ymm6, %ymm12, %ymm9 #16.35
  vpshufb %ymm7, %ymm11, %ymm10 #16.35
  vperm2i128 $129, %ymm15, %ymm15, %ymm14 #16.35
  vperm2i128 $129, %ymm8, %ymm8, %ymm11 #16.35
  vperm2i128 $40, %ymm9, %ymm9, %ymm12 #16.35
  vpand %ymm4, %ymm9, %ymm9 #16.35
  vperm2i128 $40, %ymm10, %ymm10, %ymm13 #16.35
  vpor %ymm12, %ymm9, %ymm12 #16.35
  vpand %ymm10, %ymm4, %ymm10 #16.35
  vpor %ymm13, %ymm10, %ymm9 #16.35
  vpsrld $24, %ymm2, %ymm10 #16.35
  vpaddd %ymm5, %ymm2, %ymm2 #15.27
  vmovaps %xmm15, %xmm15 #16.35
  vmovaps %xmm8, %xmm8 #16.35
  vpor %ymm15, %ymm14, %ymm15 #16.35
  vpor %ymm8, %ymm11, %ymm14 #16.35
  vpsrld $24, %ymm1, %ymm8 #16.35
  vpaddd %ymm5, %ymm1, %ymm1 #15.27
  vpor %ymm14, %ymm15, %ymm15 #16.35
  vpor %ymm12, %ymm15, %ymm11 #16.35
  vpsrld $24, %ymm0, %ymm12 #16.35
  vpsrld $24, %ymm3, %ymm15 #16.35
  vpaddd %ymm5, %ymm0, %ymm0 #15.27
  vpor %ymm9, %ymm11, %ymm13 #16.35
  vpaddd %ymm5, %ymm3, %ymm3 #15.27
  vpshufb %ymm6, %ymm12, %ymm11 #16.35
  vpshufb %ymm7, %ymm15, %ymm14 #16.35
  vpshufb %ymm6, %ymm10, %ymm15 #16.35
  vmovdqu %ymm13, bytes(%rdx) #16.9
  vpshufb %ymm7, %ymm8, %ymm13 #16.35
  vperm2i128 $129, %ymm11, %ymm11, %ymm8 #16.35
  addl $64, %edx #14.5
  vperm2i128 $129, %ymm14, %ymm14, %ymm10 #16.35
  vperm2i128 $40, %ymm15, %ymm15, %ymm12 #16.35
  vpand %ymm4, %ymm15, %ymm15 #16.35
  vperm2i128 $40, %ymm13, %ymm13, %ymm9 #16.35
  vpand %ymm13, %ymm4, %ymm13 #16.35
  vpor %ymm9, %ymm13, %ymm9 #16.35
  vmovaps %xmm11, %xmm11 #16.35
  vmovaps %xmm14, %xmm14 #16.35
  vpor %ymm11, %ymm8, %ymm11 #16.35
  vpor %ymm14, %ymm10, %ymm8 #16.35
  vpor %ymm8, %ymm11, %ymm10 #16.35
  vpor %ymm12, %ymm15, %ymm11 #16.35
  vpor %ymm11, %ymm10, %ymm12 #16.35
  vpor %ymm9, %ymm12, %ymm8 #16.35
  vmovdqu %ymm8, bytes(%rax) #16.9
  addl $64, %eax #14.5
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
  vpxor %ymm1, %ymm1, %ymm1 #20.20
  vmovdqa %ymm1, %ymm0 #20.20
..B1.6: # Preds ..B1.6 ..B1.5
  lea 4(%rdx), %ecx #22.18
  vpaddq bins(,%rcx,8), %ymm0, %ymm0 #22.9
  vpaddq bins(,%rdx,8), %ymm1, %ymm1 #22.9
  addl $8, %edx #21.5
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
  vpaddq %ymm0, %ymm1, %ymm0 #20.20
  vextractf128 $1, %ymm0, %xmm1 #20.20
  vpaddq %xmm1, %xmm0, %xmm2 #20.20
  vpsrldq $8, %xmm2, %xmm3 #20.20
  vpaddq %xmm3, %xmm2, %xmm4 #20.20
  vmovq %xmm4, %rdx #20.20
  cmpq $262144, %rdx #25.18
  jne ..B1.11 # Prob 28% #25.18
..B1.10: # Preds ..B1.9
  movq $0x99779461c17bd852, %rdx #27.24
  movl $2, %ecx #27.24
  cmpq %rax, %rdx #27.24
  movl $0, %eax #27.24
  cmovne %ecx, %eax #27.24
  vzeroupper #27.24
  movq %rbp, %rsp #27.24
  popq %rbp #27.24
  ret #27.24
..B1.11: # Preds ..B1.9
  vzeroupper #26.16
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
.L_2il0floatpacket.6:
.L_2il0floatpacket.8:
