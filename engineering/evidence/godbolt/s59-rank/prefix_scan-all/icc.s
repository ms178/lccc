main:
..B1.1: # Preds ..B1.0
  pushq %rbp #12.16
  movq %rsp, %rbp #12.16
  andq $-128, %rsp #12.16
  subq $128, %rsp #12.16
  movl $3, %edi #12.16
  xorl %esi, %esi #12.16
  call __intel_new_feature_proc_init #12.16
..B1.11: # Preds ..B1.1
  stmxcsr (%rsp) #12.16
  xorl %eax, %eax #15.5
  orl $32832, (%rsp) #12.16
  ldmxcsr (%rsp) #12.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #15.69
  movdqu .L_2il0floatpacket.1(%rip), %xmm0 #15.69
..B1.2: # Preds ..B1.2 ..B1.11
  movdqu %xmm0, a.126.0.2(,%rax,2) #15.36
  paddw %xmm1, %xmm0 #15.69
  movdqu %xmm0, 16+a.126.0.2(,%rax,2) #15.36
  paddw %xmm1, %xmm0 #15.69
  movdqu %xmm0, 32+a.126.0.2(,%rax,2) #15.36
  paddw %xmm1, %xmm0 #15.69
  movdqu %xmm0, 48+a.126.0.2(,%rax,2) #15.36
  addq $32, %rax #15.5
  paddw %xmm1, %xmm0 #15.69
  cmpq $1024, %rax #15.5
  jb ..B1.2 # Prob 99% #15.5
..B1.3: # Preds ..B1.2
  xorl %edx, %edx #5.16
  xorl %eax, %eax #6.5
..B1.4: # Preds ..B1.4 ..B1.3
  lea (%rax,%rax), %ecx #7.14
  incl %eax #6.5
  movzwl a.126.0.2(,%rcx,2), %esi #16.13
  addl %esi, %edx #7.9
  movzwl 2+a.126.0.2(,%rcx,2), %edi #16.13
  movl %edx, d.126.0.2(,%rcx,4) #16.10
  addl %edi, %edx #7.9
  movl %edx, 4+d.126.0.2(,%rcx,4) #16.10
  cmpl $512, %eax #6.5
  jb ..B1.4 # Prob 63% #6.5
..B1.5: # Preds ..B1.4
  movl d.126.0.2(%rip), %ecx #17.28
  xorb %dl, %dl #18.5
  addl 2044+d.126.0.2(%rip), %ecx #17.35
  xorl %eax, %eax #18.5
  addl 4092+d.126.0.2(%rip), %ecx #17.44
..B1.6: # Preds ..B1.6 ..B1.5
  movq %rcx, %rsi #18.47
  incb %dl #18.5
  shlq $5, %rsi #18.47
  addq %rcx, %rsi #18.47
  movl d.126.0.2(%rax), %ecx #18.52
  addq %rcx, %rsi #18.52
  movq %rsi, %rcx #18.47
  shlq $5, %rcx #18.47
  addq %rsi, %rcx #18.47
  movl 28+d.126.0.2(%rax), %edi #18.52
  addq %rdi, %rcx #18.52
  addq $56, %rax #18.5
  cmpb $73, %dl #18.5
  jb ..B1.6 # Prob 98% #18.5
..B1.7: # Preds ..B1.6
  movq %rcx, %rsi #19.5
  movl $.L_2__STRING.0, %edi #19.5
  shlq $5, %rsi #19.5
  xorl %eax, %eax #19.5
  addq %rcx, %rsi #19.5
  movl 4088+d.126.0.2(%rip), %ecx #19.5
  addq %rcx, %rsi #19.5
  call printf #19.5
..B1.8: # Preds ..B1.7
  xorl %eax, %eax #20.12
  movq %rbp, %rsp #20.12
  popq %rbp #20.12
  ret #20.12
a.126.0.2:
d.126.0.2:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2__STRING.0:
