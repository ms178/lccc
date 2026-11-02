main:
..B1.1: # Preds ..B1.0
  pushq %rbp #30.16
  movq %rsp, %rbp #30.16
  andq $-128, %rsp #30.16
  subq $128, %rsp #30.16
  movl $3, %edi #30.16
  xorl %esi, %esi #30.16
  call __intel_new_feature_proc_init #30.16
..B1.42: # Preds ..B1.1
  stmxcsr (%rsp) #30.16
  xorl %esi, %esi #31.14
  movl $777, %edx #32.23
  orl $32832, (%rsp) #30.16
  xorl %eax, %eax #33.16
  ldmxcsr (%rsp) #30.16
..B1.2: # Preds ..B1.37 ..B1.42
  imull $1664525, %edx, %edx #34.23
  addl $1013904223, %edx #34.34
  movl %edx, %ecx #35.27
  shrl $16, %ecx #35.27
  movzwl %dx, %edi #36.45
  andl $15, %ecx #35.33
..B1.3: # Preds ..B1.2
  jmp *.2.5_2.switchtab.6(,%rcx,8) #9.5
..1.1_0.TAG.15.0.1.3:
..B1.5: # Preds ..B1.3
  lea 1(%rax,%rdi), %edi #33.5
  jmp ..B1.37 # Prob 100% #33.5
..1.1_0.TAG.14.0.1.3:
..B1.7: # Preds ..B1.3
  andl %eax, %edi #24.30
  addl $2, %edi #24.35
  jmp ..B1.37 # Prob 100% #24.35
..1.1_0.TAG.13.0.1.3:
..B1.9: # Preds ..B1.3
  orl %eax, %edi #23.30
  decl %edi #23.35
  jmp ..B1.37 # Prob 100% #23.35
..1.1_0.TAG.12.0.1.3:
..B1.11: # Preds ..B1.3
  xorl %eax, %edi #22.30
  incl %edi #22.35
  jmp ..B1.37 # Prob 100% #22.35
..1.1_0.TAG.11.0.1.3:
..B1.13: # Preds ..B1.3
  negl %edi #21.26
  addl %eax, %edi #21.26
  lea (%rdi,%rdi,4), %edi #21.35
  jmp ..B1.37 # Prob 100% #21.35
..1.1_0.TAG.10.0.1.3:
..B1.15: # Preds ..B1.3
  addl %eax, %edi #20.30
  lea (%rdi,%rdi,2), %edi #20.35
  jmp ..B1.37 # Prob 100% #20.35
..1.1_0.TAG.9.0.1.3:
..B1.17: # Preds ..B1.3
  notl %edi #19.29
  negl %edi #33.5
  addl %eax, %edi #33.5
  jmp ..B1.37 # Prob 100% #33.5
..1.1_0.TAG.8.0.1.3:
..B1.19: # Preds ..B1.3
  movl %eax, %ecx #18.25
  notl %ecx #18.25
  addl %ecx, %edi #18.29
  jmp ..B1.37 # Prob 100% #18.29
..1.1_0.TAG.7.0.1.3:
..B1.21: # Preds ..B1.3
  andl $7, %edi #17.34
  movl %edi, %ecx #17.34
  movl %eax, %edi #17.34
  shrl %cl, %edi #17.34
  jmp ..B1.37 # Prob 100% #17.34
..1.1_0.TAG.6.0.1.3:
..B1.23: # Preds ..B1.3
  andl $7, %edi #16.34
  movl %edi, %ecx #16.34
  movl %eax, %edi #16.34
  shll %cl, %edi #16.34
  jmp ..B1.37 # Prob 100% #16.34
..1.1_0.TAG.5.0.1.3:
..B1.25: # Preds ..B1.3
  andl %eax, %edi #15.28
  jmp ..B1.37 # Prob 100% #15.28
..1.1_0.TAG.4.0.1.3:
..B1.27: # Preds ..B1.3
  orl %eax, %edi #14.28
  jmp ..B1.37 # Prob 100% #14.28
..1.1_0.TAG.3.0.1.3:
..B1.29: # Preds ..B1.3
  xorl %eax, %edi #13.28
  jmp ..B1.37 # Prob 100% #13.28
..1.1_0.TAG.2.0.1.3:
..B1.31: # Preds ..B1.3
  imull %eax, %edi #12.28
  jmp ..B1.37 # Prob 100% #12.28
..1.1_0.TAG.1.0.1.3:
..B1.33: # Preds ..B1.3
  negl %edi #11.24
  addl %eax, %edi #11.24
  jmp ..B1.37 # Prob 100% #11.24
..1.1_0.TAG.0.0.1.3:
..B1.35: # Preds ..B1.3
  addl %eax, %edi #10.28
..B1.37: # Preds ..B1.35 ..B1.33 ..B1.31 ..B1.29 ..B1.27
  movslq %edi, %rdi #36.16
  incl %eax #33.28
  addq %rdi, %rsi #36.9
  cmpl $50000000, %eax #33.25
  jl ..B1.2 # Prob 100% #33.25
..B1.38: # Preds ..B1.37
  movl $.L_2__STRING.0, %edi #38.5
  xorl %eax, %eax #38.5
  call printf #38.5
..B1.39: # Preds ..B1.38
  xorl %eax, %eax #39.12
  movq %rbp, %rsp #39.12
  popq %rbp #39.12
  ret #39.12
.2.5_2.switchtab.6:
.L_2__STRING.0:
