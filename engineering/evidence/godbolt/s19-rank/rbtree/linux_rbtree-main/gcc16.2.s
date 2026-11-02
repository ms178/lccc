main:
  subq $8, %rsp
  movl $node_pool, %r9d
  vpxor %xmm0, %xmm0, %xmm0
  xorl %r11d, %r11d
  movl $1663615696, node_pool+24(%rip)
  movq %r9, %r8
  movq %r9, %rdi
  movl $1663615696, %r10d
  movl $0, node_pool+28(%rip)
  movq $0, node_pool(%rip)
  vmovdqu %xmm0, node_pool+8(%rip)
.L2:
  orq $1, (%rdi)
.L9:
  addl $1, %r11d
  cmpl $16384, %r11d
  je .L39
  imull $1664525, %r10d, %r10d
  addq $32, %r9
  movq %r8, %rax
  movl %r11d, 28(%r9)
  movq %r9, %rdi
  vmovdqu %xmm0, 8(%r9)
  addl $1013904223, %r10d
  movl %r10d, %ecx
  andl $2147483647, %ecx
  movl %ecx, 24(%r9)
  jmp .L5
.L72:
  movq 16(%rax), %rdx
  leaq 16(%rax), %rsi
  testq %rdx, %rdx
  je .L71
.L35:
  movq %rdx, %rax
.L5:
  cmpl %ecx, 24(%rax)
  jg .L72
  movq 8(%rax), %rdx
  leaq 8(%rax), %rsi
  testq %rdx, %rdx
  jne .L35
.L71:
  movq %rax, (%r9)
  movq %r9, (%rsi)
  cmpq $3, %rax
  jbe .L2
  andq $-4, %rax
.L8:
  movq (%rax), %rdx
  testb $1, %dl
  jne .L9
  andq $-4, %rdx
  movq 8(%rdx), %rcx
  cmpq %rax, %rcx
  je .L12
  testq %rcx, %rcx
  je .L13
  movq (%rcx), %rsi
  testb $1, %sil
  je .L63
.L13:
  movq 8(%rax), %rcx
  cmpq %rdi, %rcx
  je .L15
  movq %rax, %rdi
.L16:
  movq %rcx, 16(%rdx)
  movq %rdx, 8(%rax)
  testq %rcx, %rcx
  je .L25
.L68:
  movq %rdx, %rsi
  orq $1, %rsi
  movq %rsi, (%rcx)
.L25:
  movq (%rdx), %rcx
  movq %rcx, (%rax)
  movq %rdi, (%rdx)
  cmpq $3, %rcx
  jbe .L38
.L74:
  andq $-4, %rcx
  cmpq 16(%rcx), %rdx
  je .L73
  movq %rax, 8(%rcx)
  jmp .L9
.L63:
  orq $1, %rsi
  movq %rdx, %rdi
  movq %rsi, (%rcx)
  orq $1, (%rax)
  movq (%rdx), %rax
  movq %rax, %rcx
  andq $-4, %rax
  andq $-2, %rcx
  movq %rcx, (%rdx)
  testq %rax, %rax
  jne .L8
  jmp .L2
.L12:
  movq 16(%rdx), %rcx
  testq %rcx, %rcx
  je .L21
  movq (%rcx), %rsi
  testb $1, %sil
  je .L63
.L21:
  movq 16(%rax), %rcx
  cmpq %rdi, %rcx
  je .L22
  movq %rax, %rdi
.L23:
  movq %rcx, 8(%rdx)
  movq %rdx, 16(%rax)
  testq %rcx, %rcx
  jne .L68
  movq (%rdx), %rcx
  movq %rcx, (%rax)
  movq %rdi, (%rdx)
  cmpq $3, %rcx
  ja .L74
.L38:
  movq %rax, %r8
  jmp .L9
.L15:
  movq 16(%rdi), %rcx
  movq %rcx, 8(%rax)
  movq %rax, 16(%rdi)
  testq %rcx, %rcx
  je .L17
  movq %rax, %rsi
  orq $1, %rsi
  movq %rsi, (%rcx)
.L17:
  movq %rdi, (%rax)
  movq 8(%rdi), %rcx
  movq %rdi, %rax
  jmp .L16
.L73:
  movq %rax, 16(%rcx)
  jmp .L9
.L22:
  movq 8(%rdi), %rcx
  movq %rcx, 16(%rax)
  movq %rax, 8(%rdi)
  testq %rcx, %rcx
  je .L24
  movq %rax, %rsi
  orq $1, %rsi
  movq %rsi, (%rcx)
.L24:
  movq %rdi, (%rax)
  movq 16(%rdi), %rcx
  movq %rdi, %rax
  jmp .L23
.L39:
  movl $8, %r9d
  xorl %edi, %edi
.L27:
  xorl %esi, %esi
.L32:
  movl %esi, %eax
  andl $16383, %eax
  salq $5, %rax
  movl node_pool+24(%rax), %ecx
  movq %r8, %rax
  jmp .L31
.L75:
  movq 16(%rax), %rax
  testq %rax, %rax
  je .L34
.L31:
  movslq 24(%rax), %rdx
  cmpl %edx, %ecx
  jl .L75
  jle .L30
  movq 8(%rax), %rax
  testq %rax, %rax
  jne .L31
.L34:
  addl $104729, %esi
  cmpl $1715879936, %esi
  jne .L32
.L76:
  subl $1, %r9d
  jne .L27
  movq %rdi, %rsi
  xorl %eax, %eax
  movl $.LC0, %edi
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
.L30:
  movslq 28(%rax), %rax
  salq $16, %rdx
  addl $104729, %esi
  xorq %rax, %rdx
  addq %rdx, %rdi
  cmpl $1715879936, %esi
  jne .L32
  jmp .L76
