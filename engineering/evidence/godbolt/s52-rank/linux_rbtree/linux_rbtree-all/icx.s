main:
  subq $24, %rsp
  vstmxcsr 20(%rsp)
  orl $32832, 20(%rsp)
  vldmxcsr 20(%rsp)
  movq $0, 8(%rsp)
  movl $1294694901, %eax
  xorl %ecx, %ecx
  vxorps %xmm0, %xmm0, %xmm0
  leaq 8(%rsp), %rdx
  jmp .LBB0_3
.LBB0_1:
  orq $1, %r9
  movq %r9, (%rsi)
.LBB0_2:
  incq %rcx
  cmpq $16384, %rcx
  je .LBB0_43
.LBB0_3:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  movl %eax, %edi
  andl $2147483647, %edi
  movq %rcx, %r8
  shlq $5, %r8
  leaq node_pool(%r8), %rsi
  movl %edi, node_pool+24(%r8)
  movl %ecx, node_pool+28(%r8)
  vmovups %xmm0, node_pool+8(%r8)
  movq 8(%rsp), %r10
  testq %r10, %r10
  je .LBB0_12
.LBB0_4:
  leaq 16(%r10), %r9
  leaq 8(%r10), %r8
  xorl %r11d, %r11d
  cmpl 24(%r10), %edi
  setl %r11b
  cmovlq %r9, %r8
  movq 8(%r10,%r11,8), %r11
  movq %r10, %r9
  testq %r11, %r11
  je .LBB0_13
  leaq 16(%r11), %r9
  leaq 8(%r11), %r8
  xorl %r10d, %r10d
  cmpl 24(%r11), %edi
  setl %r10b
  cmovlq %r9, %r8
  movq 8(%r11,%r10,8), %r10
  movq %r11, %r9
  testq %r10, %r10
  je .LBB0_13
  leaq 16(%r10), %r9
  leaq 8(%r10), %r8
  xorl %r11d, %r11d
  cmpl 24(%r10), %edi
  setl %r11b
  cmovlq %r9, %r8
  movq 8(%r10,%r11,8), %r11
  movq %r10, %r9
  testq %r11, %r11
  je .LBB0_13
  leaq 16(%r11), %r9
  leaq 8(%r11), %r8
  xorl %r10d, %r10d
  cmpl 24(%r11), %edi
  setl %r10b
  cmovlq %r9, %r8
  movq 8(%r11,%r10,8), %r10
  movq %r11, %r9
  testq %r10, %r10
  je .LBB0_13
  leaq 16(%r10), %r9
  leaq 8(%r10), %r8
  xorl %r11d, %r11d
  cmpl 24(%r10), %edi
  setl %r11b
  cmovlq %r9, %r8
  movq 8(%r10,%r11,8), %r11
  movq %r10, %r9
  testq %r11, %r11
  je .LBB0_13
  leaq 16(%r11), %r9
  leaq 8(%r11), %r8
  xorl %r10d, %r10d
  cmpl 24(%r11), %edi
  setl %r10b
  cmovlq %r9, %r8
  movq 8(%r11,%r10,8), %r10
  movq %r11, %r9
  testq %r10, %r10
  je .LBB0_13
  leaq 16(%r10), %r9
  leaq 8(%r10), %r8
  xorl %r11d, %r11d
  cmpl 24(%r10), %edi
  setl %r11b
  cmovlq %r9, %r8
  movq 8(%r10,%r11,8), %r11
  movq %r10, %r9
  testq %r11, %r11
  je .LBB0_13
  leaq 16(%r11), %r9
  leaq 8(%r11), %r8
  xorl %r10d, %r10d
  cmpl 24(%r11), %edi
  setl %r10b
  cmovlq %r9, %r8
  movq 8(%r11,%r10,8), %r10
  movq %r11, %r9
  testq %r10, %r10
  jne .LBB0_4
  jmp .LBB0_13
.LBB0_12:
  movq %rdx, %r8
  xorl %r9d, %r9d
.LBB0_13:
  movq %r9, (%rsi)
  movq %rsi, (%r8)
  movq %r9, %r8
  andq $-4, %r8
  je .LBB0_1
  movq %rsi, %rdi
  jmp .LBB0_16
.LBB0_15:
  orq $1, %r10
  movq %r10, (%r9)
  orb $1, (%r8)
  movq (%rsi), %r9
  andq $-2, %r9
  movq %r9, (%rsi)
  movq %r9, %r8
  movq %rsi, %rdi
  andq $-4, %r8
  je .LBB0_1
.LBB0_16:
  movq (%r8), %rsi
  testb $1, %sil
  jne .LBB0_2
  andq $-4, %rsi
  movq 8(%rsi), %r9
  cmpq %r8, %r9
  je .LBB0_20
  testq %r9, %r9
  je .LBB0_28
  movq (%r9), %r10
  testb $1, %r10b
  je .LBB0_15
  jmp .LBB0_28
.LBB0_20:
  movq 16(%rsi), %r9
  testq %r9, %r9
  je .LBB0_22
  movq (%r9), %r10
  testb $1, %r10b
  je .LBB0_15
.LBB0_22:
  movq 16(%r8), %r9
  cmpq %r9, %rdi
  je .LBB0_39
  movq %r8, %rdi
  movq %r9, 8(%rsi)
  movq %rsi, 16(%rdi)
  testq %r9, %r9
  je .LBB0_25
.LBB0_24:
  leaq 1(%rsi), %r8
  movq %r8, (%r9)
.LBB0_25:
  movq (%rsi), %r8
  movq %r8, (%rdi)
  movq %rdi, (%rsi)
  andq $-4, %r8
  je .LBB0_34
  cmpq %rsi, 16(%r8)
  jne .LBB0_33
.LBB0_38:
  movq %rdi, 16(%r8)
  jmp .LBB0_2
.LBB0_28:
  movq 8(%r8), %r9
  cmpq %r9, %rdi
  je .LBB0_35
  movq %r8, %rdi
  movq %r9, 16(%rsi)
  movq %rsi, 8(%rdi)
  testq %r9, %r9
  je .LBB0_31
.LBB0_30:
  leaq 1(%rsi), %r8
  movq %r8, (%r9)
.LBB0_31:
  movq (%rsi), %r8
  movq %r8, (%rdi)
  movq %rdi, (%rsi)
  andq $-4, %r8
  je .LBB0_34
  cmpq %rsi, 16(%r8)
  je .LBB0_38
.LBB0_33:
  movq %rdi, 8(%r8)
  jmp .LBB0_2
.LBB0_34:
  movq %rdi, 8(%rsp)
  jmp .LBB0_2
.LBB0_35:
  movq 16(%rdi), %r9
  movq %r9, 8(%r8)
  movq %r8, 16(%rdi)
  testq %r9, %r9
  je .LBB0_37
  movq %r8, %r10
  incq %r10
  movq %r10, (%r9)
.LBB0_37:
  movq %rdi, (%r8)
  movq 8(%rdi), %r9
  movq %r9, 16(%rsi)
  movq %rsi, 8(%rdi)
  testq %r9, %r9
  jne .LBB0_30
  jmp .LBB0_31
.LBB0_39:
  movq 8(%rdi), %r9
  movq %r9, 16(%r8)
  movq %r8, 8(%rdi)
  testq %r9, %r9
  je .LBB0_41
  movq %r8, %r10
  incq %r10
  movq %r10, (%r9)
.LBB0_41:
  movq %rdi, (%r8)
  movq 16(%rdi), %r9
  movq %r9, 8(%rsi)
  movq %rsi, 16(%rdi)
  testq %r9, %r9
  jne .LBB0_24
  jmp .LBB0_25
.LBB0_43:
  movq 8(%rsp), %rax
  xorl %ecx, %ecx
  xorl %esi, %esi
  testq %rax, %rax
  jne .LBB0_47
.LBB0_44:
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  retq
.LBB0_45:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_46:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_56
.LBB0_47:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_49
.LBB0_48:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_46
.LBB0_49:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_51
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_53
  jmp .LBB0_46
.LBB0_51:
  jge .LBB0_45
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_46
.LBB0_53:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_48
  jge .LBB0_45
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_49
  jmp .LBB0_46
.LBB0_56:
  xorl %ecx, %ecx
  jmp .LBB0_59
.LBB0_57:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_58:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_68
.LBB0_59:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_61
.LBB0_60:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_58
.LBB0_61:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_63
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_65
  jmp .LBB0_58
.LBB0_63:
  jge .LBB0_57
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_58
.LBB0_65:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_60
  jge .LBB0_57
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_61
  jmp .LBB0_58
.LBB0_68:
  xorl %ecx, %ecx
  jmp .LBB0_71
.LBB0_69:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_70:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_80
.LBB0_71:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_73
.LBB0_72:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_70
.LBB0_73:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_75
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_77
  jmp .LBB0_70
.LBB0_75:
  jge .LBB0_69
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_70
.LBB0_77:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_72
  jge .LBB0_69
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_73
  jmp .LBB0_70
.LBB0_80:
  xorl %ecx, %ecx
  jmp .LBB0_83
.LBB0_81:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_82:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_92
.LBB0_83:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_85
.LBB0_84:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_82
.LBB0_85:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_87
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_89
  jmp .LBB0_82
.LBB0_87:
  jge .LBB0_81
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_82
.LBB0_89:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_84
  jge .LBB0_81
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_85
  jmp .LBB0_82
.LBB0_92:
  xorl %ecx, %ecx
  jmp .LBB0_95
.LBB0_93:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_94:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_104
.LBB0_95:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_97
.LBB0_96:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_94
.LBB0_97:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_99
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_101
  jmp .LBB0_94
.LBB0_99:
  jge .LBB0_93
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_94
.LBB0_101:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_96
  jge .LBB0_93
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_97
  jmp .LBB0_94
.LBB0_104:
  xorl %ecx, %ecx
  jmp .LBB0_107
.LBB0_105:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_106:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_116
.LBB0_107:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_109
.LBB0_108:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_106
.LBB0_109:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_111
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_113
  jmp .LBB0_106
.LBB0_111:
  jge .LBB0_105
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_106
.LBB0_113:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_108
  jge .LBB0_105
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_109
  jmp .LBB0_106
.LBB0_116:
  xorl %ecx, %ecx
  jmp .LBB0_119
.LBB0_117:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_118:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_128
.LBB0_119:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_121
.LBB0_120:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_118
.LBB0_121:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_123
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_125
  jmp .LBB0_118
.LBB0_123:
  jge .LBB0_117
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_118
.LBB0_125:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_120
  jge .LBB0_117
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_121
  jmp .LBB0_118
.LBB0_128:
  xorl %ecx, %ecx
  jmp .LBB0_131
.LBB0_129:
  movslq 28(%rdx), %rdx
  movslq %r8d, %rdi
  shlq $16, %rdi
  xorq %rdx, %rdi
  addq %rdi, %rsi
.LBB0_130:
  incl %ecx
  cmpl $16384, %ecx
  je .LBB0_44
.LBB0_131:
  imull $6425, %ecx, %edx
  andl $16383, %edx
  shlq $5, %rdx
  movl node_pool+24(%rdx), %edi
  movq %rax, %rdx
  jmp .LBB0_133
.LBB0_132:
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_130
.LBB0_133:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jle .LBB0_135
  addq $16, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_137
  jmp .LBB0_130
.LBB0_135:
  jge .LBB0_129
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  je .LBB0_130
.LBB0_137:
  movl 24(%rdx), %r8d
  cmpl %edi, %r8d
  jg .LBB0_132
  jge .LBB0_129
  addq $8, %rdx
  movq (%rdx), %rdx
  testq %rdx, %rdx
  jne .LBB0_133
  jmp .LBB0_130

.L.str:

