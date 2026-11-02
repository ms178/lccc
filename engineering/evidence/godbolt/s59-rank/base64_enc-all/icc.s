main:
..B1.1: # Preds ..B1.0
  pushq %rbp #22.16
  movq %rsp, %rbp #22.16
  andq $-128, %rsp #22.16
  subq $128, %rsp #22.16
  movl $3, %edi #22.16
  xorl %esi, %esi #22.16
  call __intel_new_feature_proc_init #22.16
..B1.15: # Preds ..B1.1
  stmxcsr (%rsp) #22.16
  xorl %eax, %eax #25.5
  orl $32832, (%rsp) #22.16
  ldmxcsr (%rsp) #22.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #25.67
  movdqu .L_2il0floatpacket.1(%rip), %xmm0 #25.67
..B1.2: # Preds ..B1.2 ..B1.15
  movdqu %xmm0, a.130.0.2(%rax) #25.35
  paddb %xmm1, %xmm0 #25.67
  movdqu %xmm0, 16+a.130.0.2(%rax) #25.35
  addq $32, %rax #25.5
  paddb %xmm1, %xmm0 #25.67
  cmpq $288, %rax #25.5
  jb ..B1.2 # Prob 99% #25.5
..B1.3: # Preds ..B1.2
  movl $452317355, %eax #25.67
  xorb %dl, %dl #25.5
  movdqu .L_2il0floatpacket.2(%rip), %xmm1 #25.67
  movd %eax, %xmm0 #25.67
  movl $a.130.0.2+288, %eax #25.35
..B1.4: # Preds ..B1.4 ..B1.3
  addb $4, %dl #25.5
  movd %xmm0, (%rax) #25.35
  addq $4, %rax #25.5
  paddb %xmm1, %xmm0 #25.67
  cmpb $12, %dl #25.5
  jb ..B1.4 # Prob 99% #25.5
..B1.5: # Preds ..B1.4
  xorl %ecx, %ecx #9.5
  xorl %edx, %edx #9.5
..B1.6: # Preds ..B1.6 ..B1.5
  movzbl a.130.0.2(%rdx), %esi #26.20
  lea (,%rcx,4), %eax #14.9
  movzbl 1+a.130.0.2(%rdx), %edi #26.20
  incl %ecx #9.5
  shll $16, %esi #10.40
  shll $8, %edi #12.49
  orl %edi, %esi #12.22
  movzbl 2+a.130.0.2(%rdx), %r8d #26.20
  orl %r8d, %esi #13.22
  movl %esi, %r11d #15.28
  movl %esi, %r8d #16.38
  shrl $12, %r11d #15.28
  movl %esi, %r9d #14.28
  shrl $6, %r8d #16.38
  addq $3, %rdx #9.5
  andq $63, %r11 #15.34
  andq $63, %rsi #17.36
  andq $63, %r8 #16.43
  shrl $18, %r9d #14.28
  movb tab(%r9), %r10b #14.18
  movb %r10b, d.130.0.2(%rax) #26.17
  movb tab(%r11), %dil #15.18
  movb tab(%r8), %r9b #16.28
  movb tab(%rsi), %r10b #17.28
  movb %dil, 1+d.130.0.2(%rax) #26.17
  movb %r9b, 2+d.130.0.2(%rax) #26.17
  movb %r10b, 3+d.130.0.2(%rax) #26.17
  cmpl $100, %ecx #9.5
  jb ..B1.6 # Prob 82% #9.5
..B1.7: # Preds ..B1.6
  movl $400, %esi #27.38
  xorl %eax, %eax #28.5
..B1.8: # Preds ..B1.8 ..B1.7
  movq %rsi, %rdx #28.41
  shlq $5, %rdx #28.41
  addq %rsi, %rdx #28.41
  movzbl d.130.0.2(,%rax,2), %esi #28.61
  addq %rsi, %rdx #28.61
  movq %rdx, %rsi #28.41
  shlq $5, %rsi #28.41
  addq %rdx, %rsi #28.41
  movzbl 1+d.130.0.2(,%rax,2), %ecx #28.61
  incq %rax #28.5
  addq %rcx, %rsi #28.61
  cmpq $200, %rax #28.5
  jb ..B1.8 # Prob 99% #28.5
..B1.9: # Preds ..B1.8
  lea 1(%rax,%rax), %eax #28.33
  lea -1(%rax), %edx #28.5
  cmpl $400, %edx #28.5
  jae ..B1.11 # Prob 10% #28.5
..B1.10: # Preds ..B1.9
  movq %rsi, %rdx #28.41
  shlq $5, %rdx #28.41
  addq %rsi, %rdx #28.41
  movzbl -1+d.130.0.2(%rax), %esi #28.61
  addq %rdx, %rsi #28.61
..B1.11: # Preds ..B1.10 ..B1.9
  movl $.L_2__STRING.0, %edi #29.5
  xorl %eax, %eax #29.5
  call printf #29.5
..B1.12: # Preds ..B1.11
  xorl %eax, %eax #30.12
  movq %rbp, %rsp #30.12
  popq %rbp #30.12
  ret #30.12
a.130.0.2:
d.130.0.2:
tab:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2__STRING.0:
