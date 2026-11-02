main:
  pushq %rax
  movl $1294694901, %edx
  xorl %ecx, %ecx
  leaq node_pool(%rip), %rax
  vxorps %xmm0, %xmm0, %xmm0
  xorl %esi, %esi
  jmp .LBB0_4
.LBB0_1:
  movq $0, (%rdi)
  movq %rdi, %rcx
  xorl %r10d, %r10d
.LBB0_2:
  orq $1, %r10
  movq %r10, (%rdi)
.LBB0_3:
  incq %rsi
  cmpq $16384, %rsi
  je .LBB0_36
.LBB0_4:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  movl %edx, %r8d
  andl $2147483647, %r8d
  movq %rsi, %r9
  shlq $5, %r9
  leaq (%rax,%r9), %rdi
  movl %r8d, 24(%r9,%rax)
  movl %esi, 28(%r9,%rax)
  vmovups %xmm0, 8(%r9,%rax)
  movq %rcx, %r11
  testq %rcx, %rcx
  je .LBB0_1
.LBB0_5:
  movq %r11, %r10
  xorl %r9d, %r9d
  cmpl 24(%r11), %r8d
  setl %r9b
  movq 8(%r11,%r9,8), %r11
  testq %r11, %r11
  jne .LBB0_5
  leaq 8(,%r9,8), %r8
  movq %r10, (%rdi)
  movq %rdi, (%r10,%r8)
  movq %r10, %r8
  andq $-4, %r8
  je .LBB0_2
  movq %rdi, %r9
  jmp .LBB0_9
.LBB0_8:
  incq %r11
  movq %r11, (%r10)
  orb $1, (%r8)
  movq (%rdi), %r8
  movq %r8, %r10
  andq $-2, %r10
  movq %r10, (%rdi)
  movq %rdi, %r9
  andq $-4, %r8
  je .LBB0_2
.LBB0_9:
  movq (%r8), %rdi
  testb $1, %dil
  jne .LBB0_3
  andq $-4, %rdi
  movq 8(%rdi), %r10
  cmpq %r8, %r10
  je .LBB0_13
  testq %r10, %r10
  je .LBB0_21
  movq (%r10), %r11
  testb $1, %r11b
  je .LBB0_8
  jmp .LBB0_21
.LBB0_13:
  movq 16(%rdi), %r10
  testq %r10, %r10
  je .LBB0_15
  movq (%r10), %r11
  testb $1, %r11b
  je .LBB0_8
.LBB0_15:
  movq 16(%r8), %r10
  cmpq %r10, %r9
  je .LBB0_32
  movq %r10, 8(%rdi)
  movq %rdi, 16(%r8)
  testq %r10, %r10
  je .LBB0_18
.LBB0_17:
  leaq 1(%rdi), %r9
  movq %r9, (%r10)
.LBB0_18:
  movq (%rdi), %r9
  movq %r9, (%r8)
  movq %r8, (%rdi)
  andq $-4, %r9
  je .LBB0_30
  cmpq %rdi, 16(%r9)
  jne .LBB0_26
.LBB0_31:
  movq %r8, 16(%r9)
  jmp .LBB0_3
.LBB0_21:
  movq 8(%r8), %r10
  cmpq %r10, %r9
  je .LBB0_27
  movq %r10, 16(%rdi)
  movq %rdi, 8(%r8)
  testq %r10, %r10
  je .LBB0_24
.LBB0_23:
  leaq 1(%rdi), %r9
  movq %r9, (%r10)
.LBB0_24:
  movq (%rdi), %r9
  movq %r9, (%r8)
  movq %r8, (%rdi)
  andq $-4, %r9
  je .LBB0_30
  cmpq %rdi, 16(%r9)
  je .LBB0_31
.LBB0_26:
  movq %r8, 8(%r9)
  jmp .LBB0_3
.LBB0_30:
  movq %r8, %rcx
  jmp .LBB0_3
.LBB0_27:
  movq 16(%r9), %r10
  movq %r10, 8(%r8)
  movq %r8, 16(%r9)
  testq %r10, %r10
  je .LBB0_29
  leaq 1(%r8), %r11
  movq %r11, (%r10)
.LBB0_29:
  movq %r9, (%r8)
  movq 8(%r9), %r10
  movq %r9, %r8
  movq %r10, 16(%rdi)
  movq %rdi, 8(%r8)
  testq %r10, %r10
  jne .LBB0_23
  jmp .LBB0_24
.LBB0_32:
  movq 8(%r9), %r10
  movq %r10, 16(%r8)
  movq %r8, 8(%r9)
  testq %r10, %r10
  je .LBB0_34
  leaq 1(%r8), %r11
  movq %r11, (%r10)
.LBB0_34:
  movq %r9, (%r8)
  movq 16(%r9), %r10
  movq %r9, %r8
  movq %r10, 8(%rdi)
  movq %rdi, 16(%r8)
  testq %r10, %r10
  jne .LBB0_17
  jmp .LBB0_18
.LBB0_36:
  xorl %esi, %esi
  xorl %edx, %edx
  jmp .LBB0_38
.LBB0_37:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_48
.LBB0_38:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_40
.LBB0_39:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_43
.LBB0_40:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_39
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_39
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_43:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_45
.LBB0_44:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_37
.LBB0_45:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_44
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_44
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_37
.LBB0_48:
  xorl %edx, %edx
  jmp .LBB0_50
.LBB0_49:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_60
.LBB0_50:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_52
.LBB0_51:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_55
.LBB0_52:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_51
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_51
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_55:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_57
.LBB0_56:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_49
.LBB0_57:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_56
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_56
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_49
.LBB0_60:
  xorl %edx, %edx
  jmp .LBB0_62
.LBB0_61:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_72
.LBB0_62:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_64
.LBB0_63:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_67
.LBB0_64:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_63
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_63
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_67:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_69
.LBB0_68:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_61
.LBB0_69:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_68
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_68
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_61
.LBB0_72:
  xorl %edx, %edx
  jmp .LBB0_74
.LBB0_73:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_84
.LBB0_74:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_76
.LBB0_75:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_79
.LBB0_76:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_75
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_75
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_79:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_81
.LBB0_80:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_73
.LBB0_81:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_80
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_80
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_73
.LBB0_84:
  xorl %edx, %edx
  jmp .LBB0_86
.LBB0_85:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_96
.LBB0_86:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_88
.LBB0_87:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_91
.LBB0_88:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_87
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_87
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_91:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_93
.LBB0_92:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_85
.LBB0_93:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_92
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_92
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_85
.LBB0_96:
  xorl %edx, %edx
  jmp .LBB0_98
.LBB0_97:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_108
.LBB0_98:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_100
.LBB0_99:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_103
.LBB0_100:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_99
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_99
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_103:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_105
.LBB0_104:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_97
.LBB0_105:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_104
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_104
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_97
.LBB0_108:
  xorl %edx, %edx
  jmp .LBB0_110
.LBB0_109:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_120
.LBB0_110:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_112
.LBB0_111:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_115
.LBB0_112:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_111
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_111
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_115:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_117
.LBB0_116:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_109
.LBB0_117:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_116
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_116
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_109
.LBB0_120:
  xorl %edx, %edx
  jmp .LBB0_122
.LBB0_121:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_132
.LBB0_122:
  imull $6425, %edx, %edi
  movl %edi, %r8d
  andl $16382, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_124
.LBB0_123:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_127
.LBB0_124:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_123
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_123
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_127:
  addl $6425, %edi
  andl $16383, %edi
  shll $5, %edi
  movl 24(%rdi,%rax), %r8d
  movq %rcx, %rdi
  jmp .LBB0_129
.LBB0_128:
  movq (%rdi,%r10), %rdi
  testq %rdi, %rdi
  je .LBB0_121
.LBB0_129:
  movslq 24(%rdi), %r9
  movl $16, %r10d
  cmpl %r9d, %r8d
  jl .LBB0_128
  movl $8, %r10d
  cmpl %r9d, %r8d
  jg .LBB0_128
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  jmp .LBB0_121
.LBB0_132:
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

