main:
..B1.1: # Preds ..B1.0
  pushq %rbp #247.1
  movq %rsp, %rbp #247.1
  andq $-128, %rsp #247.1
  pushq %r13 #247.1
  pushq %r14 #247.1
  pushq %r15 #247.1
  pushq %rbx #247.1
  subq $96, %rsp #247.1
  movl $3, %edi #247.1
  xorl %esi, %esi #247.1
  call __intel_new_feature_proc_init #247.1
..B1.73: # Preds ..B1.1
  stmxcsr (%rsp) #247.1
  xorl %esi, %esi #249.16
  orl $32832, (%rsp) #247.1
  ldmxcsr (%rsp) #247.1
  movq %rsi, %r14 #235.8
  movq %rsi, %rbx #235.8
..B1.2: # Preds ..B1.20 ..B1.73
  movq values.159.0.5(,%r14,8), %r15 #237.45
  movq %r15, %rdx #237.19
  movq $0, 8(%rsp) #236.17
  cmpq $127, %r15 #42.12
  jbe ..B1.16 # Prob 28% #42.12
..B1.3: # Preds ..B1.2
  cmpq $16383, %r15 #46.12
  jbe ..B1.15 # Prob 28% #46.12
..B1.4: # Preds ..B1.3
  movq $0xff00000000000000, %rcx #51.3
  testq %r15, %rcx #51.11
  jne ..B1.66 # Prob 10% #51.11
..B1.5: # Preds ..B1.4
  xorl %r13d, %r13d #61.3
  xorl %eax, %eax #61.7
..B1.6: # Preds ..B1.6 ..B1.5
  movq %rdx, %rcx #63.35
  incl %r13d #63.9
  orq $-128, %rcx #63.35
  movb %cl, 25(%rsp,%rax) #63.5
  incq %rax #63.9
  shrq $7, %rdx #64.5
  jne ..B1.6 # Prob 82% #65.17
..B1.7: # Preds ..B1.6
  movl %r13d, %eax #67.23
  andb $127, 25(%rsp) #66.3
  decl %eax #67.23
  js ..B1.17 # Prob 50% #67.31
..B1.8: # Preds ..B1.7
  movl %r13d, %edi #67.3
  movl $1, %r8d #67.3
  xorl %ecx, %ecx #67.3
  xorl %edx, %edx #68.5
  shrl $1, %edi #67.3
  je ..B1.12 # Prob 2% #67.3
..B1.9: # Preds ..B1.8
  movslq %eax, %rax #68.12
  lea 25(%rsp,%rax), %r8 #68.12
..B1.10: # Preds ..B1.10 ..B1.9
  movb (%rdx,%r8), %r10b #68.12
  lea (%rcx,%rcx), %r9d #68.5
  movslq %r9d, %r9 #68.5
  incl %ecx #67.3
  movb -1(%rdx,%r8), %r11b #68.12
  addq $-2, %rdx #67.3
  movb %r10b, 16(%rsp,%r9) #237.37
  movb %r11b, 17(%rsp,%r9) #237.37
  cmpl %edi, %ecx #67.3
  jb ..B1.10 # Prob 80% #67.3
..B1.11: # Preds ..B1.10
  lea 1(%rcx,%rcx), %r8d #68.5
..B1.12: # Preds ..B1.11 ..B1.8
  lea -1(%r8), %edx #67.3
  cmpl %r13d, %edx #67.3
  jae ..B1.17 # Prob 2% #67.3
..B1.13: # Preds ..B1.12
  movslq %eax, %rax #68.12
  movslq %r8d, %r8 #68.12
  lea 25(%rsp,%rax), %rdx #68.12
  subq %r8, %rdx #68.12
  movb 1(%rdx), %cl #68.12
  movb %cl, 15(%rsp,%r8) #237.37
  jmp ..B1.17 # Prob 100% #237.37
..B1.15: # Preds ..B1.3
  movq %r15, %rdx #47.24
  movq %r15, %rcx #48.21
  shrq $7, %rdx #47.24
  andq $127, %rcx #48.21
  orq $-128, %rdx #47.38
  movl $2, %r13d #237.19
  movb %dl, 16(%rsp) #237.37
  movb %cl, 17(%rsp) #237.37
  jmp ..B1.17 # Prob 100% #237.37
..B1.16: # Preds ..B1.2
  movq %r15, %rdx #43.21
  movl $1, %r13d #237.19
  andq $127, %rdx #43.21
  movb %dl, 16(%rsp) #237.37
..B1.17: # Preds ..B1.13 ..B1.7 ..B1.12 ..B1.16 ..B1.15
  lea 16(%rsp), %rdi #238.15
  lea 8(%rsp), %rsi #238.15
  call sqlite_get_varint #238.15
..B1.18: # Preds ..B1.17
  movzbl %al, %eax #239.14
  cmpl %r13d, %eax #239.22
  jne ..B1.69 # Prob 20% #239.22
..B1.19: # Preds ..B1.18
  cmpq 8(%rsp), %r15 #239.44
  jne ..B1.69 # Prob 20% #239.44
..B1.20: # Preds ..B1.19
  incl %r14d #235.55
  cmpq $11, %r14 #235.19
  jb ..B1.2 # Prob 82% #235.19
..B1.21: # Preds ..B1.20
  xorl %ecx, %ecx #212.21
  movq %rbx, %rsi #
  movl $1831565813, %edi #211.22
  xorl %r8d, %r8d #214.8
..B1.22: # Preds ..B1.56 ..B1.21
  movl $954437177, %eax #185.19
  mull %r8d #185.19
  imull $1664525, %edi, %edi #216.21
  shrl $1, %edx #185.19
  addl $1013904223, %edi #216.32
  lea (%rdx,%rdx,8), %eax #185.19
  negl %eax #185.19
  addl %r8d, %eax #185.19
  cmpl $7, %eax #185.3
  ja ..B1.40 # Prob 28% #185.3
..B1.23: # Preds ..B1.22
  jmp *.2.13_2.switchtab.12(,%rax,8) #185.3
..1.3_0.TAG.7.0.3.9:
..B1.25: # Preds ..B1.23
  movl %edi, %eax #201.39
  shlq $24, %rax #201.48
  btsq $49, %rax #201.48
  jmp ..B1.41 # Prob 100% #201.48
..1.3_0.TAG.6.0.3.9:
..B1.27: # Preds ..B1.23
  movl %edi, %eax #199.37
  shlq $17, %rax #199.46
  btsq $42, %rax #199.46
  jmp ..B1.41 # Prob 100% #199.46
..1.3_0.TAG.5.0.3.9:
..B1.29: # Preds ..B1.23
  movl %edi, %eax #197.35
  shlq $10, %rax #197.44
  btsq $35, %rax #197.44
  jmp ..B1.41 # Prob 100% #197.44
..1.3_0.TAG.4.0.3.9:
..B1.31: # Preds ..B1.23
  movl %edi, %eax #195.34
  shlq $3, %rax #195.43
  orq $268435456, %rax #195.43
  jmp ..B1.41 # Prob 100% #195.43
..1.3_0.TAG.3.0.3.9:
..B1.33: # Preds ..B1.23
  movl %edi, %eax #193.38
  andq $268435455, %rax #193.38
  orq $2097152, %rax #193.38
  jmp ..B1.41 # Prob 100% #193.38
..1.3_0.TAG.2.0.3.9:
..B1.35: # Preds ..B1.23
  movl %edi, %eax #191.36
  andq $2097151, %rax #191.36
  orq $16384, %rax #191.36
  jmp ..B1.41 # Prob 100% #191.36
..1.3_0.TAG.1.0.3.9:
..B1.37: # Preds ..B1.23
  movl %edi, %eax #189.34
  andq $16383, %rax #189.34
  orq $128, %rax #189.34
  jmp ..B1.41 # Prob 100% #189.34
..1.3_0.TAG.0.0.3.9:
..B1.39: # Preds ..B1.23
  movl %edi, %eax #187.26
  andq $127, %rax #187.26
  jmp ..B1.41 # Prob 100% #187.26
..B1.40: # Preds ..B1.22
  movl %edi, %eax #203.42
  shlq $25, %rax #203.51
  btsq $63, %rax #203.51
  orq %r8, %rax #203.57
..B1.41: # Preds ..B1.39 ..B1.37 ..B1.35 ..B1.33 ..B1.31
  movl %r8d, %r8d #218.5
  movl %ecx, %ebx #219.45
  movl %ecx, sqlite_varint_offsets(,%r8,4) #218.5
  cmpq $127, %rax #42.12
  jbe ..B1.55 # Prob 28% #42.12
..B1.42: # Preds ..B1.41
  cmpq $16383, %rax #46.12
  jbe ..B1.54 # Prob 28% #46.12
..B1.43: # Preds ..B1.42
  movq $0xff00000000000000, %rdx #51.3
  testq %rax, %rdx #51.11
  jne ..B1.67 # Prob 10% #51.11
..B1.44: # Preds ..B1.43
  xorl %r15d, %r15d #61.3
  xorl %edx, %edx #61.7
..B1.45: # Preds ..B1.45 ..B1.44
  movq %rax, %r9 #63.35
  incl %r15d #63.9
  orq $-128, %r9 #63.35
  movb %r9b, 25(%rsp,%rdx) #63.5
  incq %rdx #63.9
  shrq $7, %rax #64.5
  jne ..B1.45 # Prob 82% #65.17
..B1.46: # Preds ..B1.45
  movl %r15d, %r9d #67.23
  andb $127, 25(%rsp) #66.3
  decl %r9d #67.23
  js ..B1.56 # Prob 50% #67.31
..B1.47: # Preds ..B1.46
  movl %r15d, %edx #67.3
  xorl %r10d, %r10d #67.3
  movl $1, %r14d #67.3
  xorl %r11d, %r11d #67.3
  shrl $1, %edx #67.3
  je ..B1.51 # Prob 2% #67.3
..B1.48: # Preds ..B1.47
  movslq %r9d, %r9 #68.12
  lea 25(%rsp,%r9), %rax #68.12
..B1.49: # Preds ..B1.49 ..B1.48
  movb (%r11,%rax), %r14b #68.12
  movb %r14b, sqlite_varint_bytes(%rbx,%r10,2) #68.5
  movb -1(%r11,%rax), %r14b #68.12
  addq $-2, %r11 #67.3
  movb %r14b, 1+sqlite_varint_bytes(%rbx,%r10,2) #68.5
  incq %r10 #67.3
  cmpq %rdx, %r10 #67.3
  jb ..B1.49 # Prob 79% #67.3
..B1.50: # Preds ..B1.49
  lea 1(%r10,%r10), %r14d #68.5
..B1.51: # Preds ..B1.50 ..B1.47
  lea -1(%r14), %eax #67.3
  cmpl %r15d, %eax #67.3
  jae ..B1.56 # Prob 2% #67.3
..B1.52: # Preds ..B1.51
  movslq %r9d, %r9 #68.12
  movslq %r14d, %r14 #68.12
  lea 25(%rsp,%r9), %rax #68.12
  subq %r14, %rax #68.12
  movb 1(%rax), %dl #68.12
  movb %dl, -1+sqlite_varint_bytes(%r14,%rbx) #68.5
  jmp ..B1.56 # Prob 100% #68.5
..B1.54: # Preds ..B1.42
  movq %rax, %rdx #47.24
  andq $127, %rax #48.21
  shrq $7, %rdx #47.24
  movl $2, %r15d #219.27
  orq $-128, %rdx #47.38
  movb %dl, sqlite_varint_bytes(%rbx) #47.5
  movb %al, 1+sqlite_varint_bytes(%rbx) #48.5
  jmp ..B1.56 # Prob 100% #48.5
..B1.55: # Preds ..B1.41
  andq $127, %rax #43.21
  movl $1, %r15d #219.27
  movb %al, sqlite_varint_bytes(%rbx) #43.5
..B1.56: # Preds ..B1.52 ..B1.46 ..B1.51 ..B1.55 ..B1.54
  incl %r8d #214.32
  addl %r15d, %ecx #219.5
  cmpl $262144, %r8d #214.19
  jb ..B1.22 # Prob 99% #214.19
..B1.57: # Preds ..B1.56
  xorb %dl, %dl #255.8
  xorl %eax, %eax #255.15
  movl %ecx, sqlite_varint_used(%rip) #221.3
  movl %eax, %r13d #255.15
  movq %r12, (%rsp) #255.15[spill]
  movb %dl, %r15b #255.15
  movl %ecx, %ebx #255.15
  movq %rsi, %r14 #255.15
..B1.58: # Preds ..B1.61 ..B1.57
  xorl %r12d, %r12d #257.10
..B1.59: # Preds ..B1.60 ..B1.58
  lea 32(%rsp), %rsi #259.14
  movq $0, (%rsi) #258.17
  movl sqlite_varint_offsets(,%r12,4), %r8d #259.14
  lea sqlite_varint_bytes(%r8), %rdi #259.14
  call sqlite_get_varint #259.14
..B1.60: # Preds ..B1.59
  movl %r12d, %ecx #261.43
  incl %r12d #257.34
  andl $31, %ecx #261.43
  movzbl %al, %edi #261.33
  shlq %cl, %rdi #261.43
  xorq 32(%rsp), %rdi #261.43
  addq %rdi, %r14 #261.7
  cmpl $262144, %r12d #257.21
  jb ..B1.59 # Prob 99% #257.21
..B1.61: # Preds ..B1.60
  movl %r13d, %edi #264.25
  incb %r15b #255.33
  andq $262143, %rdi #265.50
  addl $104729, %r13d #255.33
  movl sqlite_varint_offsets(,%rdi,4), %r8d #264.25
  xorb $1, sqlite_varint_bytes(%r8) #264.5
  cmpb $24, %r15b #255.25
  jb ..B1.58 # Prob 95% #255.25
..B1.62: # Preds ..B1.61
  movl %ebx, %ecx #
  movq %r14, %rsi #
  movq (%rsp), %r12 #[spill]
  testl %ecx, %ecx #268.29
  jne ..B1.64 # Prob 22% #268.29
..B1.63: # Preds ..B1.62
  movl $3, %eax #269.12
  addq $96, %rsp #269.12
  popq %rbx #269.12
  popq %r15 #269.12
  popq %r14 #269.12
  popq %r13 #269.12
  movq %rbp, %rsp #269.12
  popq %rbp #269.12
  ret #269.12
..B1.64: # Preds ..B1.62
  movl $.L_2__STRING.0, %edi #270.3
  xorl %eax, %eax #270.3
  call printf #270.3
..B1.65: # Preds ..B1.64
  xorl %eax, %eax #271.10
  addq $96, %rsp #271.10
  popq %rbx #271.10
  popq %r15 #271.10
  popq %r14 #271.10
  popq %r13 #271.10
  movq %rbp, %rsp #271.10
  popq %rbp #271.10
  ret #271.10
..B1.66: # Preds ..B1.4
  movq %r15, %rdx #53.5
  movq %r15, %rcx #56.7
  movq %r15, %rdi #56.7
  movq %r15, %r8 #56.7
  movq %r15, %r9 #56.7
  movq %r15, %r10 #56.7
  movq %r15, %r11 #56.7
  movq %r15, %r13 #56.7
  shrq $8, %rdx #53.5
  shrq $15, %rcx #56.7
  orq $-128, %rdx #55.33
  shrq $22, %rdi #56.7
  orq $-128, %rcx #55.33
  shrq $29, %r8 #56.7
  orq $-128, %rdi #55.33
  shrq $36, %r9 #56.7
  orq $-128, %r8 #55.33
  shrq $43, %r10 #56.7
  orq $-128, %r9 #55.33
  shrq $50, %r11 #56.7
  orq $-128, %r10 #55.33
  shrq $57, %r13 #56.7
  orq $-128, %r11 #55.33
  addq $-128, %r13 #55.33
  movb %r15b, 24(%rsp) #237.37
  movb %dl, 23(%rsp) #237.37
  movb %cl, 22(%rsp) #237.37
  movb %dil, 21(%rsp) #237.37
  movb %r8b, 20(%rsp) #237.37
  movb %r9b, 19(%rsp) #237.37
  movb %r10b, 18(%rsp) #237.37
  movb %r11b, 17(%rsp) #237.37
  movb %r13b, 16(%rsp) #237.37
  movl $9, %r13d #237.19
  jmp ..B1.17 # Prob 100% #237.19
..B1.67: # Preds ..B1.43
  movq %rax, %rdx #53.5
  movq %rax, %r9 #56.7
  shrq $8, %rdx #53.5
  movq %rax, %r10 #56.7
  orq $-128, %rdx #55.33
  movq %rax, %r11 #56.7
  movb %dl, 7+sqlite_varint_bytes(%rbx) #55.7
  movq %rax, %rdx #56.7
  shrq $43, %rdx #56.7
  movq %rax, %r15 #56.7
  orq $-128, %rdx #55.33
  movb %dl, 2+sqlite_varint_bytes(%rbx) #55.7
  movq %rax, %rdx #56.7
  movb %al, 8+sqlite_varint_bytes(%rbx) #52.5
  shrq $15, %r9 #56.7
  shrq $22, %r10 #56.7
  orq $-128, %r9 #55.33
  shrq $29, %r11 #56.7
  orq $-128, %r10 #55.33
  shrq $36, %r15 #56.7
  orq $-128, %r11 #55.33
  shrq $50, %rdx #56.7
  orq $-128, %r15 #55.33
  shrq $57, %rax #56.7
  addq $-128, %rdx #55.33
  addq $-128, %rax #55.33
  movb %r9b, 6+sqlite_varint_bytes(%rbx) #55.7
  movb %r10b, 5+sqlite_varint_bytes(%rbx) #55.7
  movb %r11b, 4+sqlite_varint_bytes(%rbx) #55.7
  movb %r15b, 3+sqlite_varint_bytes(%rbx) #55.7
  movl $9, %r15d #219.27
  movb %dl, 1+sqlite_varint_bytes(%rbx) #55.7
  movb %al, sqlite_varint_bytes(%rbx) #55.7
  jmp ..B1.56 # Prob 100% #55.7
..B1.69: # Preds ..B1.18 ..B1.19
  movl $2, %eax #252.12
  addq $96, %rsp #252.12
  popq %rbx #252.12
  popq %r15 #252.12
  popq %r14 #252.12
  popq %r13 #252.12
  movq %rbp, %rsp #252.12
  popq %rbp #252.12
  ret #252.12
values.159.0.5:
.2.13_2.switchtab.12:
sqlite_get_varint:
..B2.1: # Preds ..B2.0
  cmpb $0, (%rdi) #80.38
  jl ..B2.3 # Prob 32% #80.38
..B2.2: # Preds ..B2.1
  movzbl (%rdi), %eax #81.11
  movq %rax, (%rsi) #81.6
  movl $1, %eax #82.12
  ret #82.12
..B2.3: # Preds ..B2.1
  cmpb $0, 1(%rdi) #84.38
  jl ..B2.5 # Prob 32% #84.38
..B2.4: # Preds ..B2.3
  movzbl (%rdi), %eax #85.17
  andl $127, %eax #85.24
  shll $7, %eax #85.34
  movzbl 1(%rdi), %edi #85.39
  orl %edi, %eax #85.39
  movq %rax, (%rsi) #85.6
  movl $2, %eax #86.12
  ret #86.12
..B2.5: # Preds ..B2.3
  movzbl (%rdi), %ecx #89.13
  shll $14, %ecx #89.22
  movzbl 2(%rdi), %eax #92.9
  orl %eax, %ecx #92.3
  movzbl 1(%rdi), %edx #90.7
  testl $128, %ecx #93.13
  je ..B2.18 # Prob 28% #93.13
..B2.6: # Preds ..B2.5
  shll $14, %edx #104.3
  andl $2080895, %ecx #102.3
  movzbl 3(%rdi), %eax #105.9
  orl %eax, %edx #105.3
  testl $128, %edx #106.13
  je ..B2.17 # Prob 28% #106.13
..B2.7: # Preds ..B2.6
  movl %ecx, %eax #117.3
  andl $2080895, %edx #114.3
  shll $14, %eax #117.3
  movzbl 4(%rdi), %r9d #118.9
  orl %r9d, %eax #118.3
  testl $128, %eax #119.13
  je ..B2.16 # Prob 28% #119.13
..B2.8: # Preds ..B2.7
  shll $7, %ecx #127.3
  orl %edx, %ecx #128.3
  shll $14, %edx #130.3
  movzbl 5(%rdi), %r8d #131.9
  orl %r8d, %edx #131.3
  testl $128, %edx #132.13
  je ..B2.15 # Prob 28% #132.13
..B2.9: # Preds ..B2.8
  shll $14, %eax #142.3
  movzbl 6(%rdi), %r8d #143.9
  orl %r8d, %eax #143.3
  testl $128, %eax #144.13
  je ..B2.14 # Prob 28% #144.13
..B2.10: # Preds ..B2.9
  shll $14, %edx #156.3
  andl $2080895, %eax #154.3
  movzbl 7(%rdi), %r8d #157.9
  orl %r8d, %edx #157.3
  testl $128, %edx #158.13
  jne ..B2.12 # Prob 50% #158.13
..B2.11: # Preds ..B2.10
  shrl $4, %ecx #162.5
  andl $-266354561, %edx #159.5
  shll $7, %eax #160.5
  orl %edx, %eax #161.5
  shlq $32, %rcx #163.21
  orq %rax, %rcx #163.27
  movl $8, %eax #164.12
  movq %rcx, (%rsi) #163.6
  ret #163.6
..B2.12: # Preds ..B2.10
  andl $127, %r9d #175.3
  andl $2080895, %edx #170.3
  shll $4, %ecx #173.3
  shrl $3, %r9d #176.3
  shll $15, %eax #168.3
  orl %r9d, %ecx #177.3
  movzbl 8(%rdi), %edi #169.9
  orl %edi, %eax #169.3
  shll $8, %edx #171.3
  orl %edx, %eax #172.3
  shlq $32, %rcx #178.19
  orq %rax, %rcx #178.25
  movl $9, %eax #179.10
  movq %rcx, (%rsi) #178.4
..B2.13: # Preds ..B2.12
  ret #164.12
..B2.14: # Preds ..B2.9
  andl $2080895, %edx #146.5
  andl $-266354561, %eax #145.5
  shrl $11, %ecx #149.5
  shll $7, %edx #147.5
  orl %edx, %eax #148.5
  shlq $32, %rcx #150.21
  orq %rax, %rcx #150.27
  movl $7, %eax #151.12
  movq %rcx, (%rsi) #150.6
  ret #151.12
..B2.15: # Preds ..B2.8
  andl $2080895, %eax #133.5
  shrl $18, %ecx #136.5
  shll $7, %eax #134.5
  orl %edx, %eax #135.5
  shlq $32, %rcx #137.21
  orq %rax, %rcx #137.27
  movl $6, %eax #138.12
  movq %rcx, (%rsi) #137.6
  ret #138.12
..B2.16: # Preds ..B2.7
  shrl $18, %ecx #122.5
  shll $7, %edx #120.5
  orl %edx, %eax #121.5
  shlq $32, %rcx #123.21
  orq %rax, %rcx #123.27
  movl $5, %eax #124.12
  movq %rcx, (%rsi) #123.6
  ret #124.12
..B2.17: # Preds ..B2.6
  shll $7, %ecx #108.5
  andl $2080895, %edx #107.5
  orl %edx, %ecx #109.5
  movl $4, %eax #111.12
  movq %rcx, (%rsi) #110.6
  ret #111.12
..B2.18: # Preds ..B2.5
  andl $127, %edx #95.5
  andl $2080895, %ecx #94.5
  shll $7, %edx #96.5
  movl $3, %eax #99.12
  orl %edx, %ecx #97.5
  movq %rcx, (%rsi) #98.6
  ret #99.12
sqlite_varint_offsets:
sqlite_varint_bytes:
sqlite_varint_used:
.L_2__STRING.0:
