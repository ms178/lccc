main:
  pushq %rbx
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
  je .LBB0_24
  movq (%r10), %r11
  testb $1, %r11b
  je .LBB0_8
  jmp .LBB0_24
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
  jne .LBB0_19
  movq 8(%r9), %r10
  movq %r10, 16(%r8)
  movq %r8, 8(%r9)
  testq %r10, %r10
  je .LBB0_18
  leaq 1(%r8), %r11
  movq %r11, (%r10)
.LBB0_18:
  movq %r9, (%r8)
  movq 16(%r9), %r10
  movq %r9, %r8
.LBB0_19:
  movq %r10, 8(%rdi)
  movq %rdi, 16(%r8)
  testq %r10, %r10
  je .LBB0_21
  leaq 1(%rdi), %r9
  movq %r9, (%r10)
.LBB0_21:
  movq (%rdi), %r9
  movq %r9, (%r8)
  movq %r8, (%rdi)
  andq $-4, %r9
  je .LBB0_33
  cmpq %rdi, 16(%r9)
  jne .LBB0_32
.LBB0_34:
  movq %r8, 16(%r9)
  jmp .LBB0_3
.LBB0_24:
  movq 8(%r8), %r10
  cmpq %r10, %r9
  jne .LBB0_28
  movq 16(%r9), %r10
  movq %r10, 8(%r8)
  movq %r8, 16(%r9)
  testq %r10, %r10
  je .LBB0_27
  leaq 1(%r8), %r11
  movq %r11, (%r10)
.LBB0_27:
  movq %r9, (%r8)
  movq 8(%r9), %r10
  movq %r9, %r8
.LBB0_28:
  movq %r10, 16(%rdi)
  movq %rdi, 8(%r8)
  testq %r10, %r10
  je .LBB0_30
  leaq 1(%rdi), %r9
  movq %r9, (%r10)
.LBB0_30:
  movq (%rdi), %r9
  movq %r9, (%r8)
  movq %r8, (%rdi)
  andq $-4, %r9
  je .LBB0_33
  cmpq %rdi, 16(%r9)
  je .LBB0_34
.LBB0_32:
  movq %r8, 8(%r9)
  jmp .LBB0_3
.LBB0_33:
  movq %r8, %rcx
  jmp .LBB0_3
.LBB0_36:
  xorl %esi, %esi
  xorl %edx, %edx
  jmp .LBB0_38
.LBB0_37:
  incl %edx
  cmpl $8, %edx
  je .LBB0_50
.LBB0_38:
  xorl %edi, %edi
  jmp .LBB0_40
.LBB0_39:
  addl $2, %edi
  cmpl $16384, %edi
  je .LBB0_37
.LBB0_40:
  imull $6425, %edi, %r8d
  movl %r8d, %r9d
  andl $16382, %r9d
  shll $5, %r9d
  movl 24(%r9,%rax), %r10d
  movq %rcx, %r9
  jmp .LBB0_42
.LBB0_41:
  movq (%r9,%rbx), %r9
  testq %r9, %r9
  je .LBB0_45
.LBB0_42:
  movslq 24(%r9), %r11
  movl $16, %ebx
  cmpl %r11d, %r10d
  jl .LBB0_41
  movl $8, %ebx
  cmpl %r11d, %r10d
  jg .LBB0_41
  movslq 28(%r9), %r9
  shlq $16, %r11
  xorq %r9, %r11
  addq %r11, %rsi
.LBB0_45:
  addl $6425, %r8d
  andl $16383, %r8d
  shll $5, %r8d
  movl 24(%r8,%rax), %r9d
  movq %rcx, %r8
  jmp .LBB0_47
.LBB0_46:
  movq (%r8,%r11), %r8
  testq %r8, %r8
  je .LBB0_39
.LBB0_47:
  movslq 24(%r8), %r10
  movl $16, %r11d
  cmpl %r10d, %r9d
  jl .LBB0_46
  movl $8, %r11d
  cmpl %r10d, %r9d
  jg .LBB0_46
  movslq 28(%r8), %r8
  shlq $16, %r10
  xorq %r8, %r10
  addq %r10, %rsi
  jmp .LBB0_39
.LBB0_50:
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rbx
  retq

.L.str:

