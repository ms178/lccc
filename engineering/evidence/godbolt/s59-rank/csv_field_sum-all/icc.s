main:
..B1.1: # Preds ..B1.0
  pushq %rbp #32.16
  movq %rsp, %rbp #32.16
  andq $-128, %rsp #32.16
  pushq %r13 #32.16
  pushq %r14 #32.16
  pushq %r15 #32.16
  pushq %rbx #32.16
  subq $96, %rsp #32.16
  movl $3, %edi #32.16
  xorl %esi, %esi #32.16
  call __intel_new_feature_proc_init #32.16
..B1.71: # Preds ..B1.1
  stmxcsr (%rsp) #32.16
  xorl %edi, %edi #34.11
  xorb %sil, %sil #35.16
  orl $32832, (%rsp) #32.16
  xorl %ebx, %ebx #34.11
  ldmxcsr (%rsp) #32.16
  xorl %ecx, %ecx #35.16
  xorl %r8d, %r8d #35.16
..B1.2: # Preds ..B1.21 ..B1.71
  movslq %edi, %r13 #40.17
  movl %ebx, %r11d #36.20
  movl %ecx, %r10d #36.20
..B1.3: # Preds ..B1.2 ..B1.19 ..B1.18
  movl $274877907, %eax #38.42
  movl %r10d, %r9d #38.42
  imull %r10d #38.42
  sarl $31, %r9d #38.42
  sarl $6, %edx #38.42
  subl %r9d, %edx #38.42
  imull $-1000, %edx, %r14d #38.42
  lea -500(%r10,%r14), %r9d #38.49
  testl %r9d, %r9d #39.21
  jns ..B1.5 # Prob 76% #39.21
..B1.4: # Preds ..B1.3
  incl %edi #40.21
  negl %r9d #41.22
  movb $45, buf.141.0.2(%r13) #40.17
  movslq %edi, %r13 #44.19
..B1.5: # Preds ..B1.4 ..B1.3
  movq %r8, %r14 #44.19
  jne ..B1.7 # Prob 50% #45.22
..B1.6: # Preds ..B1.5
  movb $48, (%rsp) #45.25
  movl $1, %edx #50.13
  jmp ..B1.11 # Prob 100% #50.13
..B1.7: # Preds ..B1.5
  testl %r9d, %r9d #46.24
  jle ..B1.17 # Prob 2% #46.24
..B1.9: # Preds ..B1.7 ..B1.9
  movl $1717986919, %eax #47.45
  movl %r9d, %r15d #47.45
  imull %r9d #47.45
  sarl $31, %r15d #47.45
  sarl $2, %edx #47.45
  subl %r15d, %edx #47.45
  lea (%rdx,%rdx,4), %eax #47.45
  addl %eax, %eax #47.45
  subl %eax, %r9d #47.45
  addl $48, %r9d #47.45
  movb %r9b, (%rsp,%r14) #47.17
  movl %edx, %r9d #48.17
  incq %r14 #47.21
  testl %r9d, %r9d #46.24
  jg ..B1.9 # Prob 82% #46.24
..B1.10: # Preds ..B1.9
  movl %r14d, %edx #50.13
  testq %r14, %r14 #50.24
  jle ..B1.17 # Prob 50% #50.24
..B1.11: # Preds ..B1.10 ..B1.6
  movl %edx, %r15d #50.13
  movq %r8, %rax #50.13
  movl $1, %r14d #50.13
  movq %rax, %r9 #50.13
  shrl $1, %r15d #50.13
  je ..B1.15 # Prob 2% #50.13
..B1.12: # Preds ..B1.11
  movslq %edx, %rdx #50.38
  lea (%rsp,%rdx), %rdi #50.38
..B1.13: # Preds ..B1.13 ..B1.12
  movb -1(%r9,%rdi), %r14b #50.38
  movb %r14b, buf.141.0.2(%r13,%rax,2) #50.27
  movb -2(%r9,%rdi), %r14b #50.38
  addq $-2, %r9 #50.13
  movb %r14b, 1+buf.141.0.2(%r13,%rax,2) #50.27
  incq %rax #50.13
  cmpq %r15, %rax #50.13
  jb ..B1.13 # Prob 64% #50.13
..B1.14: # Preds ..B1.13
  lea (%r13,%rax,2), %edi #52.9
  lea 1(%rax,%rax), %r14d #50.44
..B1.15: # Preds ..B1.14 ..B1.11
  lea -1(%r14), %eax #50.13
  cmpl %edx, %eax #50.13
  jae ..B1.20 # Prob 2% #50.13
..B1.16: # Preds ..B1.15
  movslq %edx, %rdx #50.38
  movslq %r14d, %r14 #50.38
  lea (%rsp,%rdx), %rax #50.38
  subq %r14, %rax #50.38
  movb (%rax), %dil #50.38
  movb %dil, -1+buf.141.0.2(%r14,%r13) #50.27
  movl %r13d, %edi #50.31
  addl %r14d, %edi #52.9
  movslq %edi, %r13 #40.17
..B1.17: # Preds ..B1.7 ..B1.16 ..B1.20 ..B1.10
  incl %r11d #36.32
  addl $17, %r10d #36.32
  cmpl $6, %r11d #36.29
  jge ..B1.21 # Prob 16% #36.29
..B1.18: # Preds ..B1.17
  testl %r11d, %r11d #37.17
  je ..B1.3 # Prob 50% #37.17
..B1.19: # Preds ..B1.18
  incl %edi #37.24
  movb $44, buf.141.0.2(%r13) #37.20
  movslq %edi, %r13 #40.17
  jmp ..B1.3 # Prob 100% #40.17
..B1.20: # Preds ..B1.15
  movslq %edi, %r13 #40.17
  jmp ..B1.17 # Prob 100% #40.17
..B1.21: # Preds ..B1.17
  incb %sil #35.29
  addl $131, %ecx #35.29
  incl %edi #52.13
  movb $10, buf.141.0.2(%r13) #52.9
  cmpb $64, %sil #35.25
  jl ..B1.2 # Prob 98% #35.25
..B1.22: # Preds ..B1.21
  movslq %edi, %rdi #54.5
  movq %r8, %rsi #5.16
  movl %ebx, %r11d #6.13
  movl $buf.141.0.2, %r15d #55.29
  movq %rsi, %rdx #7.14
  movl %r11d, %r10d #8.13
  movb $0, buf.141.0.2(%rdi) #54.5
  movsbl buf.141.0.2(%rip), %edi #10.19
  movl %edi, %ecx #12.31
  movl %ecx, %eax #12.31
  lea -48(%rdi), %r9d #11.18
  cmpl $9, %r9d #11.18
  ja ..B1.29 # Prob 50% #11.18
..B1.25: # Preds ..B1.34 ..B1.22 ..B1.25
  movslq %ecx, %rcx #12.35
  incq %r15 #9.13
  lea (%rdx,%rdx,4), %rdx #12.25
  lea -48(%rcx,%rdx,2), %rdx #12.35
  movsbl (%r15), %ecx #10.19
  lea -48(%rcx), %r14d #11.18
  cmpl $9, %r14d #11.18
  jbe ..B1.25 # Prob 50% #11.18
..B1.26: # Preds ..B1.25
  testl %r11d, %r11d #15.30
  jne ..B1.29 # Prob 50% #15.30
..B1.28: # Preds ..B1.26
  movq %rdx, %r14 #15.53
  negq %r14 #15.53
  testl %r10d, %r10d #15.37
  cmovne %r14, %rdx #15.37
  addq %rdx, %rsi #15.37
..B1.29: # Preds ..B1.28 ..B1.22 ..B1.26 ..B1.34
  movq %r8, %rdx #16.13
  movl %ebx, %r10d #17.13
  cmpl $44, %ecx #19.17
  je ..B1.68 # Prob 20% #19.17
..B1.30: # Preds ..B1.29
  testl %ecx, %ecx #19.17
  je ..B1.33 # Prob 25% #19.17
..B1.31: # Preds ..B1.30
  cmpl $10, %ecx #19.17
  je ..B1.33 # Prob 33% #19.17
..B1.32: # Preds ..B1.31
  cmpl $45, %ecx #25.17
  movl %ebx, %r10d #25.17
  sete %r10b #25.17
  jmp ..B1.34 # Prob 100% #25.17
..B1.33: # Preds ..B1.31 ..B1.30
  movl %ebx, %r11d #22.17
  testl %ecx, %ecx #23.26
  je ..B1.36 # Prob 20% #23.26
..B1.34: # Preds ..B1.33 ..B1.32 ..B1.68
  incq %r15 #9.13
  movsbl (%r15), %ecx #10.19
  lea -48(%rcx), %r14d #11.18
  cmpl $9, %r14d #11.18
  ja ..B1.29 # Prob 50% #11.18
  jmp ..B1.25 # Prob 100% #11.18
..B1.36: # Preds ..B1.33
  movq %r8, %rdx #5.16
  movl %ebx, %r11d #6.13
  movl $buf.141.0.2, %r15d #55.49
  movq %rdx, %rcx #7.14
  movl %r11d, %r10d #8.13
  cmpl $9, %r9d #11.18
  ja ..B1.43 # Prob 50% #11.18
..B1.39: # Preds ..B1.48 ..B1.36 ..B1.39
  movslq %eax, %rax #12.35
  incq %r15 #9.13
  lea (%rcx,%rcx,4), %rcx #12.25
  lea -48(%rax,%rcx,2), %rcx #12.35
  movsbl (%r15), %eax #10.19
  lea -48(%rax), %r14d #11.18
  cmpl $9, %r14d #11.18
  jbe ..B1.39 # Prob 50% #11.18
..B1.40: # Preds ..B1.39
  cmpl $3, %r11d #15.30
  jne ..B1.43 # Prob 50% #15.30
..B1.42: # Preds ..B1.40
  movq %rcx, %r14 #15.53
  negq %r14 #15.53
  testl %r10d, %r10d #15.37
  cmovne %r14, %rcx #15.37
  addq %rcx, %rdx #15.37
..B1.43: # Preds ..B1.42 ..B1.36 ..B1.40 ..B1.48
  movq %r8, %rcx #16.13
  movl %ebx, %r10d #17.13
  cmpl $44, %eax #19.17
  je ..B1.67 # Prob 20% #19.17
..B1.44: # Preds ..B1.43
  testl %eax, %eax #19.17
  je ..B1.47 # Prob 25% #19.17
..B1.45: # Preds ..B1.44
  cmpl $10, %eax #19.17
  je ..B1.47 # Prob 33% #19.17
..B1.46: # Preds ..B1.45
  cmpl $45, %eax #25.17
  movl %ebx, %r10d #25.17
  sete %r10b #25.17
  jmp ..B1.48 # Prob 100% #25.17
..B1.47: # Preds ..B1.45 ..B1.44
  movl %ebx, %r11d #22.17
  testl %eax, %eax #23.26
  je ..B1.50 # Prob 20% #23.26
..B1.48: # Preds ..B1.47 ..B1.46 ..B1.67
  incq %r15 #9.13
  movsbl (%r15), %eax #10.19
  lea -48(%rax), %r14d #11.18
  cmpl $9, %r14d #11.18
  ja ..B1.43 # Prob 50% #11.18
  jmp ..B1.39 # Prob 100% #11.18
..B1.50: # Preds ..B1.47
  movq %r8, %rcx #5.16
  movl %ebx, %r11d #6.13
  movl $buf.141.0.2, %r15d #56.12
  movq %rcx, %rax #7.14
  movl %r11d, %r10d #8.13
  cmpl $9, %r9d #11.18
  ja ..B1.57 # Prob 50% #11.18
..B1.53: # Preds ..B1.62 ..B1.50 ..B1.53
  movslq %edi, %rdi #12.35
  incq %r15 #9.13
  lea (%rax,%rax,4), %rax #12.25
  lea -48(%rdi,%rax,2), %rax #12.35
  movsbl (%r15), %edi #10.19
  lea -48(%rdi), %r9d #11.18
  cmpl $9, %r9d #11.18
  jbe ..B1.53 # Prob 50% #11.18
..B1.54: # Preds ..B1.53
  cmpl $5, %r11d #15.30
  jne ..B1.57 # Prob 50% #15.30
..B1.56: # Preds ..B1.54
  movq %rax, %r9 #15.53
  negq %r9 #15.53
  testl %r10d, %r10d #15.37
  cmovne %r9, %rax #15.37
  addq %rax, %rcx #15.37
..B1.57: # Preds ..B1.56 ..B1.50 ..B1.54 ..B1.62
  movq %r8, %rax #16.13
  movl %ebx, %r10d #17.13
  cmpl $44, %edi #19.17
  je ..B1.66 # Prob 20% #19.17
..B1.58: # Preds ..B1.57
  testl %edi, %edi #19.17
  je ..B1.61 # Prob 25% #19.17
..B1.59: # Preds ..B1.58
  cmpl $10, %edi #19.17
  je ..B1.61 # Prob 33% #19.17
..B1.60: # Preds ..B1.59
  cmpl $45, %edi #25.17
  movl %ebx, %r10d #25.17
  sete %r10b #25.17
  jmp ..B1.62 # Prob 100% #25.17
..B1.61: # Preds ..B1.59 ..B1.58
  movl %ebx, %r11d #22.17
  testl %edi, %edi #23.26
  je ..B1.64 # Prob 20% #23.26
..B1.62: # Preds ..B1.61 ..B1.60 ..B1.66
  incq %r15 #9.13
  movsbl (%r15), %edi #10.19
  lea -48(%rdi), %r9d #11.18
  cmpl $9, %r9d #11.18
  ja ..B1.57 # Prob 50% #11.18
  jmp ..B1.53 # Prob 100% #11.18
..B1.64: # Preds ..B1.61
  movl $.L_2__STRING.0, %edi #55.5
  xorl %eax, %eax #55.5
  call printf #55.5
..B1.65: # Preds ..B1.64
  xorl %eax, %eax #57.12
  addq $96, %rsp #57.12
  popq %rbx #57.12
  popq %r15 #57.12
  popq %r14 #57.12
  popq %r13 #57.12
  movq %rbp, %rsp #57.12
  popq %rbp #57.12
  ret #57.12
..B1.66: # Preds ..B1.57
  incl %r11d #20.17
  jmp ..B1.62 # Prob 100% #20.17
..B1.67: # Preds ..B1.43
  incl %r11d #20.17
  jmp ..B1.48 # Prob 100% #20.17
..B1.68: # Preds ..B1.29
  incl %r11d #20.17
  jmp ..B1.34 # Prob 100% #20.17
buf.141.0.2:
.L_2__STRING.0:
