.LC0:
main:
  subq $8, %rsp
  movl $node_pool, %esi
  vpxor %xmm0, %xmm0, %xmm0
  xorl %r10d, %r10d
  vmovdqu %xmm0, node_pool+8(%rip)
  xorl %ecx, %ecx
  movq %rsi, %rdi
  movq %rsi, %r8
  movl $1663615696, node_pool+24(%rip)
  movl $1663615696, %r9d
  movl $0, node_pool+28(%rip)
  movq $0, node_pool(%rip)
.L2:
  orq $1, %rcx
  movq %rcx, (%r8)
.L11:
  addl $1, %r10d
  cmpl $16384, %r10d
  je .L42
  imull $1664525, %r9d, %r9d
  addq $32, %rsi
  movq %rdi, %rax
  movl %r10d, 28(%rsi)
  movq %rsi, %r8
  vmovdqu %xmm0, 8(%rsi)
  addl $1013904223, %r9d
  movl %r9d, %ecx
  andl $2147483647, %ecx
  movl %ecx, 24(%rsi)
  jmp .L7
.L76:
  movq 16(%rax), %rdx
  testq %rdx, %rdx
  je .L75
.L38:
  movq %rdx, %rax
.L7:
  cmpl %ecx, 24(%rax)
  jg .L76
  movq 8(%rax), %rdx
  testq %rdx, %rdx
  jne .L38
  leaq 8(%rax), %rdx
.L6:
  movq %rax, (%rsi)
  movq %rax, %rcx
  movq %rsi, (%rdx)
  cmpq $3, %rax
  jbe .L2
  andq $-4, %rax
.L10:
  movq (%rax), %rdx
  testb $1, %dl
  jne .L11
  andq $-4, %rdx
  movq 8(%rdx), %rcx
  cmpq %rax, %rcx
  je .L14
  testq %rcx, %rcx
  je .L15
  movq (%rcx), %r11
  testb $1, %r11b
  je .L67
.L15:
  movq 8(%rax), %rcx
  cmpq %r8, %rcx
  je .L17
  movq %rax, %r8
.L18:
  movq %rcx, 16(%rdx)
  movq %rdx, 8(%rax)
  testq %rcx, %rcx
  je .L27
.L72:
  movq %rdx, %r11
  orq $1, %r11
  movq %r11, (%rcx)
.L27:
  movq (%rdx), %rcx
  movq %rcx, (%rax)
  movq %r8, (%rdx)
  cmpq $3, %rcx
  jbe .L41
.L78:
  andq $-4, %rcx
  cmpq 16(%rcx), %rdx
  je .L77
  movq %rax, 8(%rcx)
  jmp .L11
.L67:
  orq $1, %r11
  movq %rdx, %r8
  movq %r11, (%rcx)
  orq $1, (%rax)
  movq (%rdx), %rax
  movq %rax, %rcx
  andq $-4, %rax
  andq $-2, %rcx
  movq %rcx, (%rdx)
  testq %rax, %rax
  jne .L10
  jmp .L2
.L75:
  leaq 16(%rax), %rdx
  jmp .L6
.L14:
  movq 16(%rdx), %rcx
  testq %rcx, %rcx
  je .L23
  movq (%rcx), %r11
  testb $1, %r11b
  je .L67
.L23:
  movq 16(%rax), %rcx
  cmpq %r8, %rcx
  je .L24
  movq %rax, %r8
.L25:
  movq %rcx, 8(%rdx)
  movq %rdx, 16(%rax)
  testq %rcx, %rcx
  jne .L72
  movq (%rdx), %rcx
  movq %rcx, (%rax)
  movq %r8, (%rdx)
  cmpq $3, %rcx
  ja .L78
.L41:
  movq %rax, %rdi
  jmp .L11
.L17:
  movq 16(%r8), %rcx
  movq %rcx, 8(%rax)
  movq %rax, 16(%r8)
  testq %rcx, %rcx
  je .L19
  movq %rax, %r11
  orq $1, %r11
  movq %r11, (%rcx)
.L19:
  movq %r8, (%rax)
  movq 8(%r8), %rcx
  movq %r8, %rax
  jmp .L18
.L77:
  movq %rax, 16(%rcx)
  jmp .L11
.L24:
  movq 8(%r8), %rcx
  movq %rcx, 16(%rax)
  movq %rax, 8(%r8)
  testq %rcx, %rcx
  je .L26
  movq %rax, %r11
  orq $1, %r11
  movq %r11, (%rcx)
.L26:
  movq %r8, (%rax)
  movq 16(%r8), %rcx
  movq %r8, %rax
  jmp .L25
.L42:
  movl $8, %r9d
  xorl %r8d, %r8d
.L29:
  xorl %esi, %esi
.L34:
  movl %esi, %eax
  andl $16383, %eax
  salq $5, %rax
  movl node_pool+24(%rax), %ecx
  movq %rdi, %rax
  jmp .L33
.L79:
  movq 16(%rax), %rax
  testq %rax, %rax
  je .L36
.L33:
  movslq 24(%rax), %rdx
  cmpl %edx, %ecx
  jl .L79
  jle .L32
  movq 8(%rax), %rax
  testq %rax, %rax
  jne .L33
.L36:
  addl $104729, %esi
  cmpl $1715879936, %esi
  jne .L34
.L80:
  subl $1, %r9d
  jne .L29
  movq %r8, %rsi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
.L32:
  movslq 28(%rax), %rax
  salq $16, %rdx
  addl $104729, %esi
  xorq %rax, %rdx
  addq %rdx, %r8
  cmpl $1715879936, %esi
  jne .L34
  jmp .L80
