main:
..B1.1: # Preds ..B1.0
  pushq %rbp #105.1
  movq %rsp, %rbp #105.1
  andq $-128, %rsp #105.1
  subq $128, %rsp #105.1
  movl $3, %edi #105.1
  xorl %esi, %esi #105.1
  call __intel_new_feature_proc_init #105.1
..B1.11: # Preds ..B1.1
  stmxcsr (%rsp) #105.1
  xorl %esi, %esi #107.20
  movl $305419896, %edx #95.22
  orl $32832, (%rsp) #105.1
  xorl %eax, %eax #97.3
  ldmxcsr (%rsp) #105.1
..B1.2: # Preds ..B1.2 ..B1.11
  imull $1664525, %edx, %ecx #98.21
  addl $1013904223, %ecx #98.32
  movl %ecx, %edx #99.49
  shrl $24, %edx #99.49
  movb %dl, gzip_crc_data(,%rax,2) #99.5
  imull $1664525, %ecx, %edx #98.21
  addl $1013904223, %edx #98.32
  movl %edx, %edi #99.49
  shrl $24, %edi #99.49
  movb %dil, 1+gzip_crc_data(,%rax,2) #99.5
  incq %rax #97.3
  cmpq $524288, %rax #97.3
  jb ..B1.2 # Prob 99% #97.3
..B1.3: # Preds ..B1.2
  xorb %cl, %cl #115.8
  xorl %edx, %edx #87.10
..B1.4: # Preds ..B1.6 ..B1.3
  notl %esi #87.41
  xorl %eax, %eax #77.3
..B1.5: # Preds ..B1.5 ..B1.4
  movl %esi, %r8d #78.11
  movzbl gzip_crc_data(,%rax,2), %edi #116.34
  xorq %rdi, %r8 #78.35
  movzbl %r8b, %r9d #78.45
  shrl $8, %esi #78.62
  movzbl 1+gzip_crc_data(,%rax,2), %r10d #116.34
  incq %rax #77.3
  xorl gzip_crc32_table(,%r9,4), %esi #78.62
  movl %esi, %r11d #78.11
  shrl $8, %esi #78.62
  xorq %r10, %r11 #78.35
  movzbl %r11b, %edi #78.45
  xorl gzip_crc32_table(,%rdi,4), %esi #78.62
  cmpq $524288, %rax #77.3
  jb ..B1.5 # Prob 64% #77.3
..B1.6: # Preds ..B1.5
  movl %edx, %eax #118.27
  notl %esi #88.12
  andq $1048575, %rax #118.36
  incb %cl #115.8
  addl $8191, %edx #115.8
  xorb %sil, gzip_crc_data(%rax) #118.5
  cmpb $64, %cl #115.8
  jb ..B1.4 # Prob 99% #115.8
..B1.7: # Preds ..B1.6
  movl $.L_2__STRING.0, %edi #121.3
  xorl %eax, %eax #121.3
  call printf #121.3
..B1.8: # Preds ..B1.7
  xorl %eax, %eax #122.10
  movq %rbp, %rsp #122.10
  popq %rbp #122.10
  ret #122.10
gzip_crc_data:
gzip_crc32_table:
.L_2__STRING.0:
