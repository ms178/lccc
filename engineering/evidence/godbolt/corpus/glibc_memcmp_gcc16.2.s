glibc_memcmp_common_alignment:
  leaq (%rdi,%rdx,8), %rcx
.L19:
  movq (%rdi), %rdx
  movq (%rsi), %rax
  cmpq %rax, %rdx
  jne .L29
  movq 8(%rdi), %rdx
  movq 8(%rsi), %rax
  cmpq %rax, %rdx
  jne .L30
  movq 16(%rdi), %rdx
  movq 16(%rsi), %rax
  cmpq %rax, %rdx
  jne .L31
  movq 24(%rdi), %rdx
  movq 24(%rsi), %rax
  cmpq %rax, %rdx
  jne .L32
  addq $32, %rdi
  addq $32, %rsi
  cmpq %rcx, %rdi
  jne .L19
  xorl %eax, %eax
  ret
.L29:
  movq %rax, -8(%rsp)
  leaq -16(%rsp), %rsi
  leaq -8(%rsp), %rcx
  movq %rdx, -16(%rsp)
  xorl %edx, %edx
.L5:
  movzbl (%rsi,%rdx), %eax
  movzbl (%rcx,%rdx), %edi
  cmpb %dil, %al
  jne .L27
  addq $1, %rdx
  cmpq $8, %rdx
  jne .L5
  xorl %eax, %eax
  ret
.L27:
  subl %edi, %eax
  ret
.L30:
  movq %rax, -8(%rsp)
  leaq -16(%rsp), %rsi
  leaq -8(%rsp), %rcx
  movq %rdx, -16(%rsp)
  xorl %edx, %edx
.L10:
  movzbl (%rsi,%rdx), %eax
  movzbl (%rcx,%rdx), %edi
  cmpb %dil, %al
  jne .L27
  addq $1, %rdx
  cmpq $8, %rdx
  jne .L10
  xorl %eax, %eax
  ret
.L31:
  movq %rax, -8(%rsp)
  leaq -16(%rsp), %rsi
  leaq -8(%rsp), %rcx
  movq %rdx, -16(%rsp)
  xorl %edx, %edx
.L14:
  movzbl (%rsi,%rdx), %eax
  movzbl (%rcx,%rdx), %edi
  cmpb %dil, %al
  jne .L27
  addq $1, %rdx
  cmpq $8, %rdx
  jne .L14
  xorl %eax, %eax
  ret
.L32:
  movq %rax, -8(%rsp)
  leaq -16(%rsp), %rsi
  leaq -8(%rsp), %rcx
  movq %rdx, -16(%rsp)
  xorl %edx, %edx
.L18:
  movzbl (%rsi,%rdx), %eax
  movzbl (%rcx,%rdx), %edi
  cmpb %dil, %al
  jne .L27
  addq $1, %rdx
  cmpq $8, %rdx
  jne .L18
  xorl %eax, %eax
  ret
.LC1:
  .string "%lu\n"
main:
  pushq %rbp
  movl $4, %edx
  movq %rsp, %rbp
  pushq %r12
  pushq %rbx
  andq $-32, %rsp
  subq $64, %rsp
  vmovdqa .LC0(%rip), %ymm0
  leaq 32(%rsp), %rsi
  movq %rsp, %rdi
  vmovdqa %ymm0, (%rsp)
  vmovdqa %ymm0, 32(%rsp)
  call glibc_memcmp_common_alignment
  testl %eax, %eax
  jne .L36
  movl %eax, %ebx
  movl $4, %edx
  leaq 32(%rsp), %rsi
  movq %rsp, %rdi
  movabsq $72623859790382857, %rax
  movq %rax, 40(%rsp)
  call glibc_memcmp_common_alignment
  testl %eax, %eax
  jns .L36
  movabsq $-7046029254386353131, %rdx
  xorl %ecx, %ecx
.L37:
  movq %rdx, %rax
  salq $7, %rax
  xorq %rdx, %rax
  movq %rax, %rdx
  shrq $9, %rdx
  xorq %rdx, %rax
  movq %rax, %rdx
  salq $8, %rdx
  xorq %rax, %rdx
  movq %rdx, glibc_left(,%rcx,8)
  movq %rdx, glibc_right(,%rcx,8)
  addq $1, %rcx
  cmpq $8192, %rcx
  jne .L37
  xorl %r8d, %r8d
  xorl %r10d, %r10d
  movl $1, %r11d
.L38:
  imulq $4051, %r8, %r9
  leal (%r8,%r8,4), %eax
  movl $8192, %edx
  movl $glibc_right, %esi
  leal (%r8,%rax,2), %eax
  movl $glibc_left, %edi
  addq $1, %r8
  shlx %rax, %r11, %rax
  andl $8191, %r9d
  movq glibc_left(,%r9,8), %r12
  xorq %r12, %rax
  movq %rax, glibc_right(,%r9,8)
  call glibc_memcmp_common_alignment
  movq %r12, glibc_right(,%r9,8)
  addl $257, %eax
  imulq %r8, %rax
  addq %rax, %r10
  cmpq $4096, %r8
  jne .L38
  movq %r10, %rsi
  movl $.LC1, %edi
  xorl %eax, %eax
  vzeroupper
  call printf
.L33:
  leaq -16(%rbp), %rsp
  movl %ebx, %eax
  popq %rbx
  popq %r12
  popq %rbp
  ret
.L36:
  movl $2, %ebx
  vzeroupper
  jmp .L33
.LC0:
  .quad 0
  .quad 72623859790382856
  .quad -1
  .quad 7
