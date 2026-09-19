main:
..B1.1: # Preds ..B1.0
  pushq %rbp #33.16
  movq %rsp, %rbp #33.16
  andq $-128, %rsp #33.16
  pushq %r12 #33.16
  pushq %r13 #33.16
  pushq %r14 #33.16
  pushq %r15 #33.16
  pushq %rbx #33.16
  subq $344, %rsp #33.16
  movl $3, %edi #33.16
  xorl %esi, %esi #33.16
  call __intel_new_feature_proc_init #33.16
..B1.42: # Preds ..B1.1
  stmxcsr (%rsp) #33.16
  xorl %edi, %edi #38.30
  xorl %ecx, %ecx #38.30
  orl $32832, (%rsp) #33.16
  xorl %r8d, %r8d #38.30
  ldmxcsr (%rsp) #33.16
  movl seed.365.0.3(%rip), %esi #38.30
..B1.2: # Preds ..B1.6 ..B1.42
  imull $1664525, %esi, %r10d #16.15
  movl $381774871, %eax #38.38
  lea 1013904223(%r10), %esi #16.26
  movl %esi, seed.365.0.3(%rip) #38.30
  movl %esi, %ebx #38.38
  shrl $2, %ebx #38.38
  mull %ebx #38.38
  movq %r8, %rbx #39.9
  shrl $2, %edx #38.38
  imull $-180, %edx, %r9d #38.38
  lea 1013904233(%r9,%r10), %r9d #38.38
..B1.3: # Preds ..B1.2
  movq %rdi, %r10 #40.13
..B1.4: # Preds ..B1.4 ..B1.3
  imull $1664525, %esi, %r11d #16.15
  movl $1321528399, %eax #40.49
  incq %rbx #39.9
  lea 1013904223(%r11), %esi #16.26
  mull %esi #40.49
  shrl $3, %edx #40.49
  imull $-26, %edx, %eax #40.49
  lea 1013904320(%rax,%r11), %r11d #40.49
  movb %r11b, strings(%r10) #40.13
  incq %r10 #39.9
  cmpq %r9, %rbx #39.9
  jb ..B1.4 # Prob 99% #39.9
..B1.5: # Preds ..B1.4
  movl %esi, seed.365.0.3(%rip) #40.41
..B1.6: # Preds ..B1.5
  incl %ecx #38.30
  movb $0, strings(%r9,%rdi) #41.9
  addq $200, %rdi #38.30
  cmpl $100000, %ecx #38.30
  jb ..B1.2 # Prob 99% #38.30
..B1.7: # Preds ..B1.6
  movq %r8, %r9 #45.20
  xorb %dil, %dil #46.18
..B1.8: # Preds ..B1.10 ..B1.7
  xorl %ebx, %ebx #47.20
  movl $strings, %esi #48.33
..B1.9: # Preds ..B1.43 ..B1.8
  movq %rsi, %rdx #48.26
  movq %rdx, %rcx #48.26
  andq $-16, %rdx #48.26
  pxor %xmm0, %xmm0 #48.26
  pcmpeqb (%rdx), %xmm0 #48.26
  pmovmskb %xmm0, %eax #48.26
  andl $15, %ecx #48.26
  shrl %cl, %eax #48.26
  bsf %eax, %eax #48.26
  jne ..L17 # Prob 60% #48.26
  movq %rdx, %rax #48.26
  addq %rcx, %rdx #48.26
  call *__intel_sse2_strlen@GOTPCREL(%rip) #48.26
..L17: #
..B1.43: # Preds ..B1.9
  incl %ebx #47.39
  addq %rax, %r9 #48.13
  addq $200, %rsi #47.39
  cmpl $100000, %ebx #47.29
  jl ..B1.9 # Prob 99% #47.29
..B1.10: # Preds ..B1.43
  incb %dil #46.33
  cmpb $50, %dil #46.29
  jl ..B1.8 # Prob 98% #46.29
..B1.11: # Preds ..B1.10
  movq %r8, %rcx #51.18
  xorl %edx, %edx #52.16
  movq %rcx, %rax #52.16
  movl %edx, %r15d #52.16
  movq %rax, %rbx #52.16
..B1.12: # Preds ..B1.44 ..B1.11
  lea strings(%rbx), %rdi #56.19
  lea 200+strings(%rbx), %rsi #56.19
..L19: #56.19
  movb (%rdi), %dl #56.19
  cmpb (%rsi), %dl #56.19
  jne ..L21 # Prob 50% #56.19
  testb %dl, %dl #56.19
  je ..L20 # Prob 50% #56.19
  movb 1(%rdi), %dl #56.19
  cmpb 1(%rsi), %dl #56.19
  jne ..L21 # Prob 50% #56.19
  addq $2, %rdi #56.19
  addq $2, %rsi #56.19
  testb %dl, %dl #56.19
  jne ..L19 # Prob 50% #56.19
..L20: #
  xorl %eax, %eax #56.19
  jmp ..L22 # Prob 100% #56.19
..L21: #
  sbbl %eax, %eax #56.19
  orl $1, %eax #56.19
..L22: #
..B1.44: # Preds ..B1.12
  xorl %edi, %edi #57.9
  testl %eax, %eax #57.9
  setg %dil #57.9
  incl %r15d #52.39
  shrl $31, %eax #57.9
  addq $200, %rbx #52.39
  subq %rax, %rdi #57.27
  addq %rdi, %rcx #57.9
  cmpl $99999, %r15d #52.25
  jl ..B1.12 # Prob 99% #52.25
..B1.13: # Preds ..B1.44
  movl $6513249, (%rsp) #62.20
  movq %r8, %rax #61.16
  xorl %r10d, %r10d #63.16
  movl $strings, %edi #64.23
..B1.14: # Preds ..B1.23 ..B1.13
..B1.15: # Preds ..B1.14
  movb (%rdi), %r11b #25.37
  movq %rdi, %rbx #25.26
  testb %r11b, %r11b #25.37
  je ..B1.23 # Prob 4% #25.37
..B1.16: # Preds ..B1.15
  movq %rdi, %r15 #26.25
  lea (%rsp), %r11 #26.33
..B1.18: # Preds ..B1.16 ..B1.34 ..B1.20
  movb (%r11), %sil #27.23
  testb %sil, %sil #27.23
  je ..B1.22 # Prob 20% #27.23
..B1.19: # Preds ..B1.18
  movb (%r15), %dl #27.29
  cmpb %sil, %dl #27.35
  jne ..B1.33 # Prob 20% #27.35
..B1.20: # Preds ..B1.19
  incq %r15 #27.40
  incq %r11 #27.45
  cmpb $0, (%r15) #27.17
  je ..B1.31 # Prob 18% #27.17
  jmp ..B1.18 # Prob 100% #27.17
..B1.22: # Preds ..B1.32 ..B1.18
  cmpq %rbx, %r8 #64.44
  adcq $0, %rax #64.44
..B1.23: # Preds ..B1.33 ..B1.15 ..B1.22
  incl %r10d #63.35
  addq $200, %rdi #63.35
  cmpl $100000, %r10d #63.25
  jl ..B1.14 # Prob 99% #63.25
..B1.24: # Preds ..B1.23
  movq %r8, %r10 #68.19
  xorb %dl, %dl #69.18
  movq %rax, (%rsp) #69.18[spill]
  movb %dl, %bl #69.18
  movq %rcx, 8(%rsp) #69.18[spill]
  movq %r8, %r14 #69.18
  movq %r9, 16(%rsp) #69.18[spill]
  movq %r10, %r15 #69.18
..B1.25: # Preds ..B1.28 ..B1.24
  xorl %r13d, %r13d #70.20
  movq %r14, %r12 #70.20
..B1.26: # Preds ..B1.27 ..B1.25
  lea strings(%r12), %rsi #71.30
  movq %rsi, %rdx #71.23
  movq %rdx, %rcx #71.23
  andq $-16, %rdx #71.23
  pxor %xmm0, %xmm0 #71.23
  pcmpeqb (%rdx), %xmm0 #71.23
  pmovmskb %xmm0, %eax #71.23
  andl $15, %ecx #71.23
  shrl %cl, %eax #71.23
  bsf %eax, %eax #71.23
  jne ..L30 # Prob 60% #71.23
  movq %rdx, %rax #71.23
  addq %rcx, %rdx #71.23
  call *__intel_sse2_strlen@GOTPCREL(%rip) #71.23
..L30: #
..B1.45: # Preds ..B1.26
  movslq %eax, %rdx #72.13
  lea 32(%rsp), %rdi #72.13
  incq %rdx #72.13
  call _intel_fast_memcpy #72.13
..B1.27: # Preds ..B1.45
  incl %r13d #70.39
  addq $200, %r12 #70.39
  movsbq 32(%rsp), %rax #73.25
  addq %rax, %r15 #73.13
  cmpl $100000, %r13d #70.29
  jl ..B1.26 # Prob 99% #70.29
..B1.28: # Preds ..B1.27
  incb %bl #69.33
  cmpb $50, %bl #69.29
  jl ..B1.25 # Prob 98% #69.29
..B1.29: # Preds ..B1.28
  movq %r15, %r10 #
  movl $.L_2__STRING.0, %edi #77.5
  movq (%rsp), %rax #[spill]
  movq %r10, %r8 #77.5
  movq 8(%rsp), %rcx #[spill]
  movq %rcx, %rdx #77.5
  movq 16(%rsp), %r9 #[spill]
  movq %r9, %rsi #77.5
  movq %rax, %rcx #77.5
  xorl %eax, %eax #77.5
  call printf #77.5
..B1.30: # Preds ..B1.29
  xorl %eax, %eax #79.12
  addq $344, %rsp #79.12
  popq %rbx #79.12
  popq %r15 #79.12
  popq %r14 #79.12
  popq %r13 #79.12
  popq %r12 #79.12
  movq %rbp, %rsp #79.12
  popq %rbp #79.12
  ret #79.12
..B1.31: # Preds ..B1.20
  movb (%r11), %r11b #28.15
..B1.32: # Preds ..B1.31
  testb %r11b, %r11b #28.15
  je ..B1.22 # Prob 20% #28.15
..B1.33: # Preds ..B1.19 ..B1.32
  incq %rbx #25.40
  cmpb $0, (%rbx) #25.37
  je ..B1.23 # Prob 18% #25.37
..B1.34: # Preds ..B1.33
  movq %rbx, %r15 #26.25
  lea (%rsp), %r11 #26.33
  jmp ..B1.18 # Prob 100% #26.33
seed.365.0.3:
strings:
.L_2__STRING.0:
