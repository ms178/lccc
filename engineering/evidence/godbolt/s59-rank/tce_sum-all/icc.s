main:
..B1.1: # Preds ..B1.0
  pushq %rbp #11.16
  movq %rsp, %rbp #11.16
  andq $-128, %rsp #11.16
  subq $128, %rsp #11.16
  movl $3, %edi #11.16
  xorl %esi, %esi #11.16
  call __intel_new_feature_proc_init #11.16
..B1.6: # Preds ..B1.1
  stmxcsr 8(%rsp) #11.16
  movl $9999996, %edi #8.12
  movl $39999994, %esi #8.12
  orl $32832, 8(%rsp) #11.16
  ldmxcsr 8(%rsp) #11.16
  call sum #8.12
..B1.5: # Preds ..B1.6
  movq %rax, (%rsp) #12.26
  movl $.L_2__STRING.0, %edi #13.5
  xorl %eax, %eax #13.5
  movq (%rsp), %rsi #13.37
  call printf #13.5
..B1.2: # Preds ..B1.5
  xorl %eax, %eax #14.12
  movq %rbp, %rsp #14.12
  popq %rbp #14.12
  ret #14.12
sum:
..B2.1: # Preds ..B2.0
  movslq %edi, %rdx #6.34
  testq %rdx, %rdx #7.14
  jle ..B2.8 # Prob 2% #7.14
..B2.2: # Preds ..B2.1
  addq %rdx, %rsi #8.29
  cmpq $1, %rdx #7.14
  jle ..B2.8 # Prob 2% #7.14
..B2.3: # Preds ..B2.2
  lea -1(%rsi,%rdx), %rsi #8.29
  cmpq $2, %rdx #7.14
  jle ..B2.8 # Prob 2% #7.14
..B2.4: # Preds ..B2.3
  lea -2(%rsi,%rdx), %rsi #8.29
  cmpq $3, %rdx #7.14
  jle ..B2.8 # Prob 2% #7.14
..B2.5: # Preds ..B2.4
  addl $-4, %edi #8.12
  lea -3(%rsi,%rdx), %rsi #8.12
  jmp sum #8.12
..B2.8: # Preds ..B2.1 ..B2.2 ..B2.3 ..B2.4
  movq %rsi, %rax #8.12
  ret #8.12
.L_2__STRING.0:
