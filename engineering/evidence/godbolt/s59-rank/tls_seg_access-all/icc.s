main:
..B1.1: # Preds ..B1.0
  pushq %rbp #54.1
  movq %rsp, %rbp #54.1
  andq $-128, %rsp #54.1
  pushq %r12 #54.1
  pushq %r13 #54.1
  subq $112, %rsp #54.1
  movl $3, %edi #54.1
  xorl %esi, %esi #54.1
  call __intel_new_feature_proc_init #54.1
..B1.8: # Preds ..B1.1
  stmxcsr (%rsp) #54.1
  xorl %esi, %esi #55.23
  xorl %eax, %eax #56.19
  orl $32832, (%rsp) #54.1
  movl %eax, %r13d #56.19
  ldmxcsr (%rsp) #54.1
  movq %rsi, %r12 #56.19
..B1.2: # Preds ..B1.3 ..B1.8
  movl %r13d, %edi #57.14
  orl $1, %edi #57.14
  call tls_pass #57.14
..B1.3: # Preds ..B1.2
  incl %r13d #56.36
  addq %rax, %r12 #57.5
  cmpl $200000, %r13d #56.28
  jb ..B1.2 # Prob 99% #56.28
..B1.4: # Preds ..B1.3
  movq %r12, %rsi #
  movl $.L_2__STRING.0, %edi #58.3
  xorl %eax, %eax #58.3
  call printf #58.3
..B1.5: # Preds ..B1.4
  xorl %eax, %eax #59.10
  addq $112, %rsp #59.10
  popq %r13 #59.10
  popq %r12 #59.10
  movq %rbp, %rsp #59.10
  popq %rbp #59.10
  ret #59.10
tls_pass:
..B2.1: # Preds ..B2.0
  xorl %edx, %edx #36.21
  movq %fs:0, %rsi #41.3
  movl %edi, %edi #35.1
  lea tls_slots@TPOFF(%rsi), %rax #41.19
  movq %rdi, 8(%rax) #42.3
  lea 8(%rax), %rcx #41.3
  movq %rcx, tls_indirect@TPOFF(%rsi) #41.3
  movb $2, %sil #43.14
  movl $2, %ecx #43.14
..B2.2: # Preds ..B2.2 ..B2.1
  movb %sil, %r8b #46.73
  incb %sil #43.31
  andb $7, %r8b #46.73
  movzbl %r8b, %r9d #46.58
  movq -8(%rax,%rcx,8), %r11 #45.22
  addq %rdi, %r11 #45.41
  movq %r11, (%rax,%rcx,8) #45.7
  incq %rcx #43.31
  movq 8(%rax,%r9,8), %r10 #46.58
  xorq %r10, %r11 #46.58
  addq %r11, %rdx #46.7
  cmpb $64, %sil #43.24
  jle ..B2.2 # Prob 98% #43.24
..B2.3: # Preds ..B2.2
  movq 8(%rax), %rax #49.17
  addq %rdx, %rax #49.17
  ret #49.17
.L_2__STRING.0:
tls_slots:
tls_indirect:
