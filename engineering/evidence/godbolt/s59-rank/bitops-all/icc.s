main:
..B1.1: # Preds ..B1.0
  pushq %rbp #43.16
  movq %rsp, %rbp #43.16
  andq $-128, %rsp #43.16
  pushq %r12 #43.16
  pushq %r13 #43.16
  pushq %r14 #43.16
  subq $104, %rsp #43.16
  movl $3, %edi #43.16
  xorl %esi, %esi #43.16
  call __intel_new_feature_proc_init #43.16
..B1.16: # Preds ..B1.1
  stmxcsr (%rsp) #43.16
  xorl %esi, %esi #45.18
  movl $-559038737, %r11d #44.23
  xorl %edx, %edx #45.31
  xorl %r10d, %r10d #45.18
  orl $32832, (%rsp) #43.16
  xorl %ecx, %ecx #45.44
  ldmxcsr (%rsp) #43.16
  xorl %r8d, %r8d #45.58
  xorl %edi, %edi #47.16
  movl $16, %r9d #47.16
  movl $2147483647, %eax #47.16
..B1.2: # Preds ..B1.11 ..B1.16
  imull $1664525, %r11d, %r11d #48.23
  addl $1013904223, %r11d #48.34
  movl %r11d, %r12d #9.20
  movl %r11d, %r14d #9.5
  shrl $1, %r12d #9.20
  andl $1431655765, %r12d #9.25
  subl %r12d, %r14d #9.5
  movl %r14d, %r13d #10.14
  shrl $2, %r14d #10.35
  andl $858993459, %r13d #10.14
  andl $858993459, %r14d #10.40
  addl %r14d, %r13d #10.40
  movl %r13d, %r14d #11.20
  shrl $4, %r14d #11.20
  addl %r14d, %r13d #11.20
  andl $252645135, %r13d #11.26
  imull $16843009, %r13d, %r13d #12.17
  shrl $24, %r13d #12.32
  addq %r13, %rsi #49.9
  testl %r11d, %r11d #16.14
  je ..B1.10 # Prob 28% #16.14
..B1.3: # Preds ..B1.2
  cmpl $65535, %r11d #18.28
  movq %r10, %r14 #18.28
  movl %r11d, %r13d #18.37
  cmovbe %r9, %r14 #18.28
  shll $16, %r13d #18.37
  cmpl $65535, %r11d #18.37
  cmova %r11d, %r13d #18.37
  cmpl $16777215, %r13d #19.14
  ja ..B1.5 # Prob 38% #19.14
..B1.4: # Preds ..B1.3
  shll $8, %r13d #19.37
  addq $8, %r14 #19.28
..B1.5: # Preds ..B1.4 ..B1.3
  cmpl $268435455, %r13d #20.14
  ja ..B1.7 # Prob 38% #20.14
..B1.6: # Preds ..B1.5
  shll $4, %r13d #20.37
  addq $4, %r14 #20.28
..B1.7: # Preds ..B1.6 ..B1.5
  cmpl $1073741823, %r13d #21.14
  ja ..B1.9 # Prob 38% #21.14
..B1.8: # Preds ..B1.7
  shll $2, %r13d #21.37
  addq $2, %r14 #21.28
..B1.9: # Preds ..B1.8 ..B1.7
  incq %r14 #22.28
  cmpl %r13d, %eax #50.20
  sbbq $0, %r14 #50.20
  jmp ..B1.11 # Prob 100% #50.20
..B1.10: # Preds ..B1.2
  movl $32, %r14d #50.20
..B1.11: # Preds ..B1.10 ..B1.9
  movl %r11d, %r13d #27.41
  addq %r14, %rdx #50.9
  andl $-715827883, %r13d #27.41
  incl %edi #47.28
  addl %r13d, %r13d #27.56
  orl %r13d, %r12d #27.56
  movl %r12d, %r14d #28.16
  andl $-214748365, %r12d #28.41
  shrl $2, %r14d #28.16
  shll $2, %r12d #28.56
  andl $858993459, %r14d #28.21
  orl %r12d, %r14d #28.56
  movl %r14d, %r12d #29.16
  andl $-15790321, %r14d #29.41
  shrl $4, %r12d #29.16
  shll $4, %r14d #29.56
  andl $252645135, %r12d #29.21
  orl %r14d, %r12d #29.56
  movl %r12d, %r13d #30.16
  andl $-65281, %r12d #30.41
  shrl $8, %r13d #30.16
  shll $8, %r12d #30.56
  orl %r12d, %r13d #30.56
  shldl $16, %r13d, %r13d #31.27
  movzwl %r11w, %r12d #52.44
  decl %r12d #36.5
  movzbl %r13b, %r14d #51.41
  movl %r12d, %r13d #37.15
  shrl $1, %r13d #37.15
  addq %r14, %rcx #51.9
  orl %r13d, %r12d #37.5
  movl %r12d, %r14d #37.28
  shrl $2, %r14d #37.28
  orl %r14d, %r12d #37.18
  movl %r12d, %r13d #37.41
  shrl $4, %r13d #37.41
  orl %r13d, %r12d #37.31
  movl %r12d, %r14d #38.15
  shrl $8, %r14d #38.15
  orl %r14d, %r12d #38.5
  movl %r12d, %r13d #38.28
  shrl $16, %r13d #38.28
  orl %r13d, %r12d #38.18
  incl %r12d #39.5
  addq %r12, %r8 #52.9
  cmpl $50000000, %edi #47.25
  jl ..B1.2 # Prob 100% #47.25
..B1.12: # Preds ..B1.11
  movl $.L_2__STRING.0, %edi #55.5
  xorl %eax, %eax #55.5
  call printf #55.5
..B1.13: # Preds ..B1.12
  xorl %eax, %eax #56.12
  addq $104, %rsp #56.12
  popq %r14 #56.12
  popq %r13 #56.12
  popq %r12 #56.12
  movq %rbp, %rsp #56.12
  popq %rbp #56.12
  ret #56.12
.L_2__STRING.0:
