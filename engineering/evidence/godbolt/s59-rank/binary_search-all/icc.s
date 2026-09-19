main:
..B1.1: # Preds ..B1.0
  pushq %rbp #32.16
  movq %rsp, %rbp #32.16
  andq $-128, %rsp #32.16
  subq $128, %rsp #32.16
  movl $3, %edi #32.16
  xorl %esi, %esi #32.16
  call __intel_new_feature_proc_init #32.16
..B1.13: # Preds ..B1.1
  stmxcsr (%rsp) #32.16
  xorl %eax, %eax #13.5
  orl $32832, (%rsp) #32.16
  ldmxcsr (%rsp) #32.16
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #14.28
  movdqu .L_2il0floatpacket.1(%rip), %xmm0 #14.28
..B1.2: # Preds ..B1.2 ..B1.13
  movdqu %xmm0, table(,%rax,4) #14.9
  paddd %xmm1, %xmm0 #14.28
  movdqu %xmm0, 16+table(,%rax,4) #14.9
  paddd %xmm1, %xmm0 #14.28
  movdqu %xmm0, 32+table(,%rax,4) #14.9
  paddd %xmm1, %xmm0 #14.28
  movdqu %xmm0, 48+table(,%rax,4) #14.9
  addq $16, %rax #13.5
  paddd %xmm1, %xmm0 #14.28
  cmpq $4096, %rax #13.5
  jb ..B1.2 # Prob 99% #13.5
..B1.3: # Preds ..B1.2
  xorl %edx, %edx #34.18
  movl $-1000, %eax #35.18
..B1.4: # Preds ..B1.8 ..B1.3
  xorl %edi, %edi #18.12
  movl $4095, %esi #18.20
..B1.5: # Preds ..B1.6 ..B1.4
  movl %esi, %r8d #20.26
  subl %edi, %r8d #20.26
  sarl $1, %r8d #20.38
  lea (%rdi,%r8), %ecx #20.38
  movslq %ecx, %rcx #21.17
  movl table(,%rcx,4), %r9d #21.17
  cmpl %eax, %r9d #22.18
  je ..B1.10 # Prob 20% #22.18
..B1.6: # Preds ..B1.5
  lea 1(%rdi,%r8), %r8d #25.24
  cmovl %r8d, %edi #25.13
  decl %ecx #27.24
  cmpl %eax, %r9d #27.13
  cmovge %ecx, %esi #27.13
  cmpl %esi, %edi #19.18
  jle ..B1.5 # Prob 82% #19.18
..B1.8: # Preds ..B1.6 ..B1.10
  addl $7, %eax #35.47
  cmpl $13288, %eax #35.33
  jl ..B1.4 # Prob 99% #35.33
..B1.9: # Preds ..B1.8
  movl $2, %ecx #43.19
  xorl %eax, %eax #43.19
  cmpl $1198665, %edx #43.19
  cmovne %ecx, %eax #43.19
  movq %rbp, %rsp #43.19
  popq %rbp #43.19
  ret #43.19
..B1.10: # Preds ..B1.5
  testl %ecx, %ecx #40.13
  lea (%rdx,%rcx), %esi #40.13
  cmovns %esi, %edx #40.13
  jmp ..B1.8 # Prob 100% #40.13
table:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
