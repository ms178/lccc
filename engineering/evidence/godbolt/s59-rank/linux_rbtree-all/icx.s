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
  movq %r9, %rdi
  andq $-4, %rdi
  je .LBB0_1
  movq %rsi, %r8
  jmp .LBB0_16
.LBB0_15:
  orq $1, %r10
  movq %r10, (%r9)
  orb $1, (%rdi)
  movq (%rsi), %r9
  andq $-2, %r9
  movq %r9, (%rsi)
  movq %r9, %rdi
  movq %rsi, %r8
  andq $-4, %rdi
  je .LBB0_1
.LBB0_16:
  movq (%rdi), %rsi
  testb $1, %sil
  jne .LBB0_2
  andq $-4, %rsi
  movq 8(%rsi), %r9
  cmpq %rdi, %r9
  je .LBB0_20
  testq %r9, %r9
  je .LBB0_31
  movq (%r9), %r10
  testb $1, %r10b
  je .LBB0_15
  jmp .LBB0_31
.LBB0_20:
  movq 16(%rsi), %r9
  testq %r9, %r9
  je .LBB0_22
  movq (%r9), %r10
  testb $1, %r10b
  je .LBB0_15
.LBB0_22:
  movq 16(%rdi), %r9
  cmpq %r9, %r8
  jne .LBB0_26
  movq 8(%r8), %r9
  movq %r9, 16(%rdi)
  movq %rdi, 8(%r8)
  testq %r9, %r9
  je .LBB0_25
  movq %rdi, %r10
  orq $1, %r10
  movq %r10, (%r9)
.LBB0_25:
  movq %r8, (%rdi)
  movq 16(%r8), %r9
  movq %r8, %rdi
.LBB0_26:
  movq %r9, 8(%rsi)
  movq %rsi, 16(%rdi)
  testq %r9, %r9
  je .LBB0_28
  movq %rsi, %r8
  orq $1, %r8
  movq %r8, (%r9)
.LBB0_28:
  movq (%rsi), %r8
  movq %r8, (%rdi)
  movq %rdi, (%rsi)
  andq $-4, %r8
  je .LBB0_40
  cmpq %rsi, 16(%r8)
  jne .LBB0_39
.LBB0_42:
  movq %rdi, 16(%r8)
  jmp .LBB0_2
.LBB0_31:
  movq 8(%rdi), %r9
  cmpq %r9, %r8
  jne .LBB0_35
  movq 16(%r8), %r9
  movq %r9, 8(%rdi)
  movq %rdi, 16(%r8)
  testq %r9, %r9
  je .LBB0_34
  movq %rdi, %r10
  orq $1, %r10
  movq %r10, (%r9)
.LBB0_34:
  movq %r8, (%rdi)
  movq 8(%r8), %r9
  movq %r8, %rdi
.LBB0_35:
  movq %r9, 16(%rsi)
  movq %rsi, 8(%rdi)
  testq %r9, %r9
  je .LBB0_37
  movq %rsi, %r8
  orq $1, %r8
  movq %r8, (%r9)
.LBB0_37:
  movq (%rsi), %r8
  movq %r8, (%rdi)
  movq %rdi, (%rsi)
  andq $-4, %r8
  je .LBB0_40
  cmpq %rsi, 16(%r8)
  je .LBB0_42
.LBB0_39:
  movq %rdi, 8(%r8)
  jmp .LBB0_2
.LBB0_40:
  movq %rdi, 8(%rsp)
  jmp .LBB0_2
.LBB0_43:
  xorl %eax, %eax
  movq 8(%rsp), %rcx
  xorl %esi, %esi
  jmp .LBB0_45
.LBB0_44:
  incl %eax
  cmpl $8, %eax
  je .LBB0_63
.LBB0_45:
  xorl %edx, %edx
  jmp .LBB0_48
.LBB0_46:
  movslq 28(%rdi), %rdi
  shlq $16, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
.LBB0_47:
  addl $2, %edx
  cmpl $16384, %edx
  je .LBB0_44
.LBB0_48:
  imull $6425, %edx, %edi
  testq %rcx, %rcx
  je .LBB0_56
  movl %edi, %r8d
  andl $16382, %r8d
  shlq $5, %r8
  movl node_pool+24(%r8), %r9d
  movq %rcx, %r8
  jmp .LBB0_52
.LBB0_50:
  addq $16, %r8
.LBB0_51:
  movq (%r8), %r8
  testq %r8, %r8
  je .LBB0_56
.LBB0_52:
  movslq 24(%r8), %r10
  cmpl %r9d, %r10d
  jg .LBB0_50
  cmpl %r9d, %r10d
  jge .LBB0_55
  addq $8, %r8
  jmp .LBB0_51
.LBB0_55:
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
.LBB0_56:
  testq %rcx, %rcx
  je .LBB0_47
  addl $6425, %edi
  andl $16383, %edi
  shlq $5, %rdi
  movl node_pool+24(%rdi), %r8d
  movq %rcx, %rdi
  jmp .LBB0_60
.LBB0_58:
  addq $16, %rdi
.LBB0_59:
  movq (%rdi), %rdi
  testq %rdi, %rdi
  je .LBB0_47
.LBB0_60:
  movslq 24(%rdi), %r9
  cmpl %r8d, %r9d
  jg .LBB0_58
  cmpl %r8d, %r9d
  jge .LBB0_46
  addq $8, %rdi
  jmp .LBB0_59
.LBB0_63:
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  retq

.L.str:

