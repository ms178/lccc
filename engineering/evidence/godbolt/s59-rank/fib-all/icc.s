main:
..B1.1: # Preds ..B1.0
  pushq %rbp #9.16
  movq %rsp, %rbp #9.16
  andq $-128, %rsp #9.16
  pushq %r12 #9.16
  pushq %r13 #9.16
  pushq %r14 #9.16
  pushq %r15 #9.16
  pushq %rbx #9.16
  subq $88, %rsp #9.16
  movl $3, %edi #9.16
  xorl %esi, %esi #9.16
  call __intel_new_feature_proc_init #9.16
..B1.13: # Preds ..B1.1
  stmxcsr 8(%rsp) #9.16
  movl $32, %edi #6.25
  orl $32832, 8(%rsp) #9.16
  ldmxcsr 8(%rsp) #9.16
  call fib #6.25
..B1.12: # Preds ..B1.13
  movl $36, %edi #6.12
  movq %rax, %r15 #6.25
  call fib #6.12
..B1.11: # Preds ..B1.12
  movl $35, %edi #6.25
  movq %rax, %r13 #6.12
  call fib #6.25
..B1.10: # Preds ..B1.11
  movl $38, %edi #6.12
  movq %rax, 16(%rsp) #6.25[spill]
  call fib #6.12
..B1.9: # Preds ..B1.10
  movl $37, %edi #6.12
  movq %rax, %r12 #6.12
  call fib #6.12
..B1.8: # Preds ..B1.9
  movl $33, %edi #6.12
  movq %rax, %rbx #6.12
  call fib #6.12
..B1.7: # Preds ..B1.8
  movl $31, %edi #6.25
  movq %rax, %r14 #6.12
  call fib #6.25
..B1.6: # Preds ..B1.7
  movl $34, %edi #6.12
  movq %rax, 8(%rsp) #6.25[spill]
  call fib #6.12
..B1.5: # Preds ..B1.6
  addq %r15, %r14 #6.25
  addq %rbx, %r12 #6.25
  addq 8(%rsp), %r15 #6.25[spill]
  movl $.L_2__STRING.0, %edi #11.5
  addq 16(%rsp), %r13 #6.25[spill]
  addq %rax, %r15 #6.25
  addq %r12, %r13 #6.25
  addq %r15, %r14 #6.25
  addq %r14, %r13 #6.25
  xorl %eax, %eax #11.5
  movq %r13, (%rsp) #10.26
  movq (%rsp), %rsi #11.31
  call printf #11.5
..B1.2: # Preds ..B1.5
  xorl %eax, %eax #12.12
  addq $88, %rsp #12.12
  popq %rbx #12.12
  popq %r15 #12.12
  popq %r14 #12.12
  popq %r13 #12.12
  popq %r12 #12.12
  movq %rbp, %rsp #12.12
  popq %rbp #12.12
  ret #12.12
fib:
..B2.1: # Preds ..B2.0
  pushq %r12 #4.17
  pushq %r13 #4.17
  pushq %r15 #4.17
  pushq %rbx #4.17
  pushq %rbp #4.17
  movl %edi, %r15d #4.17
  cmpl $1, %r15d #5.14
  jle ..B2.22 # Prob 12% #5.14
..B2.2: # Preds ..B2.1
  cmpl $2, %r15d #5.14
  jle ..B2.21 # Prob 12% #5.14
..B2.3: # Preds ..B2.2
  lea -2(%r15), %edi #6.12
  call fib #6.12
..B2.25: # Preds ..B2.3
  movq %rax, %r13 #6.12
  cmpl $4, %r15d #5.14
  jle ..B2.18 # Prob 12% #5.14
..B2.4: # Preds ..B2.25
  lea -4(%r15), %edi #6.12
  call fib #6.12
..B2.28: # Preds ..B2.4
  movq %rax, %rbx #6.12
  lea -5(%r15), %edi #6.25
  call fib #6.25
..B2.27: # Preds ..B2.28
  addq %rax, %rbx #6.25
  lea -3(%r15), %edi #6.12
  addq %rbx, %r13 #6.25
  call fib #6.12
..B2.26: # Preds ..B2.27
  movq %rax, %r12 #6.12
  cmpl $5, %r15d #5.14
  jle ..B2.16 # Prob 12% #5.14
..B2.5: # Preds ..B2.26
  cmpl $6, %r15d #5.14
  jle ..B2.15 # Prob 12% #5.14
..B2.6: # Preds ..B2.5
  lea -6(%r15), %edi #6.12
  call fib #6.12
..B2.29: # Preds ..B2.6
  movq %rax, %rbp #6.12
  cmpl $8, %r15d #5.14
  jle ..B2.12 # Prob 12% #5.14
..B2.7: # Preds ..B2.29
  lea -8(%r15), %edi #6.12
  call fib #6.12
..B2.31: # Preds ..B2.7
  movq %rax, %rbx #6.12
  lea -9(%r15), %edi #6.25
  call fib #6.25
..B2.30: # Preds ..B2.31
  addq %rbx, %rax #6.25
  addq %rax, %rbp #6.25
..B2.8: # Preds ..B2.30 ..B2.33
  addl $-7, %r15d #6.12
  movl %r15d, %edi #6.12
  call fib #6.12
..B2.32: # Preds ..B2.8
  addq %rbx, %rax #6.25
..B2.9: # Preds ..B2.32 ..B2.14
  addq %rax, %rbp #6.25
..B2.10: # Preds ..B2.9 ..B2.17
  addq %rbp, %r12 #6.25
..B2.11: # Preds ..B2.10 ..B2.20
  addq %r13, %r12 #6.25
  movq %r12, %rax #6.25
  popq %rbp #6.25
  popq %rbx #6.25
  popq %r15 #6.25
  popq %r13 #6.25
  popq %r12 #6.25
  ret #6.25
..B2.12: # Preds ..B2.29
  movslq %r15d, %rax #5.24
  lea -7(%rbp,%rax), %rbp #6.25
  cmpl $7, %r15d #5.14
  jle ..B2.14 # Prob 12% #5.14
..B2.13: # Preds ..B2.12
  lea -8(%r15), %edi #6.25
  call fib #6.25
..B2.33: # Preds ..B2.13
  movq %rax, %rbx #6.25
  jmp ..B2.8 # Prob 100% #6.25
..B2.15: # Preds ..B2.5
  movslq %r15d, %rax #5.24
  lea -5(%rax), %rbp #6.20
..B2.14: # Preds ..B2.15 ..B2.12
  addq $-6, %rax #6.33
  jmp ..B2.9 # Prob 100% #6.33
..B2.16: # Preds ..B2.26
  movslq %r15d, %rbp #5.24
..B2.17: # Preds ..B2.34 ..B2.16
  addq $-4, %rbp #6.33
  jmp ..B2.10 # Prob 100% #6.33
..B2.18: # Preds ..B2.25
  movslq %r15d, %rbp #5.24
  lea -3(%r13,%rbp), %r13 #6.25
  cmpl $3, %r15d #5.14
  jle ..B2.20 # Prob 12% #5.14
..B2.19: # Preds ..B2.18
  addl $-3, %r15d #6.12
  movl %r15d, %edi #6.12
  call fib #6.12
..B2.34: # Preds ..B2.19
  movq %rax, %r12 #6.12
  jmp ..B2.17 # Prob 100% #6.12
..B2.21: # Preds ..B2.2
  movslq %r15d, %rbp #5.24
  lea -1(%rbp), %r13 #6.20
..B2.20: # Preds ..B2.21 ..B2.18
  lea -2(%rbp), %r12 #6.33
  jmp ..B2.11 # Prob 100% #6.33
..B2.22: # Preds ..B2.1
  movslq %r15d, %r15 #5.24
  movq %r15, %rax #5.24
  popq %rbp #5.24
  popq %rbx #5.24
  popq %r15 #5.24
  popq %r13 #5.24
  popq %r12 #5.24
  ret #5.24
.L_2__STRING.0:
