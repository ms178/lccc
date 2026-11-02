main:
..B1.1: # Preds ..B1.0
  pushq %rbp #32.16
  movq %rsp, %rbp #32.16
  andq $-128, %rsp #32.16
  pushq %r12 #32.16
  pushq %r13 #32.16
  pushq %r14 #32.16
  pushq %r15 #32.16
  pushq %rbx #32.16
  subq $88, %rsp #32.16
  movl $3, %edi #32.16
  xorl %esi, %esi #32.16
  call __intel_new_feature_proc_init #32.16
..B1.74: # Preds ..B1.1
  stmxcsr (%rsp) #32.16
  xorl %r11d, %r11d #34.11
  xorb %r15b, %r15b #35.5
  orl $32832, (%rsp) #32.16
  xorl %edi, %edi #34.11
  ldmxcsr (%rsp) #32.16
  xorl %r10d, %r10d #35.5
  xorl %ebx, %ebx #35.5
..B1.2: # Preds ..B1.23 ..B1.74
  movslq %r11d, %r8 #40.17
  movl %edi, %esi #36.9
  movl %r10d, %r9d #36.9
..B1.3: # Preds ..B1.2 ..B1.21 ..B1.20
  movl $274877907, %eax #38.42
  movl %r9d, %ecx #38.42
  imull %r9d #38.42
  sarl $31, %ecx #38.42
  sarl $6, %edx #38.42
  subl %ecx, %edx #38.42
  imull $-1000, %edx, %r12d #38.42
  lea -500(%r9,%r12), %r12d #38.49
  testl %r12d, %r12d #39.21
  jns ..B1.5 # Prob 76% #39.21
..B1.4: # Preds ..B1.3
  incl %r11d #40.21
  negl %r12d #41.22
  movb $45, buf.141.0.2(%r8) #40.17
  movslq %r11d, %r8 #44.19
..B1.5: # Preds ..B1.4 ..B1.3
  jne ..B1.7 # Prob 50% #45.22
..B1.6: # Preds ..B1.5
  movb $48, (%rsp) #45.25
  movl $1, %ecx #50.13
  jmp ..B1.13 # Prob 100% #50.13
..B1.7: # Preds ..B1.5
  testl %r12d, %r12d #46.24
  jle ..B1.19 # Prob 2% #46.24
..B1.8: # Preds ..B1.7
  movq %rbx, %r13 #46.13
..B1.9: # Preds ..B1.10 ..B1.8
  movl $1717986919, %eax #47.45
  movl %r12d, %ecx #47.45
  imull %r12d #47.45
  movl %edx, %r14d #47.45
  incq %r13 #46.13
  sarl $2, %r14d #47.45
  sarl $31, %ecx #47.45
  subl %ecx, %r14d #47.45
  lea (%r14,%r14,4), %eax #47.45
  addl %eax, %eax #47.45
  subl %eax, %r12d #47.45
  addl $48, %r12d #47.45
  movb %r12b, -2(%rsp,%r13,2) #47.17
  lea (%r13,%r13), %eax #50.44
  lea -1(%r13,%r13), %ecx #47.21
  testl %r14d, %r14d #46.24
  jle ..B1.12 # Prob 18% #46.24
..B1.10: # Preds ..B1.9
  movl %eax, %ecx #47.21
  movl $1717986919, %eax #47.45
  imull %r14d #47.45
  movl %edx, %r12d #47.45
  movl %r14d, %eax #47.45
  sarl $2, %r12d #47.45
  sarl $31, %eax #47.45
  subl %eax, %r12d #47.45
  lea (%r12,%r12,4), %edx #47.45
  addl %edx, %edx #47.45
  subl %edx, %r14d #47.45
  addl $48, %r14d #47.45
  movb %r14b, -1(%rsp,%r13,2) #47.17
  testl %r12d, %r12d #46.24
  jg ..B1.9 # Prob 82% #46.24
..B1.12: # Preds ..B1.9 ..B1.10
  testl %ecx, %ecx #50.24
  jle ..B1.19 # Prob 50% #50.24
..B1.13: # Preds ..B1.12 ..B1.6
  movl %ecx, %edx #50.13
  movq %rbx, %r12 #50.13
  movl $1, %eax #50.13
  movq %r12, %r13 #50.13
  shrl $1, %edx #50.13
  je ..B1.17 # Prob 2% #50.13
..B1.14: # Preds ..B1.13
  movslq %ecx, %rcx #50.38
  lea (%rsp,%rcx), %rax #50.38
..B1.15: # Preds ..B1.15 ..B1.14
  movb -1(%r13,%rax), %r11b #50.38
  movb -2(%r13,%rax), %r14b #50.38
  addq $-2, %r13 #50.13
  movb %r11b, buf.141.0.2(%r8,%r12,2) #50.27
  movb %r14b, 1+buf.141.0.2(%r8,%r12,2) #50.27
  incq %r12 #50.13
  cmpq %rdx, %r12 #50.13
  jb ..B1.15 # Prob 64% #50.13
..B1.16: # Preds ..B1.15
  lea (%r8,%r12,2), %r11d #52.9
  lea 1(%r12,%r12), %eax #50.44
..B1.17: # Preds ..B1.16 ..B1.13
  lea -1(%rax), %edx #50.13
  cmpl %ecx, %edx #50.13
  jae ..B1.22 # Prob 2% #50.13
..B1.18: # Preds ..B1.17
  movslq %ecx, %rcx #50.38
  movslq %eax, %rax #50.38
  lea (%rsp,%rcx), %rdx #50.38
  subq %rax, %rdx #50.38
  movb (%rdx), %r11b #50.38
  movb %r11b, -1+buf.141.0.2(%rax,%r8) #50.27
  movl %r8d, %r11d #50.31
  addl %eax, %r11d #52.9
  movslq %r11d, %r8 #40.17
..B1.19: # Preds ..B1.7 ..B1.18 ..B1.22 ..B1.12
  incl %esi #36.9
  addl $17, %r9d #36.9
  cmpl $6, %esi #36.9
  jae ..B1.23 # Prob 16% #36.9
..B1.20: # Preds ..B1.19
  testl %esi, %esi #37.17
  je ..B1.3 # Prob 50% #37.17
..B1.21: # Preds ..B1.20
  incl %r11d #37.24
  movb $44, buf.141.0.2(%r8) #37.20
  movslq %r11d, %r8 #40.17
  jmp ..B1.3 # Prob 100% #40.17
..B1.22: # Preds ..B1.17
  movslq %r11d, %r8 #40.17
  jmp ..B1.19 # Prob 100% #40.17
..B1.23: # Preds ..B1.19
  incb %r15b #35.5
  incl %r11d #52.13
  addl $131, %r10d #35.5
  movb $10, buf.141.0.2(%r8) #52.9
  cmpb $64, %r15b #35.5
  jb ..B1.2 # Prob 98% #35.5
..B1.24: # Preds ..B1.23
  movslq %r11d, %r11 #54.5
  movq %rbx, %rsi #5.16
  movl $buf.141.0.2, %r15d #55.29
  movq %rsi, %rdx #7.14
  movb $0, buf.141.0.2(%r11) #54.5
  movl %edi, %r11d #6.13
  movsbl buf.141.0.2(%rip), %r8d #10.19
  movl %r8d, %ecx #12.31
  movl %r11d, %r10d #8.13
  movl %ecx, %eax #12.31
  lea -48(%r8), %r9d #11.18
  cmpl $9, %r9d #11.18
  ja ..B1.31 # Prob 50% #11.18
..B1.27: # Preds ..B1.36 ..B1.24 ..B1.27
  movslq %ecx, %rcx #12.35
  incq %r15 #9.13
  lea (%rdx,%rdx,4), %rdx #12.25
  lea -48(%rcx,%rdx,2), %rdx #12.35
  movsbl (%r15), %ecx #10.19
  lea -48(%rcx), %r14d #11.18
  cmpl $9, %r14d #11.18
  jbe ..B1.27 # Prob 50% #11.18
..B1.28: # Preds ..B1.27
  testl %r11d, %r11d #15.30
  jne ..B1.31 # Prob 50% #15.30
..B1.30: # Preds ..B1.28
  movq %rdx, %r14 #15.53
  negq %r14 #15.53
  testl %r10d, %r10d #15.37
  cmovne %r14, %rdx #15.37
  addq %rdx, %rsi #15.37
..B1.31: # Preds ..B1.30 ..B1.24 ..B1.28 ..B1.36
  movq %rbx, %rdx #16.13
  movl %edi, %r10d #17.13
  cmpl $44, %ecx #19.17
  je ..B1.70 # Prob 20% #19.17
..B1.32: # Preds ..B1.31
  testl %ecx, %ecx #19.17
  je ..B1.35 # Prob 25% #19.17
..B1.33: # Preds ..B1.32
  cmpl $10, %ecx #19.17
  je ..B1.35 # Prob 33% #19.17
..B1.34: # Preds ..B1.33
  cmpl $45, %ecx #25.17
  movl %edi, %r10d #25.17
  sete %r10b #25.17
  jmp ..B1.36 # Prob 100% #25.17
..B1.35: # Preds ..B1.32 ..B1.33
  movl %edi, %r11d #22.17
  testl %ecx, %ecx #23.26
  je ..B1.38 # Prob 20% #23.26
..B1.36: # Preds ..B1.35 ..B1.34 ..B1.70
  incq %r15 #9.13
  movsbl (%r15), %ecx #10.19
  lea -48(%rcx), %r14d #11.18
  cmpl $9, %r14d #11.18
  ja ..B1.31 # Prob 50% #11.18
  jmp ..B1.27 # Prob 100% #11.18
..B1.38: # Preds ..B1.35
  movq %rbx, %rdx #5.16
  movl %edi, %r11d #6.13
  movl $buf.141.0.2, %r15d #55.49
  movq %rdx, %rcx #7.14
  movl %r11d, %r10d #8.13
  cmpl $9, %r9d #11.18
  ja ..B1.45 # Prob 50% #11.18
..B1.41: # Preds ..B1.50 ..B1.38 ..B1.41
  movslq %eax, %rax #12.35
  incq %r15 #9.13
  lea (%rcx,%rcx,4), %rcx #12.25
  lea -48(%rax,%rcx,2), %rcx #12.35
  movsbl (%r15), %eax #10.19
  lea -48(%rax), %r14d #11.18
  cmpl $9, %r14d #11.18
  jbe ..B1.41 # Prob 50% #11.18
..B1.42: # Preds ..B1.41
  cmpl $3, %r11d #15.30
  jne ..B1.45 # Prob 50% #15.30
..B1.44: # Preds ..B1.42
  movq %rcx, %r14 #15.53
  negq %r14 #15.53
  testl %r10d, %r10d #15.37
  cmovne %r14, %rcx #15.37
  addq %rcx, %rdx #15.37
..B1.45: # Preds ..B1.44 ..B1.38 ..B1.42 ..B1.50
  movq %rbx, %rcx #16.13
  movl %edi, %r10d #17.13
  cmpl $44, %eax #19.17
  je ..B1.69 # Prob 20% #19.17
..B1.46: # Preds ..B1.45
  testl %eax, %eax #19.17
  je ..B1.49 # Prob 25% #19.17
..B1.47: # Preds ..B1.46
  cmpl $10, %eax #19.17
  je ..B1.49 # Prob 33% #19.17
..B1.48: # Preds ..B1.47
  cmpl $45, %eax #25.17
  movl %edi, %r10d #25.17
  sete %r10b #25.17
  jmp ..B1.50 # Prob 100% #25.17
..B1.49: # Preds ..B1.47 ..B1.46
  movl %edi, %r11d #22.17
  testl %eax, %eax #23.26
  je ..B1.52 # Prob 20% #23.26
..B1.50: # Preds ..B1.49 ..B1.48 ..B1.69
  incq %r15 #9.13
  movsbl (%r15), %eax #10.19
  lea -48(%rax), %r14d #11.18
  cmpl $9, %r14d #11.18
  ja ..B1.45 # Prob 50% #11.18
  jmp ..B1.41 # Prob 100% #11.18
..B1.52: # Preds ..B1.49
  movq %rbx, %rcx #5.16
  movl %edi, %r10d #6.13
  movl $buf.141.0.2, %r11d #56.12
  movq %rcx, %rax #7.14
  cmpl $9, %r9d #11.18
  movl %r10d, %r9d #8.13
  ja ..B1.59 # Prob 50% #11.18
..B1.55: # Preds ..B1.64 ..B1.52 ..B1.55
  movslq %r8d, %r8 #12.35
  incq %r11 #9.13
  lea (%rax,%rax,4), %rax #12.25
  lea -48(%r8,%rax,2), %rax #12.35
  movsbl (%r11), %r8d #10.19
  lea -48(%r8), %r15d #11.18
  cmpl $9, %r15d #11.18
  jbe ..B1.55 # Prob 50% #11.18
..B1.56: # Preds ..B1.55
  cmpl $5, %r10d #15.30
  jne ..B1.59 # Prob 50% #15.30
..B1.58: # Preds ..B1.56
  movq %rax, %r15 #15.53
  negq %r15 #15.53
  testl %r9d, %r9d #15.37
  cmovne %r15, %rax #15.37
  addq %rax, %rcx #15.37
..B1.59: # Preds ..B1.58 ..B1.52 ..B1.56 ..B1.64
  movq %rbx, %rax #16.13
  movl %edi, %r9d #17.13
  cmpl $44, %r8d #19.17
  je ..B1.68 # Prob 20% #19.17
..B1.60: # Preds ..B1.59
  testl %r8d, %r8d #19.17
  je ..B1.63 # Prob 25% #19.17
..B1.61: # Preds ..B1.60
  cmpl $10, %r8d #19.17
  je ..B1.63 # Prob 33% #19.17
..B1.62: # Preds ..B1.61
  cmpl $45, %r8d #25.17
  movl %edi, %r9d #25.17
  sete %r9b #25.17
  jmp ..B1.64 # Prob 100% #25.17
..B1.63: # Preds ..B1.61 ..B1.60
  movl %edi, %r10d #22.17
  testl %r8d, %r8d #23.26
  je ..B1.66 # Prob 20% #23.26
..B1.64: # Preds ..B1.63 ..B1.62 ..B1.68
  incq %r11 #9.13
  movsbl (%r11), %r8d #10.19
  lea -48(%r8), %r15d #11.18
  cmpl $9, %r15d #11.18
  ja ..B1.59 # Prob 50% #11.18
  jmp ..B1.55 # Prob 100% #11.18
..B1.66: # Preds ..B1.63
  movl $.L_2__STRING.0, %edi #55.5
  xorl %eax, %eax #55.5
  call printf #55.5
..B1.67: # Preds ..B1.66
  xorl %eax, %eax #57.12
  addq $88, %rsp #57.12
  popq %rbx #57.12
  popq %r15 #57.12
  popq %r14 #57.12
  popq %r13 #57.12
  popq %r12 #57.12
  movq %rbp, %rsp #57.12
  popq %rbp #57.12
  ret #57.12
..B1.68: # Preds ..B1.59
  incl %r10d #20.17
  jmp ..B1.64 # Prob 100% #20.17
..B1.69: # Preds ..B1.45
  incl %r11d #20.17
  jmp ..B1.50 # Prob 100% #20.17
..B1.70: # Preds ..B1.31
  incl %r11d #20.17
  jmp ..B1.36 # Prob 100% #20.17
buf.141.0.2:
.L_2__STRING.0:
