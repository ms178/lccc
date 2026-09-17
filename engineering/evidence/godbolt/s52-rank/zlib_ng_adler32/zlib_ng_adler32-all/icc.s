main:
..B1.1: # Preds ..B1.0
  pushq %rbp #110.1
  movq %rsp, %rbp #110.1
  andq $-128, %rsp #110.1
  pushq %r12 #110.1
  subq $120, %rsp #110.1
  movl $3, %edi #110.1
  xorl %esi, %esi #110.1
  call __intel_new_feature_proc_init #110.1
..B1.12: # Preds ..B1.1
  stmxcsr (%rsp) #110.1
  movl $1, %edi #116.7
  movl $check.173.0.5, %esi #116.7
  orl $32832, (%rsp) #110.1
  movl $9, %edx #116.7
  ldmxcsr (%rsp) #110.1
  xorl %r12d, %r12d #113.25
  call zlib_ng_adler32_c..0 #116.7
..B1.11: # Preds ..B1.12
  cmpl $152961502, %eax #116.43
  je ..B1.3 # Prob 42% #116.43
..B1.2: # Preds ..B1.11
  movl $2, %eax #117.12
  addq $120, %rsp #117.12
  popq %r12 #117.12
  movq %rbp, %rsp #117.12
  popq %rbp #117.12
  ret #117.12
..B1.3: # Preds ..B1.11
  movl $-1640531527, %edx #100.22
  xorl %eax, %eax #102.3
..B1.4: # Preds ..B1.4 ..B1.3
  imull $1103515245, %edx, %ecx #103.21
  addl $12345, %ecx #103.35
  movl %ecx, %edx #104.54
  shrl $16, %edx #104.54
  movb %dl, zlib_ng_adler_data(,%rax,2) #104.5
  imull $1103515245, %ecx, %edx #103.21
  addl $12345, %edx #103.35
  movl %edx, %edi #104.54
  shrl $16, %edi #104.54
  movb %dil, 1+zlib_ng_adler_data(,%rax,2) #104.5
  incq %rax #102.3
  cmpq $1048576, %rax #102.3
  jb ..B1.4 # Prob 99% #102.3
..B1.5: # Preds ..B1.4
  xorl %edx, %edx #120.3
  movq %r13, 16(%rsp) #121.26[spill]
  movl %edx, %r13d #121.26
  movq %r14, 8(%rsp) #121.26[spill]
  movl %edx, %r14d #121.26
  movq %r15, (%rsp) #121.26[spill]
..B1.6: # Preds ..B1.13 ..B1.5
  movl $zlib_ng_adler_data, %esi #121.26
  lea 1(%r13), %r15d #121.49
  movl %r15d, %edi #121.26
  movl $2097152, %edx #121.26
  call zlib_ng_adler32_c..1 #121.26
..B1.13: # Preds ..B1.6
  movl %r14d, %edx #125.32
  addl %eax, %r13d #123.25
  andq $2097151, %rdx #125.42
  xorl %r13d, %r12d #123.5
  shrl $9, %eax #126.34
  addl $12289, %r14d #121.49
  movl %r15d, %r13d #120.3
  xorb %al, zlib_ng_adler_data(%rdx) #125.5
  cmpl $48, %r15d #120.3
  jb ..B1.6 # Prob 97% #120.3
..B1.7: # Preds ..B1.13
  movl $.L_2__STRING.0, %edi #129.3
  movl %r12d, %esi #129.3
  xorl %eax, %eax #129.3
  movq 16(%rsp), %r13 #[spill]
  movq 8(%rsp), %r14 #[spill]
  movq (%rsp), %r15 #[spill]
  call printf #129.3
..B1.8: # Preds ..B1.7
  xorl %eax, %eax #130.10
  addq $120, %rsp #130.10
  popq %r12 #130.10
  movq %rbp, %rsp #130.10
  popq %rbp #130.10
  ret #130.10
check.173.0.5:
zlib_ng_adler32_c..1:
..B2.1: # Preds ..B2.0
  movl %edi, %eax #68.1
  movl %eax, %edi #72.20
  movq %rsi, %r8 #68.1
  shrl $16, %edi #72.20
  movl $2097152, %esi #68.1
  movzwl %ax, %ecx #73.3
  testq %r8, %r8 #77.14
  je ..B2.18 # Prob 5% #77.14
..B2.3: # Preds ..B2.1 ..B2.5
  addq $-5552, %rsi #83.5
  movl $694, %r10d #84.5
..B2.4: # Preds ..B2.4 ..B2.3
  movzbl (%r8), %r11d #86.7
  addl %r11d, %ecx #86.7
  movzbl 1(%r8), %edx #86.7
  addl %ecx, %edx #86.7
  movzbl 2(%r8), %r9d #86.7
  addl %edx, %ecx #86.7
  addl %edx, %r9d #86.7
  movzbl 3(%r8), %r11d #86.7
  addl %r9d, %r11d #86.7
  movzbl 4(%r8), %edx #86.7
  addl %r11d, %r9d #86.7
  addl %r11d, %edx #86.7
  addl %ecx, %r9d #86.7
  movzbl 5(%r8), %r11d #86.7
  addl %edx, %r11d #86.7
  movzbl 6(%r8), %eax #86.7
  addl %r11d, %edx #86.7
  addl %r11d, %eax #86.7
  movzbl 7(%r8), %ecx #86.7
  addq $8, %r8 #87.7
  addl %eax, %ecx #86.7
  addl %ecx, %eax #86.7
  addl %eax, %edx #86.7
  addl %edx, %r9d #86.7
  addl %r9d, %edi #86.7
  decl %r10d #88.16
  jne ..B2.4 # Prob 82% #88.16
..B2.5: # Preds ..B2.4
  movl $-2146992015, %eax #89.5
  mull %ecx #89.5
  movl $-2146992015, %eax #90.5
  shrl $15, %edx #89.5
  imull $-65521, %edx, %r9d #89.5
  mull %edi #90.5
  addl %r9d, %ecx #89.5
  shrl $15, %edx #90.5
  imull $-65521, %edx, %r10d #90.5
  addl %r10d, %edi #90.5
  cmpq $5552, %rsi #82.17
  jae ..B2.3 # Prob 82% #82.17
..B2.6: # Preds ..B2.5
  cmpq $8, %rsi #56.17
  jb ..B2.10 # Prob 10% #56.17
..B2.8: # Preds ..B2.6 ..B2.8
  movzbl (%r8), %r9d #58.5
  addl %r9d, %ecx #58.5
  movzbl 1(%r8), %edx #58.5
  addq $-8, %rsi #57.5
  addl %ecx, %edx #58.5
  movzbl 2(%r8), %r9d #58.5
  addl %edx, %ecx #58.5
  addl %edx, %r9d #58.5
  movzbl 3(%r8), %r10d #58.5
  addl %r9d, %r10d #58.5
  movzbl 4(%r8), %r11d #58.5
  addl %r10d, %r9d #58.5
  addl %r10d, %r11d #58.5
  addl %ecx, %r9d #58.5
  movzbl 5(%r8), %eax #58.5
  addl %r11d, %eax #58.5
  movzbl 6(%r8), %edx #58.5
  addl %eax, %r11d #58.5
  addl %eax, %edx #58.5
  movzbl 7(%r8), %r10d #58.5
  addq $8, %r8 #59.5
  lea (%rdx,%r10), %ecx #58.5
  addl %ecx, %edx #58.5
  addl %edx, %r11d #58.5
  addl %r11d, %r9d #58.5
  addl %r9d, %edi #58.5
  cmpq $8, %rsi #56.17
  jae ..B2.8 # Prob 82% #56.17
..B2.10: # Preds ..B2.8 ..B2.6
  testq %rsi, %rsi #42.10
  je ..B2.17 # Prob 50% #42.10
..B2.11: # Preds ..B2.10
  movq %rsi, %r9 #61.10
  movl $1, %r10d #42.3
  xorl %edx, %edx #42.3
  shrq $1, %r9 #61.10
  je ..B2.15 # Prob 9% #42.3
..B2.12: # Preds ..B2.11
  xorl %r10d, %r10d #47.3
..B2.13: # Preds ..B2.13 ..B2.12
  movzbl (%r8,%rdx,2), %r11d #44.14
  addl %r11d, %ecx #44.5
  movzbl 1(%r8,%rdx,2), %r11d #44.14
  addl %ecx, %edi #45.5
  addl %r11d, %ecx #44.5
  incq %rdx #42.3
  addl %ecx, %r10d #45.5
  cmpq %r9, %rdx #42.3
  jb ..B2.13 # Prob 63% #42.3
..B2.14: # Preds ..B2.13
  addl %r10d, %edi #47.3
  lea 1(%rdx,%rdx), %r10 #43.7
..B2.15: # Preds ..B2.14 ..B2.11
  lea -1(%r10), %rdx #42.3
  cmpq %rsi, %rdx #42.3
  jae ..B2.17 # Prob 9% #42.3
..B2.16: # Preds ..B2.15
  movzbl -1(%r8,%r10), %edx #44.14
  addl %edx, %ecx #44.5
  addl %ecx, %edi #45.5
..B2.17: # Preds ..B2.16 ..B2.15 ..B2.10
  movl $-2146992015, %eax #47.3
  mull %ecx #47.3
  movl $-2146992015, %eax #48.3
  shrl $15, %edx #47.3
  imull $-65521, %edx, %esi #47.3
  mull %edi #48.3
  addl %esi, %ecx #47.3
  shrl $15, %edx #48.3
  imull $-65521, %edx, %r8d #48.3
  addl %r8d, %edi #48.3
  shll $16, %edi #49.26
  orl %edi, %ecx #49.26
  movl %ecx, %eax #93.10
  ret #93.10
..B2.18: # Preds ..B2.1
  movl $1, %eax #78.12
  ret #78.12
zlib_ng_adler32_c..0:
..B3.1: # Preds ..B3.0
  testq %rsi, %rsi #77.14
  je ..B3.4 # Prob 5% #77.14
..B3.2: # Preds ..B3.1
  movzwl %di, %ecx #73.3
  movzbl (%rsi), %r8d #44.14
  addl %r8d, %ecx #44.5
  shrl $16, %edi #72.20
  movzbl 1(%rsi), %r9d #44.14
  addl %ecx, %edi #45.5
  addl %r9d, %ecx #44.5
  movzbl 2(%rsi), %r10d #44.14
  addl %ecx, %edi #45.5
  addl %r10d, %ecx #44.5
  movzbl 3(%rsi), %r11d #44.14
  addl %ecx, %edi #45.5
  addl %r11d, %ecx #44.5
  movzbl 4(%rsi), %eax #44.14
  addl %ecx, %edi #45.5
  addl %eax, %ecx #44.5
  movl $-2146992015, %eax #47.3
  movzbl 5(%rsi), %edx #44.14
  addl %ecx, %edi #45.5
  addl %edx, %ecx #44.5
  movzbl 6(%rsi), %r8d #44.14
  addl %ecx, %edi #45.5
  addl %r8d, %ecx #44.5
  movzbl 7(%rsi), %r9d #44.14
  addl %ecx, %edi #45.5
  addl %r9d, %ecx #44.5
  movzbl 8(%rsi), %esi #44.14
  addl %ecx, %edi #45.5
  addl %esi, %ecx #44.5
  mull %ecx #47.3
  addl %ecx, %edi #45.5
  movl $-2146992015, %eax #48.3
  shrl $15, %edx #47.3
  imull $-65521, %edx, %esi #47.3
  mull %edi #48.3
  addl %ecx, %esi #47.3
  shrl $15, %edx #48.3
  imull $-65521, %edx, %ecx #48.3
  addl %ecx, %edi #48.3
  shll $16, %edi #49.26
  orl %edi, %esi #49.26
..B3.3: # Preds ..B3.4 ..B3.2
  movl %esi, %eax #78.12
  ret #78.12
..B3.4: # Preds ..B3.1
  movl $1, %esi #78.12
  jmp ..B3.3 # Prob 100% #78.12
zlib_ng_adler_data:
.L_2__STRING.0:
