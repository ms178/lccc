main:
..B1.1: # Preds ..B1.0
  pushq %rbp #81.1
  movq %rsp, %rbp #81.1
  andq $-128, %rsp #81.1
  pushq %r14 #81.1
  pushq %r15 #81.1
  pushq %rbx #81.1
  subq $104, %rsp #81.1
  movl $3, %edi #81.1
  xorl %esi, %esi #81.1
  call __intel_new_feature_proc_init #81.1
..B1.32: # Preds ..B1.1
  stmxcsr (%rsp) #81.1
  xorl %esi, %esi #83.16
  movl $324508639, %eax #84.13
  orl $32832, (%rsp) #81.1
  xorl %ecx, %ecx #69.8
  ldmxcsr (%rsp) #81.1
  movl $1374036596, %edx #70.5
..B1.2: # Preds ..B1.5 ..B1.4 ..B1.32
  movl %edx, %edi #75.33
  shrl $24, %edi #75.33
  movb %dil, buffer(%rcx) #75.7
  movl %ecx, %edi #69.32
  jmp ..B1.3 # Prob 100% #69.32
..B1.6: # Preds ..B1.5
  movl %edx, %r8d #73.45
  shrl $28, %r8d #73.45
  lea -64(%rdi,%r8), %r9d #73.45
  movb buffer(%r9), %r10b #73.19
  movb %r10b, buffer(%rcx) #73.7
..B1.3: # Preds ..B1.2 ..B1.6
  incl %edi #69.32
  movl %edi, %ecx #69.32
  cmpq $1048576, %rcx #69.19
  jae ..B1.9 # Prob 18% #69.19
..B1.4: # Preds ..B1.3
  imull $1664525, %edx, %edx #70.21
  addl $1013904223, %edx #70.32
  testb $7, %dl #72.18
  jne ..B1.2 # Prob 50% #72.28
..B1.5: # Preds ..B1.4
  cmpl $64, %edi #72.38
  jae ..B1.6 # Prob 50% #72.38
  jmp ..B1.2 # Prob 100% #72.38
..B1.9: # Preds ..B1.3
  xorb %dl, %dl #88.8
..B1.10: # Preds ..B1.25 ..B1.9
  xorl %r11d, %r11d #89.10
..B1.11: # Preds ..B1.24 ..B1.10
  imull $1664525, %eax, %eax #93.23
  addl $1013904223, %eax #93.34
  movl %eax, %ecx #96.33
  movl %eax, %r10d #94.25
  andl $127, %ecx #96.33
  andl $1048319, %r10d #94.25
  addl $16, %ecx #96.33
  movl %eax, %r8d #95.27
  shrl $12, %r8d #95.27
  lea buffer(%r10), %r9 #98.26
  andl $1048319, %r8d #95.33
  lea buffer(%r10,%rcx), %rcx #99.26
  movq %r9, %rdi #42.28
  lea -7(%rcx), %r14 #43.45
  lea buffer(%r8), %r8 #98.44
  cmpq %r14, %r9 #45.16
  jae ..B1.18 # Prob 10% #45.16
..B1.13: # Preds ..B1.11 ..B1.16
  movq (%r8), %rbx #35.3
..B1.14: # Preds ..B1.13
  movq (%r9), %r15 #35.3
..B1.15: # Preds ..B1.14
  xorq %r15, %rbx #46.39
  jne ..B1.29 # Prob 20% #47.10
..B1.16: # Preds ..B1.15
  addq $8, %r9 #48.7
  addq $8, %r8 #49.7
  cmpq %r14, %r9 #45.16
  jb ..B1.13 # Prob 82% #45.16
..B1.18: # Preds ..B1.16 ..B1.11
  cmpq %rcx, %r9 #56.17
  jae ..B1.23 # Prob 10% #56.17
..B1.20: # Preds ..B1.18 ..B1.21
  movb (%r9), %bl #56.32
  cmpb (%r8), %bl #56.40
  jne ..B1.23 # Prob 20% #56.40
..B1.21: # Preds ..B1.20
  incq %r9 #57.5
  incq %r8 #58.5
  cmpq %rcx, %r9 #56.17
  jb ..B1.20 # Prob 82% #56.17
..B1.23: # Preds ..B1.20 ..B1.21 ..B1.18
  subq %rdi, %r9 #60.25
  movl %r9d, %r9d #60.31
..B1.24: # Preds ..B1.23 ..B1.29
  lea (,%r11,8), %ecx #100.45
  shlq %cl, %r9 #100.45
  incl %r11d #89.34
  xorq %r10, %r9 #100.57
  addq %r9, %rsi #100.7
  cmpl $131072, %r11d #89.21
  jb ..B1.11 # Prob 99% #89.21
..B1.25: # Preds ..B1.24
  incb %dl #88.33
  cmpb $16, %dl #88.25
  jb ..B1.10 # Prob 93% #88.25
..B1.26: # Preds ..B1.25
  movl $.L_2__STRING.0, %edi #104.3
  xorl %eax, %eax #104.3
  call printf #104.3
..B1.27: # Preds ..B1.26
  xorl %eax, %eax #105.10
  addq $104, %rsp #105.10
  popq %rbx #105.10
  popq %r15 #105.10
  popq %r14 #105.10
  movq %rbp, %rsp #105.10
  popq %rbp #105.10
  ret #105.10
..B1.29: # Preds ..B1.15
  movl $64, %ecx #52.12
  bsf %rbx, %rbx #52.12
  cmove %rcx, %rbx #52.12
  subq %rdi, %r9 #53.27
  sarl $3, %ebx #52.37
  movslq %ebx, %rbx #52.37
  addq %rbx, %r9 #52.5
  movl %r9d, %r9d #53.33
  jmp ..B1.24 # Prob 100% #53.33
buffer:
.L_2__STRING.0:
