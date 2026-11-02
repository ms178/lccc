main:
..B1.1: # Preds ..B1.0
  pushq %rbp #141.1
  movq %rsp, %rbp #141.1
  andq $-128, %rsp #141.1
  pushq %r14 #141.1
  subq $120, %rsp #141.1
  movl $3, %edi #141.1
  xorl %esi, %esi #141.1
  call __intel_new_feature_proc_init #141.1
..B1.143: # Preds ..B1.1
  stmxcsr (%rsp) #141.1
  movl $ascii_name.168.0.6, %eax #132.7
  movl $ascii_name.168.0.6+7, %edx #132.42
  orl $32832, (%rsp) #141.1
  xorl %r14d, %r14d #143.26
  ldmxcsr (%rsp) #141.1
  movq %rax, %rcx #132.7
  cmpq %rdx, %rax #52.16
  jae ..B1.27 # Prob 10% #52.16
..B1.3: # Preds ..B1.143 ..B1.24
  movzbl (%rax), %r8d #53.24
  cmpl $128, %r8d #54.13
  jb ..B1.17 # Prob 22% #54.13
..B1.4: # Preds ..B1.3
  lea -194(%r8), %edi #60.16
  cmpl $29, %edi #60.16
  ja ..B1.6 # Prob 50% #60.16
..B1.5: # Preds ..B1.4
  movl $2, %r8d #61.9
  jmp ..B1.10 # Prob 100% #61.9
..B1.6: # Preds ..B1.4
  lea -224(%r8), %edi #62.21
  cmpl $15, %edi #62.21
  ja ..B1.8 # Prob 50% #62.21
..B1.7: # Preds ..B1.6
  movl $3, %r8d #63.9
  jmp ..B1.10 # Prob 100% #63.9
..B1.8: # Preds ..B1.6
  addl $-240, %r8d #64.21
  cmpl $4, %r8d #64.21
  ja ..B1.27 # Prob 20% #64.21
..B1.9: # Preds ..B1.8
  movl $4, %r8d #65.9
..B1.10: # Preds ..B1.5 ..B1.7 ..B1.9
  movq %rdx, %rdi #68.27
  subq %rax, %rdi #68.27
  cmpq %r8, %rdi #68.40
  jb ..B1.27 # Prob 20% #68.40
..B1.11: # Preds ..B1.10
  movb 1(%rax), %dil #70.12
  andb $-64, %dil #70.21
  cmpb $-128, %dil #70.31
  jne ..B1.27 # Prob 20% #70.31
..B1.12: # Preds ..B1.11
  cmpq $2, %r8 #71.23
  jbe ..B1.14 # Prob 50% #71.23
..B1.13: # Preds ..B1.12
  movb 2(%rax), %dil #71.31
  andb $-64, %dil #71.40
  cmpb $-128, %dil #71.50
  jne ..B1.27 # Prob 20% #71.50
..B1.14: # Preds ..B1.12 ..B1.13
  cmpq $3, %r8 #72.23
  jbe ..B1.16 # Prob 50% #72.23
..B1.15: # Preds ..B1.14
  movb 3(%rax), %dil #72.31
  andb $-64, %dil #72.40
  cmpb $-128, %dil #72.50
  jne ..B1.27 # Prob 20% #72.50
..B1.16: # Preds ..B1.14 ..B1.15
  addq %r8, %rax #74.7
  jmp ..B1.24 # Prob 100% #74.7
..B1.17: # Preds ..B1.3
  lea -97(%r8), %edi #32.16
  cmpl $25, %edi #32.16
  jbe ..B1.23 # Prob 50% #32.16
..B1.18: # Preds ..B1.17
  lea -65(%r8), %edi #33.16
  cmpl $25, %edi #33.16
  jbe ..B1.23 # Prob 50% #33.16
..B1.19: # Preds ..B1.18
  cmpl $58, %r8d #34.10
  je ..B1.23 # Prob 33% #34.10
..B1.20: # Preds ..B1.19
  cmpl $95, %r8d #34.10
  je ..B1.23 # Prob 50% #34.10
..B1.21: # Preds ..B1.20
  lea -48(%r8), %edi #42.16
  cmpl $9, %edi #42.16
  jbe ..B1.23 # Prob 50% #42.16
..B1.22: # Preds ..B1.21
  addl $-45, %r8d #43.10
  cmpl $1, %r8d #43.10
  ja ..B1.27 # Prob 50% #43.10
..B1.23: # Preds ..B1.21 ..B1.17 ..B1.18 ..B1.19 ..B1.20
  incq %rax #57.7
..B1.24: # Preds ..B1.23 ..B1.16
  cmpq %rdx, %rax #52.16
  jb ..B1.3 # Prob 82% #52.16
..B1.27: # Preds ..B1.15 ..B1.10 ..B1.8 ..B1.13 ..B1.11
  subq %rcx, %rax #77.26
  cmpq $7, %rax #132.61
  jne ..B1.55 # Prob 57% #132.61
..B1.28: # Preds ..B1.27
  movl $utf8_name.168.0.6, %edx #134.7
  movl $utf8_name.168.0.6+8, %eax #134.41
  movq %rdx, %rcx #134.7
  cmpq %rax, %rdx #52.16
  jae ..B1.54 # Prob 10% #52.16
..B1.30: # Preds ..B1.28 ..B1.51
  movzbl (%rdx), %r8d #53.24
  cmpl $128, %r8d #54.13
  jb ..B1.44 # Prob 22% #54.13
..B1.31: # Preds ..B1.30
  lea -194(%r8), %edi #60.16
  cmpl $29, %edi #60.16
  ja ..B1.33 # Prob 50% #60.16
..B1.32: # Preds ..B1.31
  movl $2, %r8d #61.9
  jmp ..B1.37 # Prob 100% #61.9
..B1.33: # Preds ..B1.31
  lea -224(%r8), %edi #62.21
  cmpl $15, %edi #62.21
  ja ..B1.35 # Prob 50% #62.21
..B1.34: # Preds ..B1.33
  movl $3, %r8d #63.9
  jmp ..B1.37 # Prob 100% #63.9
..B1.35: # Preds ..B1.33
  addl $-240, %r8d #64.21
  cmpl $4, %r8d #64.21
  ja ..B1.54 # Prob 20% #64.21
..B1.36: # Preds ..B1.35
  movl $4, %r8d #65.9
..B1.37: # Preds ..B1.32 ..B1.34 ..B1.36
  movq %rax, %rdi #68.27
  subq %rdx, %rdi #68.27
  cmpq %r8, %rdi #68.40
  jb ..B1.54 # Prob 20% #68.40
..B1.38: # Preds ..B1.37
  movb 1(%rdx), %dil #70.12
  andb $-64, %dil #70.21
  cmpb $-128, %dil #70.31
  jne ..B1.54 # Prob 20% #70.31
..B1.39: # Preds ..B1.38
  cmpq $2, %r8 #71.23
  jbe ..B1.41 # Prob 50% #71.23
..B1.40: # Preds ..B1.39
  movb 2(%rdx), %dil #71.31
  andb $-64, %dil #71.40
  cmpb $-128, %dil #71.50
  jne ..B1.54 # Prob 20% #71.50
..B1.41: # Preds ..B1.39 ..B1.40
  cmpq $3, %r8 #72.23
  jbe ..B1.43 # Prob 50% #72.23
..B1.42: # Preds ..B1.41
  movb 3(%rdx), %dil #72.31
  andb $-64, %dil #72.40
  cmpb $-128, %dil #72.50
  jne ..B1.54 # Prob 20% #72.50
..B1.43: # Preds ..B1.42 ..B1.41
  addq %r8, %rdx #74.7
  jmp ..B1.51 # Prob 100% #74.7
..B1.44: # Preds ..B1.30
  lea -97(%r8), %edi #32.16
  cmpl $25, %edi #32.16
  jbe ..B1.50 # Prob 50% #32.16
..B1.45: # Preds ..B1.44
  lea -65(%r8), %edi #33.16
  cmpl $25, %edi #33.16
  jbe ..B1.50 # Prob 50% #33.16
..B1.46: # Preds ..B1.45
  cmpl $58, %r8d #34.10
  je ..B1.50 # Prob 33% #34.10
..B1.47: # Preds ..B1.46
  cmpl $95, %r8d #34.10
  je ..B1.50 # Prob 50% #34.10
..B1.48: # Preds ..B1.47
  lea -48(%r8), %edi #42.16
  cmpl $9, %edi #42.16
  jbe ..B1.50 # Prob 50% #42.16
..B1.49: # Preds ..B1.48
  addl $-45, %r8d #43.10
  cmpl $1, %r8d #43.10
  ja ..B1.54 # Prob 50% #43.10
..B1.50: # Preds ..B1.44 ..B1.45 ..B1.46 ..B1.47 ..B1.48
  incq %rdx #57.7
..B1.51: # Preds ..B1.50 ..B1.43
  cmpq %rax, %rdx #52.16
  jb ..B1.30 # Prob 82% #52.16
..B1.54: # Preds ..B1.42 ..B1.37 ..B1.35 ..B1.40 ..B1.38
  subq %rcx, %rdx #77.26
  cmpq $8, %rdx #134.59
  je ..B1.56 # Prob 50% #134.59
..B1.55: # Preds ..B1.27 ..B1.54
  movl $2, %eax #146.12
  addq $120, %rsp #146.12
  popq %r14 #146.12
  movq %rbp, %rsp #146.12
  popq %rbp #146.12
  ret #146.12
..B1.56: # Preds ..B1.54
  xorl %eax, %eax #115.21
..B1.57: # Preds ..B1.59 ..B1.56
  xorl %ecx, %ecx #119.10
  movl $1, %edx #119.47
..B1.58: # Preds ..B1.58 ..B1.57
  movb fragment.161.0.5(%rcx), %cl #120.31
  movb %cl, expat_xml_data(%rax) #120.7
  movq %rdx, %rcx #119.47
  incq %rdx #119.23
  incq %rax #120.22
  cmpq $61, %rdx #119.29
  jb ..B1.58 # Prob 82% #119.29
..B1.59: # Preds ..B1.58
  lea 61(%rax), %rdx #118.16
  cmpq $1048576, %rdx #118.36
  jbe ..B1.57 # Prob 82% #118.36
..B1.60: # Preds ..B1.59
  cmpq $1048576, %rax #122.16
  jae ..B1.63 # Prob 50% #122.16
..B1.61: # Preds ..B1.60
  movq %rax, %rcx #120.22
  negq %rcx #120.22
  lea 1048576(%rcx), %rdx #120.22
  addq $1048575, %rcx #122.3
  cmpq $96, %rcx #122.3
  jbe ..B1.122 # Prob 0% #122.3
..B1.62: # Preds ..B1.61
  movl $32, %esi #122.3
  lea expat_xml_data(%rax), %rdi #122.3
  call _intel_fast_memset #122.3
..B1.63: # Preds ..B1.128 ..B1.62 ..B1.60 ..B1.126
  xorb %r10b, %r10b #149.8
  movl $expat_xml_data+1048576, %r11d #151.37
  xorl %r9d, %r9d #149.15
..B1.64: # Preds ..B1.111 ..B1.63
  movl $expat_xml_data, %edi #150.17
  movq $0x14650fb0739d0383, %r8 #83.22
  cmpq %r11, %rdi #85.16
  jae ..B1.111 # Prob 4% #85.16
..B1.66: # Preds ..B1.64 ..B1.108
  movzbl (%rdi), %ecx #86.24
  cmpl $34, %ecx #87.9
  je ..B1.68 # Prob 33% #87.9
..B1.67: # Preds ..B1.66
  cmpl $39, %ecx #87.9
  jne ..B1.75 # Prob 50% #87.9
..B1.68: # Preds ..B1.66 ..B1.67
  incq %rdi #89.7
  cmpq %r11, %rdi #90.20
  jae ..B1.111 # Prob 10% #90.20
..B1.69: # Preds ..B1.68
  cmpb (%rdi), %cl #90.35
  je ..B1.74 # Prob 20% #90.35
..B1.71: # Preds ..B1.69 ..B1.72
  incq %rdi #91.9
  cmpq %r11, %rdi #90.20
  jae ..B1.111 # Prob 18% #90.20
..B1.72: # Preds ..B1.71
  cmpb (%rdi), %cl #90.35
  jne ..B1.71 # Prob 80% #90.35
..B1.74: # Preds ..B1.79 ..B1.72 ..B1.69
  incq %rdi #93.9
  jmp ..B1.108 # Prob 100% #93.9
..B1.75: # Preds ..B1.67
  lea -97(%rcx), %eax #32.16
  cmpl $25, %eax #32.16
  jbe ..B1.81 # Prob 50% #32.16
..B1.76: # Preds ..B1.75
  lea -65(%rcx), %eax #33.16
  cmpl $25, %eax #33.16
  jbe ..B1.81 # Prob 50% #33.16
..B1.77: # Preds ..B1.76
  cmpl $58, %ecx #34.10
  je ..B1.81 # Prob 33% #34.10
..B1.78: # Preds ..B1.77
  cmpl $95, %ecx #34.10
  je ..B1.81 # Prob 50% #34.10
..B1.79: # Preds ..B1.78
  cmpl $194, %ecx #35.15
  jb ..B1.74 # Prob 50% #35.15
..B1.81: # Preds ..B1.75 ..B1.76 ..B1.77 ..B1.78 ..B1.79
  movq %rdi, %rdx #95.27
..B1.82: # Preds ..B1.103 ..B1.81
  movzbl (%rdx), %eax #53.24
  cmpl $128, %eax #54.13
  jb ..B1.96 # Prob 22% #54.13
..B1.83: # Preds ..B1.82
  lea -194(%rax), %esi #60.16
  cmpl $29, %esi #60.16
  ja ..B1.85 # Prob 50% #60.16
..B1.84: # Preds ..B1.83
  movl $2, %eax #61.9
  jmp ..B1.89 # Prob 100% #61.9
..B1.85: # Preds ..B1.83
  lea -224(%rax), %esi #62.21
  cmpl $15, %esi #62.21
  ja ..B1.87 # Prob 50% #62.21
..B1.86: # Preds ..B1.85
  movl $3, %eax #63.9
  jmp ..B1.89 # Prob 100% #63.9
..B1.87: # Preds ..B1.85
  addl $-240, %eax #64.21
  cmpl $4, %eax #64.21
  ja ..B1.106 # Prob 20% #64.21
..B1.88: # Preds ..B1.87
  movl $4, %eax #65.9
..B1.89: # Preds ..B1.84 ..B1.86 ..B1.88
  movq %r11, %rsi #95.55
  subq %rdx, %rsi #95.55
  cmpq %rax, %rsi #68.40
  jb ..B1.106 # Prob 20% #68.40
..B1.90: # Preds ..B1.89
  movb 1(%rdx), %sil #70.12
  andb $-64, %sil #70.21
  cmpb $-128, %sil #70.31
  jne ..B1.106 # Prob 20% #70.31
..B1.91: # Preds ..B1.90
  cmpq $2, %rax #71.23
  jbe ..B1.93 # Prob 50% #71.23
..B1.92: # Preds ..B1.91
  movb 2(%rdx), %sil #71.31
  andb $-64, %sil #71.40
  cmpb $-128, %sil #71.50
  jne ..B1.106 # Prob 20% #71.50
..B1.93: # Preds ..B1.92 ..B1.91
  cmpq $3, %rax #72.23
  jbe ..B1.95 # Prob 50% #72.23
..B1.94: # Preds ..B1.93
  movb 3(%rdx), %sil #72.31
  andb $-64, %sil #72.40
  cmpb $-128, %sil #72.50
  jne ..B1.106 # Prob 20% #72.50
..B1.95: # Preds ..B1.94 ..B1.93
  addq %rax, %rdx #74.7
  jmp ..B1.103 # Prob 100% #74.7
..B1.96: # Preds ..B1.82
  lea -97(%rax), %esi #32.16
  cmpl $25, %esi #32.16
  jbe ..B1.102 # Prob 50% #32.16
..B1.97: # Preds ..B1.96
  lea -65(%rax), %esi #33.16
  cmpl $25, %esi #33.16
  jbe ..B1.102 # Prob 50% #33.16
..B1.98: # Preds ..B1.97
  cmpl $58, %eax #34.10
  je ..B1.102 # Prob 33% #34.10
..B1.99: # Preds ..B1.98
  cmpl $95, %eax #34.10
  je ..B1.102 # Prob 50% #34.10
..B1.100: # Preds ..B1.99
  lea -48(%rax), %esi #42.16
  cmpl $9, %esi #42.16
  jbe ..B1.102 # Prob 50% #42.16
..B1.101: # Preds ..B1.100
  addl $-45, %eax #43.10
  cmpl $1, %eax #43.10
  ja ..B1.106 # Prob 50% #43.10
..B1.102: # Preds ..B1.101 ..B1.100 ..B1.99 ..B1.98 ..B1.97
  incq %rdx #57.7
..B1.103: # Preds ..B1.102 ..B1.95
  cmpq %r11, %rdx #52.16
  jb ..B1.82 # Prob 82% #52.16
..B1.106: # Preds ..B1.90 ..B1.89 ..B1.87 ..B1.92 ..B1.94
  subq %rdi, %rdx #77.26
  je ..B1.111 # Prob 20% #97.18
..B1.107: # Preds ..B1.106
  movq $0x100000001b3, %rax #100.7
  addq %rdx, %rcx #99.36
  addq %rdx, %rdi #101.7
  xorq %rcx, %r8 #99.7
  imulq %rax, %r8 #100.7
..B1.108: # Preds ..B1.74 ..B1.107
  cmpq %r11, %rdi #85.16
  jb ..B1.66 # Prob 82% #85.16
..B1.111: # Preds ..B1.106 ..B1.68 ..B1.71 ..B1.64 ..B1.108
  movl %r9d, %eax #153.28
  incb %r10b #149.33
  andq $1048575, %rax #153.37
  xorq %r8, %r14 #150.5
  addl $8191, %r9d #149.33
  xorb $1, expat_xml_data(%rax) #153.5
  cmpb $64, %r10b #149.25
  jb ..B1.64 # Prob 98% #149.25
..B1.112: # Preds ..B1.111
  movl $.L_2__STRING.0, %edi #156.3
  movq %r14, %rsi #156.3
  xorl %eax, %eax #156.3
  call printf #156.3
..B1.113: # Preds ..B1.112
  xorl %eax, %eax #157.10
  addq $120, %rsp #157.10
  popq %r14 #157.10
  movq %rbp, %rsp #157.10
  popq %rbp #157.10
  ret #157.10
..B1.122: # Preds ..B1.61
  cmpq $16, %rdx #122.3
  jb ..B1.130 # Prob 10% #122.3
..B1.123: # Preds ..B1.122
  movq %rdx, %r8 #122.3
  xorl %edi, %edi #122.3
  movdqu .L_2il0floatpacket.0(%rip), %xmm0 #123.29
  andq $-16, %r8 #122.3
  movq %rax, %rcx #123.5
..B1.124: # Preds ..B1.124 ..B1.123
  addq $16, %rdi #122.3
  movdqu %xmm0, expat_xml_data(%rcx) #123.5
  addq $16, %rcx #122.3
  cmpq %r8, %rdi #122.3
  jb ..B1.124 # Prob 99% #122.3
..B1.126: # Preds ..B1.124 ..B1.130
  addq %r8, %rax #122.3
  cmpq %rdx, %r8 #122.3
  jae ..B1.63 # Prob 10% #122.3
..B1.128: # Preds ..B1.126 ..B1.128
  incq %r8 #122.3
  movb $32, expat_xml_data(%rax) #123.5
  incq %rax #122.3
  cmpq %rdx, %r8 #122.3
  jb ..B1.128 # Prob 99% #122.3
  jmp ..B1.63 # Prob 100% #122.3
..B1.130: # Preds ..B1.122
  xorl %r8d, %r8d #122.3
  jmp ..B1.126 # Prob 100% #122.3
ascii_name.168.0.6:
utf8_name.168.0.6:
fragment.161.0.5:
expat_xml_data:
.L_2il0floatpacket.0:
.L_2__STRING.0:
