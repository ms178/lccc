rot:
..B1.1: # Preds ..B1.0
  pushq %r14 #3.54
  pushq %r15 #3.54
  pushq %rbx #3.54
  pushq %rbp #3.54
  movl (%rdi), %eax #4.18
  movl 4(%rdi), %r15d #4.29
  movl 8(%rdi), %r14d #4.40
  movl 12(%rdi), %r11d #4.51
  movl 16(%rdi), %r10d #5.18
  movl 20(%rdi), %r9d #5.29
  movl 24(%rdi), %r8d #5.40
  movl 28(%rdi), %ebp #5.51
  testl %edx, %edx #6.25
  jle ..B1.8 # Prob 50% #6.25
..B1.2: # Preds ..B1.1
  movl %edx, %ebx #6.5
  movl $1, %ecx #6.5
  xorl %edi, %edi #6.5
  shrl $1, %ebx #6.5
  je ..B1.6 # Prob 9% #6.5
..B1.3: # Preds ..B1.2
  movq %r12, -24(%rsp) #6.5[spill]
  movq %r13, -16(%rsp) #6.5[spill]
..B1.4: # Preds ..B1.4 ..B1.3
  addl (%rsi,%rdi,8), %ebp #7.39
  lea (%r10,%r9), %ecx #7.27
  addl %r8d, %ecx #7.31
  lea (%rax,%r15), %r12d #8.27
  addl %ebp, %ecx #7.35
  addl %r14d, %r12d #8.31
  movl %r8d, %r13d #9.9
  movl %r10d, %r8d #9.23
  movl %r14d, %r10d #10.9
  movl %eax, %r14d #10.23
  movl %r9d, %ebp #9.16
  addl 4(%rsi,%rdi,8), %r13d #7.39
  lea (%r11,%rcx), %r9d #9.38
  movl %r15d, %r11d #10.16
  lea (%rcx,%r12), %r15d #10.39
  incq %rdi #6.5
  lea (%r8,%r9), %eax #7.27
  addl %ebp, %eax #7.31
  lea (%r14,%r15), %ecx #8.27
  addl %r13d, %eax #7.35
  addl %r11d, %ecx #8.31
  addl %eax, %r10d #9.38
  addl %ecx, %eax #10.39
  cmpq %rbx, %rdi #6.5
  jb ..B1.4 # Prob 63% #6.5
..B1.5: # Preds ..B1.4
  movq -24(%rsp), %r12 #[spill]
  lea 1(%rdi,%rdi), %ecx #7.39
  movq -16(%rsp), %r13 #[spill]
..B1.6: # Preds ..B1.5 ..B1.2
  lea -1(%rcx), %ebx #6.5
  cmpl %edx, %ebx #6.5
  jae ..B1.8 # Prob 9% #6.5
..B1.7: # Preds ..B1.6
  movslq %ecx, %rcx #6.5
  lea (%r10,%r9), %edx #7.27
  addl %r8d, %edx #7.31
  lea (%rax,%r15), %ebx #8.27
  addl %r14d, %ebx #8.31
  addl -4(%rsi,%rcx,4), %ebp #7.39
  addl %ebp, %edx #7.35
  movl %r8d, %ebp #9.9
  movl %r9d, %r8d #9.16
  movl %r10d, %r9d #9.23
  lea (%r11,%rdx), %r10d #9.38
  movl %r14d, %r11d #10.9
  movl %r15d, %r14d #10.16
  movl %eax, %r15d #10.23
  lea (%rdx,%rbx), %eax #10.39
..B1.8: # Preds ..B1.7 ..B1.6 ..B1.1
  xorl %r15d, %eax #12.16
  xorl %r14d, %eax #12.20
  xorl %r11d, %eax #12.24
  xorl %r10d, %eax #12.28
  xorl %r9d, %eax #12.32
  xorl %r8d, %eax #12.36
  xorl %ebp, %eax #12.40
  popq %rbp #12.40
  popq %rbx #12.40
  popq %r15 #12.40
  popq %r14 #12.40
  ret #12.40
