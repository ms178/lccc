main:
..B1.1: # Preds ..B1.0
  pushq %rbp #177.1
  movq %rsp, %rbp #177.1
  andq $-128, %rsp #177.1
  subq $128, %rsp #177.1
  movl $3, %edi #177.1
  xorl %esi, %esi #177.1
  call __intel_new_feature_proc_init #177.1
..B1.67: # Preds ..B1.1
  stmxcsr (%rsp) #177.1
  orl $32832, (%rsp) #177.1
  ldmxcsr (%rsp) #177.1
  movq $0, 16(%rsp) #178.23
..B1.2: # Preds ..B1.67
  xorl %esi, %esi #180.16
  movl $1294694901, %edx #181.22
  movq 16(%rsp), %r8 #184.35
  xorl %ecx, %ecx #180.16
  xorl %r9d, %r9d #183.8
  xorl %edi, %edi #187.5
..B1.3: # Preds ..B1.25 ..B1.2
  imull $1664525, %edx, %edx #185.21
  lea 16(%rsp), %rax #184.35
  shlq $5, %r9 #186.5
  addl $1013904223, %edx #185.32
  movl %edx, %r10d #186.38
  xorl %r11d, %r11d #184.58
  andl $2147483647, %r10d #186.38
  movl %r10d, 24+node_pool(%r9) #186.5
  movl %edi, 28+node_pool(%r9) #187.5
  movq %rcx, 16+node_pool(%r9) #188.5
  movq %rcx, 8+node_pool(%r9) #189.5
  testq %r8, %r8 #191.13
  je ..B1.10 # Prob 1% #191.13
..B1.4: # Preds ..B1.3
  movq 16(%rsp), %r8 #184.35
..B1.5: # Preds ..B1.8 ..B1.4
  movq %r8, %r11 #192.7
  cmpl 24(%r8), %r10d #193.30
  jge ..B1.7 # Prob 50% #193.30
..B1.6: # Preds ..B1.5
  movq (%rax), %rax #194.24
  addq $16, %rax #194.24
  jmp ..B1.8 # Prob 100% #194.24
..B1.7: # Preds ..B1.5
  movq (%rax), %rax #196.24
  addq $8, %rax #196.24
..B1.8: # Preds ..B1.6 ..B1.7
  movq (%rax), %r8 #191.13
  testq %r8, %r8 #191.13
  jne ..B1.5 # Prob 82% #191.13
..B1.10: # Preds ..B1.8 ..B1.3
  movq %r11, node_pool(%r9) #199.26
  lea node_pool(%r9), %r9 #200.18
  movq (%r9), %r8 #84.28
  movq %r9, (%rax) #200.6
  andq $-4, %r8 #84.28
..B1.11: # Preds ..B1.16 ..B1.10
  testq %r8, %r8 #87.10
  je ..B1.64 # Prob 20% #87.10
..B1.12: # Preds ..B1.11
  movq (%r8), %rax #91.9
  testq $1, %rax #91.9
  jne ..B1.63 # Prob 20% #91.9
..B1.13: # Preds ..B1.12
  andq $-4, %rax #94.15
  movq 8(%rax), %r11 #95.11
  cmpq %r11, %r8 #97.19
  je ..B1.47 # Prob 12% #97.19
..B1.14: # Preds ..B1.13
  testq %r11, %r11 #98.11
  je ..B1.18 # Prob 20% #98.11
..B1.15: # Preds ..B1.14
  movq (%r11), %r10 #98.18
  btsq $0, %r10 #98.18
  jc ..B1.18 # Prob 20% #98.18
..B1.16: # Preds ..B1.48 ..B1.15
  movq %r10, (%r11) #99.9
  movq %rax, %r9 #102.9
  orq $1, (%r8) #100.9
  movq (%rax), %r8 #101.9
  andq $-2, %r8 #101.9
  movq %r8, (%rax) #101.9
  andq $-4, %r8 #103.18
  jmp ..B1.11 # Prob 100% #103.18
..B1.18: # Preds ..B1.14 ..B1.15
  movq 8(%r8), %r10 #107.13
  cmpq %r10, %r9 #108.19
  je ..B1.43 # Prob 12% #108.19
..B1.19: # Preds ..B1.45 ..B1.18
  movq %r10, 16(%rax) #119.7
  movq %rax, 8(%r8) #120.7
  testq %r10, %r10 #121.11
  je ..B1.21 # Prob 32% #121.11
..B1.20: # Preds ..B1.19
  lea 1(%rax), %r9 #54.61
  movq %r9, (%r10) #54.3
..B1.21: # Preds ..B1.19 ..B1.20
  movq (%rax), %r9 #75.28
  movq %r9, %r10 #75.28
  movq %r9, (%r8) #76.3
  movq %r8, (%rax) #54.3
  andq $-4, %r10 #75.28
  je ..B1.42 # Prob 12% #61.7
..B1.22: # Preds ..B1.21
  cmpq 16(%r10), %rax #62.28
  je ..B1.24 # Prob 12% #62.28
..B1.23: # Preds ..B1.55 ..B1.22
  movq %r8, 8(%r10) #65.7
  movq 16(%rsp), %r8 #207.42
  jmp ..B1.25 # Prob 100% #207.42
..B1.24: # Preds ..B1.55 ..B1.22
  movq %r8, 16(%r10) #63.7
  movq 16(%rsp), %r8 #207.42
..B1.25: # Preds ..B1.64 ..B1.63 ..B1.24 ..B1.23 ..B1.42
  incl %edi #183.31
  movl %edi, %r9d #183.31
  cmpl $16384, %edi #183.19
  jb ..B1.3 # Prob 99% #183.19
..B1.26: # Preds ..B1.25
  xorb %dl, %dl #204.8
