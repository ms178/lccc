main:
..B1.1: # Preds ..B1.0
  pushq %rbp #10.16
  movq %rsp, %rbp #10.16
  andq $-128, %rsp #10.16
  subq $128, %rsp #10.16
  movl $3, %edi #10.16
  xorl %esi, %esi #10.16
  call __intel_new_feature_proc_init #10.16
..B1.12: # Preds ..B1.1
  stmxcsr 4(%rsp) #10.16
  movl $3, %edi #7.29
  movl $9, %esi #7.29
  orl $32832, 4(%rsp) #10.16
  ldmxcsr 4(%rsp) #10.16
  call ackermann #7.29
..B1.11: # Preds ..B1.12
  testl %eax, %eax #6.14
  jne ..B1.3 # Prob 50% #6.14
..B1.2: # Preds ..B1.11
  movl $1, %edi #6.24
  movl %edi, %esi #6.24
  call ackermann..0 #6.24
  jmp ..B1.4 # Prob 100% #6.24
..B1.3: # Preds ..B1.11
  decl %eax #7.29
  movl $2, %edi #7.29
  movl %eax, %esi #7.29
  call ackermann #7.29
..B1.15: # Preds ..B1.3
  movl $1, %edi #7.12
  movl %eax, %esi #7.12
  call ackermann #7.12
..B1.4: # Preds ..B1.15 ..B1.2
  testl %eax, %eax #6.14
  jne ..B1.6 # Prob 50% #6.14
..B1.5: # Preds ..B1.4
  movl $1, %edi #6.24
  movl %edi, %esi #6.24
  call ackermann..0 #6.24
  jmp ..B1.7 # Prob 100% #6.24
..B1.6: # Preds ..B1.4
  decl %eax #7.29
  movl $2, %edi #7.29
  movl %eax, %esi #7.29
  call ackermann #7.29
..B1.18: # Preds ..B1.6
  movl $1, %edi #7.12
  movl %eax, %esi #7.12
  call ackermann #7.12
..B1.7: # Preds ..B1.18 ..B1.5
  movl %eax, (%rsp) #11.25
  movl $.L_2__STRING.0, %edi #12.5
  xorl %eax, %eax #12.5
  movl (%rsp), %esi #12.40
  call printf #12.5
..B1.8: # Preds ..B1.7
  xorl %eax, %eax #13.12
  movq %rbp, %rsp #13.12
  popq %rbp #13.12
  ret #13.12
ackermann..0:
..B2.1: # Preds ..B2.0
  pushq %r12 #4.36
  pushq %r13 #4.36
  pushq %rsi #4.36
  movl %edi, %r13d #4.36
  testl %r13d, %r13d #5.14
  je ..B2.8 # Prob 28% #5.14
..B2.2: # Preds ..B2.1
  movl $1, %esi #6.24
  lea -1(%r13), %r12d #6.38
  movl %r12d, %edi #6.24
  call ackermann #6.24
..B2.13: # Preds ..B2.2
  cmpl $1, %r13d #5.14
  je ..B2.6 # Prob 28% #5.14
..B2.3: # Preds ..B2.13
  testl %eax, %eax #6.14
  jne ..B2.5 # Prob 50% #6.14
..B2.4: # Preds ..B2.3
  addl $-2, %r13d #6.24
  movl $1, %esi #6.24
  movl %r13d, %edi #6.24
  addq $8, %rsp #6.24
  popq %r13 #6.24
  popq %r12 #6.24
  jmp ackermann #6.24
..B2.5: # Preds ..B2.3
  decl %eax #7.29
  movl %r12d, %edi #7.29
  movl %eax, %esi #7.29
  call ackermann #7.29
..B2.14: # Preds ..B2.5
  addl $-2, %r13d #7.12
  movl %eax, %esi #7.12
  movl %r13d, %edi #7.12
  addq $8, %rsp #7.12
  popq %r13 #7.12
  popq %r12 #7.12
  jmp ackermann #7.12
..B2.6: # Preds ..B2.13
  incl %eax #5.28
..B2.7: # Preds ..B2.6
  popq %rcx #7.12
  popq %r13 #7.12
  popq %r12 #7.12
  ret #7.12
..B2.8: # Preds ..B2.1
  movl $2, %eax #5.28
  popq %rcx #5.28
  popq %r13 #5.28
  popq %r12 #5.28
  ret #5.28
ackermann:
..B3.1: # Preds ..B3.0
  pushq %r12 #4.36
  pushq %r13 #4.36
  pushq %rsi #4.36
  movl %edi, %r12d #4.36
  testl %r12d, %r12d #5.14
  je ..B3.26 # Prob 28% #5.14
..B3.2: # Preds ..B3.1
  testl %esi, %esi #6.14
  jne ..B3.12 # Prob 50% #6.14
..B3.3: # Preds ..B3.2
  cmpl $1, %r12d #5.14
  je ..B3.10 # Prob 28% #5.14
..B3.4: # Preds ..B3.3
  movl $1, %esi #6.24
  lea -2(%r12), %r13d #6.38
  movl %r13d, %edi #6.24
  call ackermann..0 #6.24
..B3.32: # Preds ..B3.4
  cmpl $2, %r12d #5.14
  je ..B3.8 # Prob 28% #5.14
..B3.5: # Preds ..B3.32
  testl %eax, %eax #6.14
  jne ..B3.7 # Prob 50% #6.14
..B3.6: # Preds ..B3.5
  addl $-3, %r12d #6.24
  movl $1, %esi #6.24
  movl %r12d, %edi #6.24
  addq $8, %rsp #6.24
  popq %r13 #6.24
  popq %r12 #6.24
  jmp ackermann..0 #6.24
..B3.7: # Preds ..B3.5
  decl %eax #7.29
  movl %r13d, %edi #7.29
  movl %eax, %esi #7.29
  call ackermann #7.29
..B3.33: # Preds ..B3.7
  addl $-3, %r12d #7.12
  movl %eax, %esi #7.12
  movl %r12d, %edi #7.12
  addq $8, %rsp #7.12
  popq %r13 #7.12
  popq %r12 #7.12
  jmp ackermann #7.12
..B3.8: # Preds ..B3.32
  incl %eax #5.28
  popq %rcx #5.28
  popq %r13 #5.28
  popq %r12 #5.28
  ret #5.28
..B3.10: # Preds ..B3.3
  movl $2, %eax #6.24
..B3.11: # Preds ..B3.10
  popq %rcx #6.24
  popq %r13 #6.24
  popq %r12 #6.24
  ret #6.24
..B3.12: # Preds ..B3.2
  cmpl $1, %esi #6.14
  jne ..B3.15 # Prob 50% #6.14
..B3.13: # Preds ..B3.12
  movl $1, %esi #6.24
  lea -1(%r12), %edi #6.24
  call ackermann..0 #6.24
..B3.34: # Preds ..B3.13
  cmpl $1, %r12d #5.14
  je ..B3.24 # Prob 28% #5.14
..B3.14: # Preds ..B3.34
  lea -2(%r12), %r13d #6.38
  jmp ..B3.20 # Prob 100% #6.38
..B3.15: # Preds ..B3.12
  addl $-2, %esi #7.29
  movl %r12d, %edi #7.29
  call ackermann #7.29
..B3.35: # Preds ..B3.15
  cmpl $1, %r12d #5.14
  je ..B3.23 # Prob 28% #5.14
..B3.16: # Preds ..B3.35
  lea -2(%r12), %r13d #6.38
  testl %eax, %eax #6.14
  jne ..B3.18 # Prob 50% #6.14
..B3.17: # Preds ..B3.16
  movl %r13d, %edi #6.24
  movl $1, %esi #6.24
  call ackermann..0 #6.24
  jmp ..B3.20 # Prob 100% #6.24
..B3.18: # Preds ..B3.16
  decl %eax #7.29
  lea -1(%r12), %edi #7.29
  movl %eax, %esi #7.29
  call ackermann #7.29
..B3.38: # Preds ..B3.18
  movl %r13d, %edi #7.12
  movl %eax, %esi #7.12
  call ackermann #7.12
..B3.20: # Preds ..B3.17 ..B3.38 ..B3.14
  testl %eax, %eax #6.14
  jne ..B3.22 # Prob 50% #6.14
..B3.21: # Preds ..B3.20
  movl %r13d, %edi #6.24
  movl $1, %esi #6.24
  addq $8, %rsp #6.24
  popq %r13 #6.24
  popq %r12 #6.24
  jmp ackermann..0 #6.24
..B3.22: # Preds ..B3.20
  decl %r12d #7.29
  decl %eax #7.29
  movl %r12d, %edi #7.29
  movl %eax, %esi #7.29
  call ackermann #7.29
..B3.40: # Preds ..B3.22
  movl %r13d, %edi #7.12
  movl %eax, %esi #7.12
  call ackermann #7.12
  popq %rcx #7.12
  popq %r13 #7.12
  popq %r12 #7.12
  ret #7.12
..B3.23: # Preds ..B3.35
  incl %eax #5.28
..B3.24: # Preds ..B3.23 ..B3.34
  incl %eax #5.28
..B3.25: # Preds ..B3.24
  popq %rcx #7.12
  popq %r13 #7.12
  popq %r12 #7.12
  ret #7.12
..B3.26: # Preds ..B3.1
  incl %esi #5.28
  movl %esi, %eax #5.28
  popq %rcx #5.28
  popq %r13 #5.28
  popq %r12 #5.28
  ret #5.28
.L_2__STRING.0:
