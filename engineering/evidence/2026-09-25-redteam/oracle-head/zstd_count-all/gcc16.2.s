.LC0:
main:
  pushq %r12
  xorl %edx, %edx
  movl $-2058980567, %eax
  pushq %rbp
  pushq %rbx
  jmp .L4
.L30:
  cmpq $63, %rdx
  jbe .L2
  movl %eax, %ecx
  shrl $28, %ecx
  leal -64(%rcx,%rdx), %ecx
  movzbl buffer(%rcx), %ecx
.L3:
  movb %cl, buffer(%rdx)
  addq $1, %rdx
  cmpq $1048576, %rdx
  je .L29
.L4:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  testb $7, %al
  je .L30
.L2:
  movl %eax, %edi
  shrl $24, %edi
  movl %edi, %ecx
  jmp .L3
.L29:
  movl $16, %r9d
  movl $324508639, %esi
  xorl %r8d, %r8d
.L5:
  xorl %edi, %edi
  jmp .L13
.L7:
  xorq %rbx, %rdx
  tzcntq %rdx, %rdx
  shrq $3, %rdx
  addq %rdx, %rax
  subl %r11d, %eax
.L9:
  movl %edi, %edx
  addl $1, %edi
  andl $7, %edx
  sall $3, %edx
  shlx %rdx, %rax, %rax
  xorq %r10, %rax
  addq %rax, %r8
  cmpl $131072, %edi
  je .L31
.L13:
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  movl %esi, %eax
  movl %esi, %r10d
  movl %esi, %ecx
  andl $127, %eax
  andl $1048319, %r10d
  shrl $12, %ecx
  addl $16, %eax
  andl $1048319, %ecx
  leaq buffer(%r10), %r11
  leaq buffer(%r10,%rax), %rbp
  addq $buffer, %rcx
  movq %r11, %rax
  leaq -7(%rbp), %r12
  cmpq %r12, %r11
  jnb .L6
.L8:
  movq (%rcx), %rdx
  movq (%rax), %rbx
  cmpq %rbx, %rdx
  jne .L7
  addq $8, %rax
  addq $8, %rcx
  cmpq %r12, %rax
  jb .L8
.L6:
  cmpq %rbp, %rax
  jb .L10
  jmp .L11
.L12:
  addq $1, %rax
  addq $1, %rcx
  cmpq %rax, %rbp
  je .L11
.L10:
  movzbl (%rcx), %ebx
  cmpb %bl, (%rax)
  je .L12
.L11:
  subl %r11d, %eax
  jmp .L9
.L31:
  subl $1, %r9d
  jne .L5
  movq %r8, %rsi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  popq %rbx
  xorl %eax, %eax
  popq %rbp
  popq %r12
  ret