..B1.27: # Preds ..B1.37 ..B1.26
  xorl %eax, %eax #205.10
  xorl %ecx, %ecx #205.14
..B1.28: # Preds ..B1.36 ..B1.27
  movl %ecx, %edi #206.19
  movq %r8, %r9 #161.26
  andq $16383, %rdi #206.45
  shlq $5, %rdi #206.19
  movl 24+node_pool(%rdi), %r10d #206.19
  testq %r8, %r8 #162.10
  je ..B1.36 # Prob 0% #162.10
..B1.30: # Preds ..B1.28 ..B1.34
  movl 24(%r9), %edi #163.15
  cmpl %edi, %r10d #163.15
  jge ..B1.32 # Prob 50% #163.15
..B1.31: # Preds ..B1.30
  movq 16(%r9), %r9 #164.14
  jmp ..B1.34 # Prob 100% #164.14
..B1.32: # Preds ..B1.30
  jle ..B1.40 # Prob 20% #165.20
..B1.33: # Preds ..B1.32
  movq 8(%r9), %r9 #166.14
..B1.34: # Preds ..B1.31 ..B1.33
  testq %r9, %r9 #162.10
  jne ..B1.30 # Prob 82% #162.10
..B1.36: # Preds ..B1.34 ..B1.28 ..B1.41 ..B1.40
  incl %eax #205.33
  addl $104729, %ecx #205.33
  cmpl $16384, %eax #205.21
  jb ..B1.28 # Prob 99% #205.21
..B1.37: # Preds ..B1.36
  incb %dl #204.34
  cmpb $8, %dl #204.19
  jb ..B1.27 # Prob 87% #204.19
..B1.38: # Preds ..B1.37
  movl $.L_2__STRING.0, %edi #213.3
  xorl %eax, %eax #213.3
  call printf #213.3
..B1.39: # Preds ..B1.38
  xorl %eax, %eax #214.10
  movq %rbp, %rsp #214.10
  popq %rbp #214.10
  ret #214.10
..B1.40: # Preds ..B1.32
  testq %r9, %r9 #208.11
  je ..B1.36 # Prob 12% #208.11
..B1.41: # Preds ..B1.40
  movslq %edi, %rdi #209.45
  shlq $16, %rdi #209.59
  movslq 28(%r9), %r9 #209.26
  xorq %rdi, %r9 #209.59
  addq %r9, %rsi #209.9
  jmp ..B1.36 # Prob 100% #209.9
..B1.42: # Preds ..B1.54 ..B1.21
  movq %r8, 16(%rsp) #201.37
  jmp ..B1.25 # Prob 100% #201.37
..B1.43: # Preds ..B1.18
  movq 16(%r9), %r11 #109.15
  movq %r11, 8(%r8) #110.9
  movq %r8, 16(%r9) #111.9
  testq %r11, %r11 #112.13
  je ..B1.45 # Prob 32% #112.13
..B1.44: # Preds ..B1.43
  lea 1(%r8), %r10 #54.61
  movq %r10, (%r11) #54.3
..B1.45: # Preds ..B1.43 ..B1.44
  movq %r9, (%r8) #54.3
  movq %r9, %r8 #115.9
  movq 8(%r9), %r10 #116.15
  jmp ..B1.19 # Prob 100% #116.15
..B1.47: # Preds ..B1.13
  movq 16(%rax), %r11 #126.13
  testq %r11, %r11 #127.11
  je ..B1.51 # Prob 20% #127.11
..B1.48: # Preds ..B1.47
  movq (%r11), %r10 #127.18
  btsq $0, %r10 #127.18
  jnc ..B1.16 # Prob 80% #127.18
..B1.51: # Preds ..B1.47 ..B1.48
  movq 16(%r8), %r10 #136.13
  cmpq %r10, %r9 #137.19
  je ..B1.59 # Prob 12% #137.19
..B1.52: # Preds ..B1.61 ..B1.51
  movq %r10, 8(%rax) #148.7
  movq %rax, 16(%r8) #149.7
  testq %r10, %r10 #150.11
  je ..B1.54 # Prob 32% #150.11
..B1.53: # Preds ..B1.52
  lea 1(%rax), %r9 #54.61
  movq %r9, (%r10) #54.3
..B1.54: # Preds ..B1.52 ..B1.53
  movq (%rax), %r9 #75.28
  movq %r9, %r10 #75.28
  movq %r9, (%r8) #76.3
  movq %r8, (%rax) #54.3
  andq $-4, %r10 #75.28
  je ..B1.42 # Prob 12% #61.7
..B1.55: # Preds ..B1.54
  cmpq 16(%r10), %rax #62.28
  je ..B1.24 # Prob 12% #62.28
  jmp ..B1.23 # Prob 100% #62.28
..B1.59: # Preds ..B1.51
  movq 8(%r9), %r11 #138.15
  movq %r11, 16(%r8) #139.9
  movq %r8, 8(%r9) #140.9
  testq %r11, %r11 #141.13
  je ..B1.61 # Prob 32% #141.13
..B1.60: # Preds ..B1.59
  lea 1(%r8), %r10 #54.61
  movq %r10, (%r11) #54.3
..B1.61: # Preds ..B1.59 ..B1.60
  movq %r9, (%r8) #54.3
  movq %r9, %r8 #144.9
  movq 16(%r9), %r10 #145.15
  jmp ..B1.52 # Prob 100% #145.15
..B1.63: # Preds ..B1.12
  movq 16(%rsp), %r8 #207.42
  jmp ..B1.25 # Prob 100% #207.42
..B1.64: # Preds ..B1.11
  orq $1, (%r9) #88.7
  movq 16(%rsp), %r8 #207.42
  jmp ..B1.25 # Prob 100% #207.42
