main:
..B1.1: # Preds ..B1.0
  pushq %rbp #12.16
  movq %rsp, %rbp #12.16
  andq $-128, %rsp #12.16
  subq $128, %rsp #12.16
  movl $3, %edi #12.16
  xorl %esi, %esi #12.16
  call __intel_new_feature_proc_init #12.16
..B1.16: # Preds ..B1.1
  stmxcsr (%rsp) #12.16
  xorl %esi, %esi #13.19
  movl $1, %edx #14.19
  xorl %ecx, %ecx #13.29
  xorl %r8d, %r8d #13.38
  orl $32832, (%rsp) #12.16
  xorl %edi, %edi #15.5
  ldmxcsr (%rsp) #12.16
  xorl %eax, %eax #13.29
..B1.2: # Preds ..B1.10 ..B1.16
  lea (%rsi,%rax), %r9d #15.5
  andl $1023, %r9d #16.30
  cmpl $1023, %r9d #16.45
  je ..B1.4 # Prob 50% #16.45
..B1.3: # Preds ..B1.2
  imull $1664525, %edx, %edx #17.27
  movl %esi, %r9d #18.13
  addl $1013904223, %edx #17.38
  andq $1023, %r9 #18.25
  incl %esi #19.13
  movl %edx, ring(,%r9,4) #18.13
..B1.4: # Preds ..B1.3 ..B1.2
  cmpl %ecx, %esi #21.21
  je ..B1.6 # Prob 50% #21.21
..B1.5: # Preds ..B1.4
  movl %ecx, %r9d #22.20
  decl %eax #23.13
  andq $1023, %r9 #22.32
  incl %ecx #23.13
  addl ring(,%r9,4), %r8d #22.13
..B1.6: # Preds ..B1.5 ..B1.4
  lea (%rsi,%rax), %r9d #21.9
  andl $1023, %r9d #16.30
  cmpl $1023, %r9d #16.45
  je ..B1.8 # Prob 50% #16.45
..B1.7: # Preds ..B1.6
  imull $1664525, %edx, %edx #17.27
  movl %esi, %r9d #18.13
  addl $1013904223, %edx #17.38
  andq $1023, %r9 #18.25
  incl %esi #19.13
  movl %edx, ring(,%r9,4) #18.13
..B1.8: # Preds ..B1.7 ..B1.6
  cmpl %ecx, %esi #21.21
  je ..B1.10 # Prob 50% #21.21
..B1.9: # Preds ..B1.8
  movl %ecx, %r9d #22.20
  decl %eax #23.13
  andq $1023, %r9 #22.32
  incl %ecx #23.13
  addl ring(,%r9,4), %r8d #22.13
..B1.10: # Preds ..B1.9 ..B1.8
  incl %edi #15.5
  cmpl $10000, %edi #15.5
  jb ..B1.2 # Prob 99% #15.5
..B1.11: # Preds ..B1.10
  addl $-20000, %esi #26.17
  addl $-20000, %ecx #26.17
  orl %ecx, %esi #26.17
  jne ..B1.13 # Prob 28% #26.17
..B1.12: # Preds ..B1.11
  movl $2, %edx #28.19
  xorl %eax, %eax #28.19
  cmpl $1920339856, %r8d #28.19
  cmovne %edx, %eax #28.19
  movq %rbp, %rsp #28.19
  popq %rbp #28.19
  ret #28.19
..B1.13: # Preds ..B1.11
  movl $1, %eax #27.16
  movq %rbp, %rsp #27.16
  popq %rbp #27.16
  ret #27.16
ring:
