main:
..B1.1: # Preds ..B1.0
  pushq %rbp #50.16
  movq %rsp, %rbp #50.16
  andq $-128, %rsp #50.16
  pushq %r12 #50.16
  pushq %r13 #50.16
  pushq %r14 #50.16
  pushq %r15 #50.16
  pushq %rbx #50.16
  subq $88, %rsp #50.16
  movl $3, %edi #50.16
  xorl %esi, %esi #50.16
  call __intel_new_feature_proc_init #50.16
..B1.43: # Preds ..B1.1
  stmxcsr (%rsp) #50.16
  movl $12345, %edx #51.23
  xorl %esi, %esi #52.14
  orl $32832, (%rsp) #50.16
  xorl %eax, %eax #55.16
  ldmxcsr (%rsp) #50.16
  movl %eax, %r15d #55.16
  movl %edx, %r14d #55.16
  movq %rsi, %rbx #55.16
..B1.2: # Preds ..B1.9 ..B1.43
  imull $1664525, %r14d, %r14d #56.23
  addl $1013904223, %r14d #56.34
  movl %r14d, %ecx #18.15
  shrl $16, %ecx #18.15
  xorl %r14d, %ecx #18.5
  imull $73244475, %ecx, %r8d #19.5
  movl %r8d, %edi #20.15
  shrl $16, %edi #20.15
  xorl %edi, %r8d #20.5
  imull $73244475, %r8d, %r9d #21.5
  movl %r9d, %r10d #27.22
  shrl $16, %r9d #22.15
  xorq %r9, %r10 #22.5
  movzwl %r10w, %r12d #23.16
  movq table(,%r12,8), %r13 #28.16
  movq %r13, %rcx #28.16
  testq %r13, %r13 #29.12
  je ..B1.7 # Prob 0% #29.12
..B1.4: # Preds ..B1.2 ..B1.5
  cmpl (%rcx), %r14d #30.23
  je ..B1.40 # Prob 20% #30.23
..B1.5: # Preds ..B1.4
  movq 8(%rcx), %rcx #31.13
  testq %rcx, %rcx #29.12
  jne ..B1.4 # Prob 82% #29.12
..B1.7: # Preds ..B1.5 ..B1.2
  movl $16, %edi #33.18
  call malloc #33.18
..B1.8: # Preds ..B1.7
  movl %r14d, (%rax) #34.5
  movl %r15d, 4(%rax) #35.5
  movq %r13, 8(%rax) #36.5
  movq %rax, table(,%r12,8) #37.5
..B1.9: # Preds ..B1.8 ..B1.40
  incl %r15d #55.34
  cmpl $2000000, %r15d #55.25
  jl ..B1.2 # Prob 99% #55.25
..B1.10: # Preds ..B1.9
  movq %rbx, %rsi #
  movl $12345, %edx #61.5
  xorl %eax, %eax #62.16
..B1.11: # Preds ..B1.17 ..B1.10
  imull $1664525, %edx, %edx #63.23
  addl $1013904223, %edx #63.34
  movl %edx, %ecx #18.15
  shrl $16, %ecx #18.15
  xorl %edx, %ecx #18.5
  imull $73244475, %ecx, %r8d #19.5
  movl %r8d, %edi #20.15
  shrl $16, %edi #20.15
  xorl %edi, %r8d #20.5
  imull $73244475, %r8d, %r9d #21.5
  movl %r9d, %r10d #42.16
  shrl $16, %r9d #22.15
  xorq %r9, %r10 #22.5
  movzwl %r10w, %r11d #23.16
  movq table(,%r11,8), %rcx #42.16
  testq %rcx, %rcx #43.12
  je ..B1.16 # Prob 0% #43.12
..B1.13: # Preds ..B1.11 ..B1.14
  cmpl (%rcx), %edx #44.23
  je ..B1.39 # Prob 20% #44.23
..B1.14: # Preds ..B1.13
  movq 8(%rcx), %rcx #45.13
  testq %rcx, %rcx #43.12
  jne ..B1.13 # Prob 82% #43.12
..B1.16: # Preds ..B1.14 ..B1.11
  movq $-1, %rcx #64.16
..B1.17: # Preds ..B1.16 ..B1.39
  incl %eax #62.34
  addq %rcx, %rsi #64.9
  cmpl $2000000, %eax #62.25
  jl ..B1.11 # Prob 99% #62.25
..B1.18: # Preds ..B1.17
  xorl %eax, %eax #68.16
  movl %edx, %r13d #68.16
  movl %eax, %r14d #68.16
  movq %rsi, %rbx #68.16
..B1.19: # Preds ..B1.34 ..B1.18
  imull $1664525, %r13d, %r13d #69.23
  addl $1013904223, %r13d #69.34
  movl %r13d, %ecx #18.15
  shrl $16, %ecx #18.15
  xorl %r13d, %ecx #18.5
  imull $73244475, %ecx, %r8d #19.5
  movl %r8d, %edi #20.15
  shrl $16, %edi #20.15
  xorl %edi, %r8d #20.5
  imull $73244475, %r8d, %edi #21.5
  movl %edi, %ecx #22.15
  shrl $16, %ecx #22.15
  testl $1, %r14d #70.17
  je ..B1.27 # Prob 60% #70.17
..B1.20: # Preds ..B1.19
  movl %edi, %edi #27.22
  movl %ecx, %ecx #27.22
  xorq %rcx, %rdi #22.5
  movzwl %di, %r15d #23.16
  movq table(,%r15,8), %r12 #28.16
  movq %r12, %rcx #28.16
  testq %r12, %r12 #29.12
  je ..B1.25 # Prob 0% #29.12
..B1.22: # Preds ..B1.20 ..B1.23
  cmpl (%rcx), %r13d #30.23
  je ..B1.37 # Prob 20% #30.23
..B1.23: # Preds ..B1.22
  movq 8(%rcx), %rcx #31.13
  testq %rcx, %rcx #29.12
  jne ..B1.22 # Prob 82% #29.12
..B1.25: # Preds ..B1.23 ..B1.20
  movl $16, %edi #33.18
  call malloc #33.18
..B1.26: # Preds ..B1.25
  movl %r13d, (%rax) #34.5
  movl %r14d, 4(%rax) #35.5
  movq %r12, 8(%rax) #36.5
  movq %rax, table(,%r15,8) #37.5
  jmp ..B1.34 # Prob 100% #37.5
..B1.27: # Preds ..B1.19
  xorq %rcx, %rdi #22.5
  movzwl %di, %r8d #23.16
  movq table(,%r8,8), %rcx #42.16
  testq %rcx, %rcx #43.12
  je ..B1.32 # Prob 0% #43.12
..B1.29: # Preds ..B1.27 ..B1.30
  cmpl (%rcx), %r13d #44.23
  je ..B1.38 # Prob 20% #44.23
..B1.30: # Preds ..B1.29
  movq 8(%rcx), %rcx #45.13
  testq %rcx, %rcx #43.12
  jne ..B1.29 # Prob 82% #43.12
..B1.32: # Preds ..B1.30 ..B1.27
  movq $-1, %rcx #73.20
..B1.33: # Preds ..B1.32 ..B1.38
  addq %rcx, %rbx #73.13
..B1.34: # Preds ..B1.26 ..B1.37 ..B1.33
  incl %r14d #68.34
  cmpl $2000000, %r14d #68.25
  jl ..B1.19 # Prob 99% #68.25
..B1.35: # Preds ..B1.34
  movq %rbx, %rsi #
  movl $.L_2__STRING.0, %edi #77.5
  xorl %eax, %eax #77.5
  call printf #77.5
..B1.36: # Preds ..B1.35
  xorl %eax, %eax #78.12
  addq $88, %rsp #78.12
  popq %rbx #78.12
  popq %r15 #78.12
  popq %r14 #78.12
  popq %r13 #78.12
  popq %r12 #78.12
  movq %rbp, %rsp #78.12
  popq %rbp #78.12
  ret #78.12
..B1.37: # Preds ..B1.22
  movl %r14d, 4(%rcx) #30.30
  jmp ..B1.34 # Prob 100% #30.30
..B1.38: # Preds ..B1.29
  movslq 4(%rcx), %rcx #44.35
  jmp ..B1.33 # Prob 100% #44.35
..B1.39: # Preds ..B1.13
  movslq 4(%rcx), %rcx #44.35
  jmp ..B1.17 # Prob 100% #44.35
..B1.40: # Preds ..B1.4
  movl %r15d, 4(%rcx) #30.30
  jmp ..B1.9 # Prob 100% #30.30
table:
.L_2__STRING.0:
