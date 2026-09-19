main:
..B1.1: # Preds ..B1.0
  pushq %rbp #15.16
  movq %rsp, %rbp #15.16
  andq $-128, %rsp #15.16
  subq $128, %rsp #15.16
  movl $3, %edi #15.16
  xorl %esi, %esi #15.16
  call __intel_new_feature_proc_init #15.16
..B1.8: # Preds ..B1.1
  stmxcsr (%rsp) #15.16
  movl $42, %edx #16.23
  xorl %eax, %eax #17.5
  orl $32832, (%rsp) #15.16
  ldmxcsr (%rsp) #15.16
..B1.2: # Preds ..B1.2 ..B1.8
  imull $1664525, %edx, %ecx #18.23
  addl $1013904223, %ecx #18.34
  movl %ecx, %edx #19.31
  andl $2147483647, %edx #19.31
  movl %edx, arr(,%rax,8) #19.9
  imull $1664525, %ecx, %edx #18.23
  addl $1013904223, %edx #18.34
  movl %edx, %esi #19.31
  andl $2147483647, %esi #19.31
  movl %esi, 4+arr(,%rax,8) #19.9
  incq %rax #17.5
  cmpq $500000, %rax #17.5
  jb ..B1.2 # Prob 99% #17.5
..B1.3: # Preds ..B1.2
  movl $arr, %edi #21.5
  movl $1000000, %esi #21.5
  movl $4, %edx #21.5
  movl $cmp, %ecx #21.5
  call qsort #21.5
..B1.4: # Preds ..B1.3
  movl $.L_2__STRING.0, %edi #22.5
  xorl %eax, %eax #22.5
  movl 2000000+arr(%rip), %esi #22.5
  call printf #22.5
..B1.5: # Preds ..B1.4
  xorl %eax, %eax #23.12
  movq %rbp, %rsp #23.12
  popq %rbp #23.12
  ret #23.12
cmp:
..B2.1: # Preds ..B2.0
  movl (%rdi), %eax #12.19
  subl (%rsi), %eax #12.19
  ret #12.30
arr:
.L_2__STRING.0:
