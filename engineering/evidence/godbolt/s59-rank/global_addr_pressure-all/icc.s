main:
..B1.1: # Preds ..B1.0
  pushq %rbp #32.33
  movq %rsp, %rbp #32.33
  andq $-128, %rsp #32.33
  pushq %r15 #32.33
  pushq %rbx #32.33
  subq $112, %rsp #32.33
  movq %rsi, %r15 #32.33
  movl %edi, %ebx #32.33
  movl $3, %edi #32.33
  xorl %esi, %esi #32.33
  call __intel_new_feature_proc_init #32.33
..B1.12: # Preds ..B1.1
  stmxcsr (%rsp) #32.33
  orl $32832, (%rsp) #32.33
  ldmxcsr (%rsp) #32.33
  cmpl $1, %ebx #33.20
  jle ..B1.4 # Prob 50% #33.20
..B1.2: # Preds ..B1.12
  movq 8(%r15), %rdi #483.16
  call atol #483.16
..B1.3: # Preds ..B1.2
  movl %eax, %edi #483.16
  jmp ..B1.5 # Prob 100% #483.16
..B1.4: # Preds ..B1.12
  movl $2000, %edi #33.40
..B1.5: # Preds ..B1.3 ..B1.4
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #35.9
  xorl %eax, %eax #34.5
  movdqu .L_2il0floatpacket.1(%rip), %xmm3 #35.9
  movdqu .L_2il0floatpacket.2(%rip), %xmm4 #36.26
  movdqu .L_2il0floatpacket.3(%rip), %xmm2 #36.26
  movdqu .L_2il0floatpacket.4(%rip), %xmm0 #37.24
..B1.6: # Preds ..B1.6 ..B1.5
  movdqa %xmm3, %xmm6 #37.24
  movdqa %xmm3, %xmm5 #37.24
  movdqu %xmm3, perm(,%rax,4) #35.9
  psrlq $32, %xmm5 #37.24
  pmuludq %xmm3, %xmm6 #37.24
  paddd %xmm1, %xmm3 #35.9
  pmuludq %xmm5, %xmm5 #37.24
  movdqa %xmm3, %xmm8 #37.24
  movdqa %xmm3, %xmm7 #37.24
  movdqu %xmm3, 16+perm(,%rax,4) #35.9
  psrlq $32, %xmm7 #37.24
  pmuludq %xmm3, %xmm8 #37.24
  paddd %xmm1, %xmm3 #35.9
  pmuludq %xmm7, %xmm7 #37.24
  movdqa %xmm3, %xmm10 #37.24
  movdqa %xmm3, %xmm9 #37.24
  movdqu %xmm3, 32+perm(,%rax,4) #35.9
  psrlq $32, %xmm9 #37.24
  pmuludq %xmm3, %xmm10 #37.24
  paddd %xmm1, %xmm3 #35.9
  pmuludq %xmm9, %xmm9 #37.24
  movdqa %xmm3, %xmm12 #37.24
  movdqa %xmm3, %xmm11 #37.24
  movdqu %xmm3, 48+perm(,%rax,4) #35.9
  pand %xmm0, %xmm6 #37.24
  pmuludq %xmm3, %xmm12 #37.24
  paddd %xmm1, %xmm3 #35.9
  movdqa %xmm3, %xmm14 #37.24
  psllq $32, %xmm5 #37.24
  movdqu %xmm3, 64+perm(,%rax,4) #35.9
  movdqa %xmm3, %xmm13 #37.24
  pmuludq %xmm3, %xmm14 #37.24
  paddd %xmm1, %xmm3 #35.9
  por %xmm5, %xmm6 #37.24
  movdqa %xmm3, %xmm5 #37.24
  movdqu %xmm3, 80+perm(,%rax,4) #35.9
  pand %xmm0, %xmm8 #37.24
  pmuludq %xmm3, %xmm5 #37.24
  psllq $32, %xmm7 #37.24
  movdqa %xmm3, %xmm15 #37.24
  paddd %xmm1, %xmm3 #35.9
  por %xmm7, %xmm8 #37.24
  movdqa %xmm3, %xmm7 #37.24
  movdqu %xmm6, count(,%rax,4) #37.9
  movdqa %xmm3, %xmm6 #37.24
  movdqu %xmm3, 96+perm(,%rax,4) #35.9
  pand %xmm0, %xmm10 #37.24
  pmuludq %xmm3, %xmm7 #37.24
  paddd %xmm1, %xmm3 #35.9
  movdqu %xmm8, 16+count(,%rax,4) #37.9
  psllq $32, %xmm9 #37.24
  movdqa %xmm3, %xmm8 #37.24
  por %xmm9, %xmm10 #37.24
  psrlq $32, %xmm11 #37.24
  psrlq $32, %xmm13 #37.24
  psrlq $32, %xmm15 #37.24
  psrlq $32, %xmm6 #37.24
  movdqa %xmm3, %xmm9 #37.24
  psrlq $32, %xmm8 #37.24
  movdqu %xmm2, perm1(,%rax,4) #36.9
  paddd %xmm4, %xmm2 #36.26
  pmuludq %xmm11, %xmm11 #37.24
  pand %xmm0, %xmm12 #37.24
  pmuludq %xmm13, %xmm13 #37.24
  pmuludq %xmm15, %xmm15 #37.24
  pmuludq %xmm6, %xmm6 #37.24
  pmuludq %xmm3, %xmm9 #37.24
  pmuludq %xmm8, %xmm8 #37.24
  movdqu %xmm2, 16+perm1(,%rax,4) #36.9
  paddd %xmm4, %xmm2 #36.26
  movdqu %xmm2, 32+perm1(,%rax,4) #36.9
  paddd %xmm4, %xmm2 #36.26
  movdqu %xmm2, 48+perm1(,%rax,4) #36.9
  paddd %xmm4, %xmm2 #36.26
  movdqu %xmm2, 64+perm1(,%rax,4) #36.9
  paddd %xmm4, %xmm2 #36.26
  movdqu %xmm2, 80+perm1(,%rax,4) #36.9
  psllq $32, %xmm11 #37.24
  pand %xmm0, %xmm14 #37.24
  psllq $32, %xmm13 #37.24
  pand %xmm0, %xmm5 #37.24
  psllq $32, %xmm15 #37.24
  paddd %xmm4, %xmm2 #36.26
  pand %xmm0, %xmm7 #37.24
  psllq $32, %xmm6 #37.24
  pand %xmm0, %xmm9 #37.24
  psllq $32, %xmm8 #37.24
  por %xmm11, %xmm12 #37.24
  movdqu %xmm2, 96+perm1(,%rax,4) #36.9
  por %xmm13, %xmm14 #37.24
  por %xmm15, %xmm5 #37.24
  por %xmm6, %xmm7 #37.24
  paddd %xmm4, %xmm2 #36.26
  por %xmm8, %xmm9 #37.24
  movdqu %xmm10, 32+count(,%rax,4) #37.9
  movdqu %xmm12, 48+count(,%rax,4) #37.9
  movdqu %xmm14, 64+count(,%rax,4) #37.9
  movdqu %xmm5, 80+count(,%rax,4) #37.9
  movdqu %xmm7, 96+count(,%rax,4) #37.9
  movdqu %xmm3, 112+perm(,%rax,4) #35.9
  paddd %xmm1, %xmm3 #35.9
  movdqu %xmm2, 112+perm1(,%rax,4) #36.9
  paddd %xmm4, %xmm2 #36.26
  movdqu %xmm9, 112+count(,%rax,4) #37.9
  addq $32, %rax #34.5
  cmpq $128, %rax #34.5
  jb ..B1.6 # Prob 99% #34.5
..B1.7: # Preds ..B1.6
  call kernel #39.20
..B1.8: # Preds ..B1.7
  movl $.L_2__STRING.0, %edi #39.5
  movl %eax, %esi #39.5
  xorl %eax, %eax #39.5
  call printf #39.5
..B1.9: # Preds ..B1.8
  xorl %eax, %eax #40.12
  addq $112, %rsp #40.12
  popq %rbx #40.12
  popq %r15 #40.12
  movq %rbp, %rsp #40.12
  popq %rbp #40.12
  ret #40.12
kernel:
..B2.1: # Preds ..B2.0
  subq $40, %rsp #22.19
  xorl %eax, %eax #23.13
  xorl %edx, %edx #24.16
  testl %edi, %edi #24.25
  jle ..B2.8 # Prob 10% #24.25
..B2.2: # Preds ..B2.1
  movq %r12, 32(%rsp) #[spill]
  movq %r13, 24(%rsp) #[spill]
  movl %edi, %r13d #
  movq %r14, 16(%rsp) #[spill]
  movl %edx, %r14d #
  movq %r15, 8(%rsp) #[spill]
  movq %rbx, (%rsp) #[spill]
  movl %eax, %ebx #
..B2.3: # Preds ..B2.6 ..B2.2
  movl $perm, %edi #25.16
  movl $perm1, %esi #25.16
  movl $count, %edx #25.16
  movl $128, %ecx #25.16
  call mix #25.16
..B2.11: # Preds ..B2.3
  movl %eax, %r12d #25.16
..B2.4: # Preds ..B2.11
  movl $perm1, %edi #26.16
  movl $count, %esi #26.16
  movl $perm, %edx #26.16
  movl $128, %ecx #26.16
  call mix #26.16
..B2.12: # Preds ..B2.4
  movl %eax, %r15d #26.16
..B2.5: # Preds ..B2.12
  movl $count, %edi #27.16
  movl $perm, %esi #27.16
  movl $perm1, %edx #27.16
  movl $128, %ecx #27.16
  call mix #27.16
..B2.6: # Preds ..B2.5
  addl %ebx, %r12d #25.9
  incl %r14d #24.28
  addl %r15d, %r12d #26.9
  lea (%rax,%r12), %ebx #27.9
  cmpl %r13d, %r14d #24.25
  jl ..B2.3 # Prob 82% #24.25
..B2.7: # Preds ..B2.6
  movq 32(%rsp), %r12 #[spill]
  movl %ebx, %eax #
  movq 24(%rsp), %r13 #[spill]
  movq 16(%rsp), %r14 #[spill]
  movq 8(%rsp), %r15 #[spill]
  movq (%rsp), %rbx #[spill]
..B2.8: # Preds ..B2.7 ..B2.1
  addq $40, %rsp #29.12
  ret #29.12
mix:
..B3.1: # Preds ..B3.0
  movq %rdx, %rcx #12.47
  xorl %eax, %eax #13.11
  xorl %edx, %edx #14.5
..B3.2: # Preds ..B3.2 ..B3.1
  movl (%rdi,%rdx,8), %r8d #15.14
  addl (%rsi,%rdx,8), %r8d #15.21
  addl (%rcx,%rdx,8), %r8d #15.28
  addl %r8d, %eax #15.9
  movl %eax, (%rdi,%rdx,8) #16.9
  movl 4(%rdi,%rdx,8), %r9d #15.14
  addl 4(%rsi,%rdx,8), %r9d #15.21
  addl 4(%rcx,%rdx,8), %r9d #15.28
  addl %r9d, %eax #15.9
  movl %eax, 4(%rdi,%rdx,8) #16.9
  incq %rdx #14.5
  cmpq $64, %rdx #14.5
  jb ..B3.2 # Prob 63% #14.5
..B3.3: # Preds ..B3.2
  ret #18.12
perm:
count:
perm1:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2__STRING.0:
