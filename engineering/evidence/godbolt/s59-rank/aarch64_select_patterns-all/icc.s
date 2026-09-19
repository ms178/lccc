main:
..B1.1: # Preds ..B1.0
  pushq %rbp #25.16
  movq %rsp, %rbp #25.16
  andq $-128, %rsp #25.16
  pushq %r12 #25.16
  pushq %r13 #25.16
  pushq %r14 #25.16
  pushq %r15 #25.16
  subq $96, %rsp #25.16
  movl $3, %edi #25.16
  xorl %esi, %esi #25.16
  call __intel_new_feature_proc_init #25.16
..B1.7: # Preds ..B1.1
  stmxcsr (%rsp) #25.16
  xorl %r15d, %r15d #27.15
  xorl %esi, %esi #26.15
  incl %esi #26.15
  xorl %r14d, %r14d #28.18
  movl %esi, %r13d #29.16
  orl $32832, (%rsp) #25.16
  ldmxcsr (%rsp) #25.16
  movl %r14d, %r12d #29.16
..B1.2: # Preds ..B1.8 ..B1.7
  imull $1664525, %r13d, %r13d #30.25
  movl %r15d, %edi #31.17
  addl $1013904223, %r13d #30.36
  movl %r13d, %esi #31.17
  andl $1, %esi #31.17
  call conditional_increment #31.17
..B1.9: # Preds ..B1.2
  movl %r13d, %edi #32.21
  movl %r13d, %r8d #32.21
  movl %eax, %r15d #31.17
  andl $8, %edi #32.21
  shrl $3, %r8d #32.21
  movl %r13d, %esi #32.21
  movl %r12d, %edx #32.21
  movl %r15d, %ecx #32.21
  movl %r14d, %r9d #32.21
  call select_pressure #32.21
..B1.8: # Preds ..B1.9
  incl %r12d #29.38
  addl %eax, %r14d #32.9
  cmpl $50000000, %r12d #29.25
  jb ..B1.2 # Prob 100% #29.25
..B1.3: # Preds ..B1.8
  movl $65534, %edi #35.17
  call narrow_high_constant #35.17
..B1.10: # Preds ..B1.3
  xorl %eax, %r14d #36.5
  movl $.L_2__STRING.0, %edi #36.5
  movl %r15d, %esi #36.5
  movl %r14d, %edx #36.5
  xorl %eax, %eax #36.5
  call printf #36.5
..B1.4: # Preds ..B1.10
  xorl %eax, %eax #37.12
  addq $96, %rsp #37.12
  popq %r15 #37.12
  popq %r14 #37.12
  popq %r13 #37.12
  popq %r12 #37.12
  movq %rbp, %rsp #37.12
  popq %rbp #37.12
  ret #37.12
select_pressure:
..B2.1: # Preds ..B2.0
  testl %edi, %edi #22.38
  cmove %esi, %ecx #22.38
  cmovne %esi, %edx #22.38
  addl %ecx, %r9d #22.34
  addl %edx, %r9d #22.38
  movl %r9d, %eax #22.38
  ret #22.38
conditional_increment:
..B3.1: # Preds ..B3.0
  xorl %edx, %edx #9.12
  cmpl %esi, %edx #9.12
  adcl $0, %edi #9.12
  movl %edi, %eax #9.12
  ret #9.12
narrow_high_constant:
..B4.1: # Preds ..B4.0
  xorl %eax, %eax #13.21
  movzwl %di, %edx #12.63
  cmpl $65534, %edx #13.21
  sete %al #13.21
  ret #13.21
.L_2__STRING.0:
